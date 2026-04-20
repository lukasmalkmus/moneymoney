//! `mm transaction {add,set}` — offline-account entries and metadata edits.
//!
//! `add transaction` only works against offline-managed accounts; we reject
//! real bank accounts at resolve time so users don't silently fail at the
//! MoneyMoney layer. `set transaction` is the one silent write verb we
//! expose — no GUI confirmation, no TAN — so the plan marks its MCP tool
//! as `destructiveHint: true`.

use std::collections::HashMap;

use rust_decimal::Decimal;
use time::Date;

use crate::applescript::OsascriptRunner;
use crate::commands::accounts::{annotate_with_bank, fetch_all};
use crate::commands::transfer::{escape_for_script, validate_text};
use crate::moneymoney::MoneyMoneyError;
use crate::moneymoney::resolver::Resolver;

/// Parameters for `mm transaction add`.
pub struct AddOptions {
    pub account: String,
    pub date: Date,
    pub name: String,
    pub amount: Decimal,
    pub purpose: Option<String>,
    pub category: Option<String>,
    pub aliases: HashMap<String, String>,
    pub format: Option<crate::output::OutputFormat>,
}

/// Parameters for `mm transaction set`.
pub struct SetOptions {
    pub id: i64,
    pub checkmark: Option<bool>,
    pub category: Option<String>,
    pub comment: Option<String>,
    pub format: Option<crate::output::OutputFormat>,
}

/// `mm transaction add` — append a manual entry to an offline account.
pub async fn run_add<R: OsascriptRunner>(runner: &R, opts: &AddOptions) -> anyhow::Result<()> {
    let raw = fetch_all(runner).await?;
    let rows = annotate_with_bank(raw);
    let resolver = Resolver::new(rows, opts.aliases.clone());
    let row = resolver.resolve(&opts.account)?;

    // MoneyMoney exposes an `offline` flag inside `attributes` on offline
    // accounts. We don't currently decode `attributes`; the safer check is
    // to require `bank_code` empty AND `account_number` that isn't a SEPA
    // IBAN (country-code prefixed). An outright heuristic, but paired with
    // MoneyMoney's own guard it avoids silent footguns.
    //
    // A more precise check would require adding a decoded attribute map;
    // leaving that as follow-up once real offline accounts exist to test
    // against.
    if !looks_like_offline(&row.account) {
        return Err(MoneyMoneyError::AccountNotOffline(format!(
            "{}/{}",
            row.bank, row.account.name
        ))
        .into());
    }

    let script = build_add_transaction_script(&row.account.account_number, opts)?;
    runner.run(&script).await?;
    announce(opts.format, "transaction added to offline account");
    Ok(())
}

/// `mm transaction set` — mutate checkmark / category / comment on an
/// existing transaction.
pub async fn run_set<R: OsascriptRunner>(runner: &R, opts: &SetOptions) -> anyhow::Result<()> {
    let script = build_set_transaction_script(opts)?;
    runner.run(&script).await?;
    announce(opts.format, "transaction metadata updated");
    Ok(())
}

fn looks_like_offline(account: &crate::moneymoney::types::Account) -> bool {
    // IBAN-style accounts are never offline; bank code is usually populated
    // on real bank accounts; a non-empty `type` like "Offline account" or
    // "Manual" may also appear.
    let number = &account.account_number;
    let is_iban = number.len() >= 15 && number.chars().take(2).all(|c| c.is_ascii_alphabetic());
    !is_iban && account.bank_code.is_empty()
}

fn announce(format: Option<crate::output::OutputFormat>, message: &str) {
    let fmt = crate::output::OutputFormat::resolve(format);
    match fmt {
        crate::output::OutputFormat::Json | crate::output::OutputFormat::Ndjson => {
            let payload = serde_json::json!({ "message": message });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
        }
        crate::output::OutputFormat::Table => println!("{message}"),
    }
}

