//! `mm accounts` — list and inspect MoneyMoney accounts.

use std::collections::HashMap;

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
    /// The underlying account row.
    #[serde(flatten)]
    pub account: Account,
}

/// Walk the flat account list and annotate each row with its parent bank
/// name, derived from the preceding `indentation == 0` entry.
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
            AccountRow { bank, account }
        })
        .collect()
}

impl Tabular for AccountRow {
    fn headers() -> &'static [&'static str] {
        &["Bank", "Name", "Account", "Balance", "Currency"]
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
