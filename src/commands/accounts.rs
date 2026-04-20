//! `mm accounts` — list and inspect MoneyMoney accounts.

use std::collections::HashMap;
use std::str::FromStr;

use iban::{BaseIban, IbanLike};
use serde::Serialize;

use crate::applescript::{OsascriptRunner, run_plist};
use crate::moneymoney::errors;
use crate::moneymoney::resolver::Resolver;
use crate::moneymoney::types::Account;
use crate::output::{
    DetailView, FieldFilter, FieldNames, OutputFormat, Tabular, format_detail, format_list,
};

const SCRIPT_EXPORT_ACCOUNTS: &str = "tell application \"MoneyMoney\" to export accounts";

/// Fetch all accounts (including group parents) from MoneyMoney.
pub async fn fetch_all<R: OsascriptRunner>(runner: &R) -> errors::Result<Vec<Account>> {
    run_plist(runner, SCRIPT_EXPORT_ACCOUNTS).await
}

/// A raw [`Account`] plus the display `bank` name inferred from its parent
/// group. Use for listing and tabular output.
#[derive(Debug, Clone, Serialize)]
pub struct AccountRow {
    /// Bank the account belongs to: the parent group's name, or the account's
    /// own name when it's top-level and standalone (e.g., PayPal).
    pub bank: String,
    /// Normalized IBAN when [`Account::account_number`] parses as one.
    /// `None` for PayPal, legacy number accounts, or anything else that
    /// doesn't pass mod-97.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iban: Option<String>,
    /// The underlying account row.
    #[serde(flatten)]
    pub account: Account,
}

/// Walk the flat account list and annotate each row with its parent bank
/// name, derived from the preceding `indentation == 0` entry. Also derives
/// the normalized IBAN when `account_number` parses as one.
pub fn annotate_with_bank(accounts: Vec<Account>) -> Vec<AccountRow> {
    let mut current_bank: Option<String> = None;
    accounts
        .into_iter()
        .map(|account| {
            let bank = if account.indentation == 0 {
                current_bank = Some(account.name.clone());
                account.name.clone()
            } else {
                current_bank.clone().unwrap_or_default()
            };
            let iban = BaseIban::from_str(&account.account_number)
                .ok()
                .map(|b| b.electronic_str().to_owned());
            AccountRow {
                bank,
                iban,
                account,
            }
        })
        .collect()
}

impl Tabular for AccountRow {
    fn headers() -> &'static [&'static str] {
        &["Bank", "Name", "Account", "IBAN", "Balance", "Currency"]
    }

    fn row(&self) -> Vec<String> {
        let balance = self
            .account
            .balance
            .first()
            .map(|b| b.amount.to_string())
            .unwrap_or_default();
        vec![
            self.bank.clone(),
            self.account.name.clone(),
            self.account.account_number.clone(),
            self.iban.clone().unwrap_or_default(),
            balance,
            self.account.currency.clone(),
        ]
    }
}

impl DetailView for AccountRow {
    fn fields(&self) -> Vec<(&'static str, String)> {
        let Account {
            uuid,
            name,
            owner,
            account_number,
            bank_code,
            currency,
            group,
            portfolio,
            indentation,
            account_type,
            balance,
        } = &self.account;
        let balance_str = balance
            .iter()
            .map(|b| format!("{} {}", b.amount, b.currency))
            .collect::<Vec<_>>()
            .join(", ");
        vec![
            ("Bank", self.bank.clone()),
            ("Name", name.clone()),
            ("UUID", uuid.clone()),
            ("Owner", owner.clone()),
            ("Account", account_number.clone()),
            ("IBAN", self.iban.clone().unwrap_or_default()),
            ("BIC", bank_code.clone()),
            ("Type", account_type.clone()),
            ("Currency", currency.clone()),
            ("Balance", balance_str),
            ("Group", group.to_string()),
            ("Portfolio", portfolio.to_string()),
            ("Indentation", indentation.to_string()),
        ]
    }
}

impl FieldNames for AccountRow {
    fn valid_fields() -> &'static [&'static str] {
        &[
            "bank",
            "name",
            "uuid",
            "owner",
            "account",
            "iban",
            "bic",
            "type",
            "currency",
            "balance",
            "group",
            "portfolio",
            "indentation",
        ]
    }
}