/// Build the `add transaction` `AppleScript` string.
pub fn build_add_transaction_script(
    account_id: &str,
    opts: &AddOptions,
) -> Result<String, MoneyMoneyError> {
    validate_text("account", account_id)?;
    validate_text("name", &opts.name)?;
    if let Some(purpose) = &opts.purpose {
        validate_text("purpose", purpose)?;
    }
    if let Some(category) = &opts.category {
        validate_text("category", category)?;
    }

    let mut parts = vec![
        format!("to account \"{}\"", escape_for_script(account_id)),
        format!("on date \"{}\"", opts.date),
        format!("to \"{}\"", escape_for_script(&opts.name)),
        format!("amount {}", opts.amount),
    ];
    if let Some(purpose) = &opts.purpose {
        parts.push(format!("purpose \"{}\"", escape_for_script(purpose)));
    }
    if let Some(category) = &opts.category {
        parts.push(format!("category \"{}\"", escape_for_script(category)));
    }

    Ok(format!(
        "tell application \"MoneyMoney\" to add transaction {}",
        parts.join(" ")
    ))
}

/// Build the `set transaction` `AppleScript` string.
pub fn build_set_transaction_script(opts: &SetOptions) -> Result<String, MoneyMoneyError> {
    if opts.checkmark.is_none() && opts.category.is_none() && opts.comment.is_none() {
        return Err(MoneyMoneyError::ScriptError(
            "`mm transaction set` requires at least one of --checkmark, --category, or --comment"
                .to_owned(),
        ));
    }

    let mut parts = vec![format!("id {}", opts.id)];
    if let Some(on) = opts.checkmark {
        parts.push(format!(
            "checkmark to \"{}\"",
            if on { "on" } else { "off" }
        ));
    }
    if let Some(category) = &opts.category {
        validate_text("category", category)?;
        parts.push(format!("category to \"{}\"", escape_for_script(category)));
    }
    if let Some(comment) = &opts.comment {
        validate_text("comment", comment)?;
        parts.push(format!("comment to \"{}\"", escape_for_script(comment)));
    }

    Ok(format!(
        "tell application \"MoneyMoney\" to set transaction {}",
        parts.join(" ")
    ))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests assert on hand-constructed fixtures"
)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn add_transaction_script_includes_all_parts() {
        let opts = AddOptions {
            account: "offline-cash".to_owned(),
            date: Date::from_calendar_date(2026, time::Month::January, 15).unwrap(),
            name: "Coffee".to_owned(),
            amount: Decimal::from_str("-3.50").unwrap(),
            purpose: Some("morning".to_owned()),
            category: Some("Food\\Coffee".to_owned()),
            aliases: HashMap::new(),
            format: None,
        };
        let script = build_add_transaction_script("CASH", &opts).unwrap();
        assert_eq!(
            script,
            r#"tell application "MoneyMoney" to add transaction to account "CASH" on date "2026-01-15" to "Coffee" amount -3.50 purpose "morning" category "Food\\Coffee""#
        );
    }

    #[test]
    fn set_transaction_requires_at_least_one_field() {
        let opts = SetOptions {
            id: 42,
            checkmark: None,
            category: None,
            comment: None,
            format: None,
        };
        match build_set_transaction_script(&opts) {
            Err(MoneyMoneyError::ScriptError(_)) => {}
            other => panic!("expected ScriptError, got {other:?}"),
        }
    }

    #[test]
    fn set_transaction_checkmark_uses_on_off() {
        let opts = SetOptions {
            id: 42,
            checkmark: Some(true),
            category: None,
            comment: None,
            format: None,
        };
        let script = build_set_transaction_script(&opts).unwrap();
        assert_eq!(
            script,
            r#"tell application "MoneyMoney" to set transaction id 42 checkmark to "on""#
        );
    }

    #[test]
    fn set_transaction_combines_fields() {
        let opts = SetOptions {
            id: 7,
            checkmark: Some(false),
            category: Some("Food\\Eating out".to_owned()),
            comment: Some("reimbursed".to_owned()),
            format: None,
        };
        let script = build_set_transaction_script(&opts).unwrap();
        assert!(script.contains(r#"checkmark to "off""#));
        assert!(script.contains(r#"category to "Food\\Eating out""#));
        assert!(script.contains(r#"comment to "reimbursed""#));
    }
}