/// Options controlling `mm accounts list` output and filtering.
pub struct ListOptions {
    pub format: Option<OutputFormat>,
    pub fields: Option<String>,
    pub tree: bool,
    pub include_groups: bool,
}

/// `mm accounts list` — render the account list to stdout.
pub async fn run_list<R: OsascriptRunner>(runner: &R, opts: ListOptions) -> anyhow::Result<()> {
    let raw = fetch_all(runner).await?;
    let mut rows = annotate_with_bank(raw);

    // `--tree` implies showing the group rows.
    let include_groups = opts.include_groups || opts.tree;
    if !include_groups {
        rows.retain(|row| !row.account.group);
    }

    // Pretty indentation in tree mode (table only; structured formats keep
    // the raw name so consumers don't need to strip prefixes).
    let format = OutputFormat::resolve(opts.format);
    if opts.tree && matches!(format, OutputFormat::Table) {
        for row in &mut rows {
            let depth = usize::try_from(row.account.indentation).unwrap_or(0);
            if depth > 0 {
                row.account.name = format!(
                    "{}└ {}",
                    "  ".repeat(depth.saturating_sub(1)),
                    row.account.name
                );
            }
        }
    }

    let filter = opts
        .fields
        .as_deref()
        .map(FieldFilter::parse::<AccountRow>)
        .transpose()?;

    let total = rows.len();
    format_list(&rows, total, format, filter.as_ref())
}

/// Options controlling `mm accounts get <REF>`.
pub struct GetOptions {
    pub reference: String,
    pub format: Option<OutputFormat>,
    pub fields: Option<String>,
    pub aliases: HashMap<String, String>,
}

/// `mm accounts get <REF>` — resolve a reference and render full details.
pub async fn run_get<R: OsascriptRunner>(runner: &R, opts: GetOptions) -> anyhow::Result<()> {
    let raw = fetch_all(runner).await?;
    let rows = annotate_with_bank(raw);
    let resolver = Resolver::new(rows, opts.aliases);
    let row = resolver.resolve(&opts.reference)?;

    let format = OutputFormat::resolve(opts.format);
    let filter = opts
        .fields
        .as_deref()
        .map(FieldFilter::parse::<AccountRow>)
        .transpose()?;

    format_detail(row, format, filter.as_ref())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests assert on well-known fixture input"
)]
mod tests {
    use rust_decimal::Decimal;

    use super::*;
    use crate::moneymoney::types::BalanceEntry;

    fn account(name: &str, account_number: &str, indentation: u32) -> Account {
        Account {
            uuid: format!("uuid-{name}"),
            name: name.to_owned(),
            owner: String::new(),
            account_number: account_number.to_owned(),
            bank_code: String::new(),
            currency: "EUR".to_owned(),
            group: false,
            portfolio: false,
            indentation,
            account_type: "Giro account".to_owned(),
            balance: vec![BalanceEntry {
                amount: Decimal::ZERO,
                currency: "EUR".to_owned(),
            }],
        }
    }

    #[test]
    fn iban_derived_for_sepa_account() {
        let rows = annotate_with_bank(vec![
            account("ING", "", 0),
            account("Girokonto", "DE89370400440532013000", 1),
        ]);
        let leaf = rows.iter().find(|r| r.account.name == "Girokonto").unwrap();
        assert_eq!(leaf.iban.as_deref(), Some("DE89370400440532013000"));
    }

    #[test]
    fn iban_normalized_from_spaced_input() {
        let rows = annotate_with_bank(vec![account("Girokonto", "DE89 3704 0044 0532 0130 00", 0)]);
        assert_eq!(rows[0].iban.as_deref(), Some("DE89370400440532013000"));
    }

    #[test]
    fn iban_none_for_paypal_email() {
        let rows = annotate_with_bank(vec![account("PayPal", "user@example.com", 0)]);
        assert!(rows[0].iban.is_none());
    }

    #[test]
    fn iban_none_for_legacy_number() {
        let rows = annotate_with_bank(vec![account("Legacy", "123456789", 0)]);
        assert!(rows[0].iban.is_none());
    }

    #[test]
    fn iban_field_is_valid_filter_name() {
        assert!(AccountRow::valid_fields().contains(&"iban"));
    }
}
