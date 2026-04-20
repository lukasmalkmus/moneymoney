//! `mm transfer {create,direct-debit,batch}` — initiate SEPA payments via
//! MoneyMoney's AppleScript interface.
//!
//! All three verbs are safe-by-construction: MoneyMoney either opens a
//! pre-filled transfer window (default) or drops the payment into the
//! Ausgangskorb (`--into-outbox`). Either way the user still has to
//! confirm and enter a TAN before money moves.

use std::collections::HashMap;
use std::path::PathBuf;

use rust_decimal::Decimal;
use time::Date;

use crate::applescript::OsascriptRunner;
use crate::commands::accounts::{annotate_with_bank, fetch_all};
use crate::moneymoney::MoneyMoneyError;
use crate::moneymoney::resolver::Resolver;

/// Reject characters that can't be safely embedded in an `AppleScript`
/// double-quoted string. `"` and newlines break parsing; backslashes are
/// permitted and escaped by [`escape_for_script`] (MoneyMoney uses `\` to
/// separate nested category names, so rejecting it would break legitimate
/// input).
pub fn validate_text(field: &'static str, s: &str) -> Result<(), MoneyMoneyError> {
    for ch in s.chars() {
        if ch == '"' || ch == '\n' || ch == '\r' {
            return Err(MoneyMoneyError::InvalidScriptInput { field, ch });
        }
    }
    Ok(())
}

/// Escape a validated string for interpolation into an `AppleScript`
/// double-quoted literal. Only `\` needs doubling; `"` and newlines are
/// rejected upstream by [`validate_text`].
#[must_use]
pub fn escape_for_script(s: &str) -> String {
    s.replace('\\', "\\\\")
}

/// Parameters for `mm transfer create`.
pub struct CreateTransferOptions {
    pub from: String,
    pub to_iban: String,
    pub to_name: Option<String>,
    pub amount: Decimal,
    pub purpose: Option<String>,
    pub endtoend_reference: Option<String>,
    pub scheduled_date: Option<Date>,
    pub into_outbox: bool,
    pub aliases: HashMap<String, String>,
    pub format: Option<crate::output::OutputFormat>,
}

/// Parameters for `mm transfer direct-debit`.
pub struct CreateDirectDebitOptions {
    pub from: String,
    pub debtor_iban: String,
    pub debtor_name: Option<String>,
    pub amount: Decimal,
    pub purpose: Option<String>,
    pub mandate_reference: String,
    pub mandate_date: Option<Date>,
    pub scheduled_date: Option<Date>,
    pub into_outbox: bool,
    pub aliases: HashMap<String, String>,
    pub format: Option<crate::output::OutputFormat>,
}

/// Parameters for `mm transfer batch`.
pub struct BatchTransferOptions {
    pub sepa_xml_path: PathBuf,
    pub direct_debit: bool,
    pub format: Option<crate::output::OutputFormat>,
}

/// `mm transfer create` — build script, dispatch, echo a confirmation.
pub async fn run_create<R: OsascriptRunner>(
    runner: &R,
    opts: &CreateTransferOptions,
) -> anyhow::Result<()> {
    let raw = fetch_all(runner).await?;
    let rows = annotate_with_bank(raw);
    let resolver = Resolver::new(rows, opts.aliases.clone());
    let from_row = resolver.resolve(&opts.from)?;

    let script = build_create_transfer_script(&from_row.account.account_number, opts)?;
    runner.run(&script).await?;
    announce(opts.into_outbox, opts.format, "bank transfer");
    Ok(())
}

/// `mm transfer direct-debit` — build script, dispatch, echo a confirmation.
pub async fn run_direct_debit<R: OsascriptRunner>(
    runner: &R,
    opts: &CreateDirectDebitOptions,
) -> anyhow::Result<()> {
    let raw = fetch_all(runner).await?;
    let rows = annotate_with_bank(raw);
    let resolver = Resolver::new(rows, opts.aliases.clone());
    let from_row = resolver.resolve(&opts.from)?;

    let script = build_direct_debit_script(&from_row.account.account_number, opts)?;
    runner.run(&script).await?;
    announce(opts.into_outbox, opts.format, "direct debit");
    Ok(())
}

/// `mm transfer batch` — load a SEPA XML file.
pub async fn run_batch<R: OsascriptRunner>(
    runner: &R,
    opts: &BatchTransferOptions,
) -> anyhow::Result<()> {
    let script = build_batch_script(opts)?;
    runner.run(&script).await?;
    let verb = if opts.direct_debit {
        "batch direct debit"
    } else {
        "batch transfer"
    };
    announce(false, opts.format, verb);
    Ok(())
}

fn announce(into_outbox: bool, format: Option<crate::output::OutputFormat>, verb: &str) {
    let format = crate::output::OutputFormat::resolve(format);
    let destination = if into_outbox {
        "queued into the MoneyMoney outbox"
    } else {
        "opened in a MoneyMoney payment window; confirm in the GUI and enter TAN"
    };
    match format {
        crate::output::OutputFormat::Json | crate::output::OutputFormat::Ndjson => {
            let payload = serde_json::json!({
                "action": verb,
                "delivery": if into_outbox { "outbox" } else { "window" },
                "message": destination,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
        }
        crate::output::OutputFormat::Table => {
            println!("{verb}: {destination}");
        }
    }
}

/// Build the `create bank transfer` `AppleScript` string.
pub fn build_create_transfer_script(
    from_iban: &str,
    opts: &CreateTransferOptions,
) -> Result<String, MoneyMoneyError> {
    validate_text("from", from_iban)?;
    validate_text("to", &opts.to_iban)?;
    if let Some(name) = &opts.to_name {
        validate_text("name", name)?;
    }
    if let Some(purpose) = &opts.purpose {
        validate_text("purpose", purpose)?;
    }
    if let Some(e2e) = &opts.endtoend_reference {
        validate_text("endtoend reference", e2e)?;
    }

    let mut parts = vec![
        format!("from account \"{from_iban}\""),
        format!("iban \"{}\"", opts.to_iban),
        format!("amount {}", opts.amount),
    ];
    if let Some(name) = &opts.to_name {
        parts.push(format!("to \"{}\"", escape_for_script(name)));
    }
    if let Some(purpose) = &opts.purpose {
        parts.push(format!("purpose \"{}\"", escape_for_script(purpose)));
    }
    if let Some(e2e) = &opts.endtoend_reference {
        parts.push(format!("endtoend reference \"{}\"", escape_for_script(e2e)));
    }
    if let Some(date) = opts.scheduled_date {
        parts.push(format!("scheduled date \"{date}\""));
    }
    if opts.into_outbox {
        parts.push("into \"outbox\"".to_owned());
    }

    Ok(format!(
        "tell application \"MoneyMoney\" to create bank transfer {}",
        parts.join(" ")
    ))
}

/// Build the `create direct debit` `AppleScript` string.
pub fn build_direct_debit_script(
    from_iban: &str,
    opts: &CreateDirectDebitOptions,
) -> Result<String, MoneyMoneyError> {
    validate_text("from", from_iban)?;
    validate_text("to", &opts.debtor_iban)?;
    if let Some(name) = &opts.debtor_name {
        validate_text("name", name)?;
    }
    if let Some(purpose) = &opts.purpose {
        validate_text("purpose", purpose)?;
    }
    validate_text("mandate reference", &opts.mandate_reference)?;

    let mut parts = vec![
        format!("from account \"{from_iban}\""),
        format!("iban \"{}\"", opts.debtor_iban),
        format!("amount {}", opts.amount),
        format!("mandate reference \"{}\"", opts.mandate_reference),
    ];
    if let Some(name) = &opts.debtor_name {
        parts.push(format!("for \"{}\"", escape_for_script(name)));
    }
    if let Some(purpose) = &opts.purpose {
        parts.push(format!("purpose \"{}\"", escape_for_script(purpose)));
    }
    if let Some(date) = opts.mandate_date {
        parts.push(format!("mandate date \"{date}\""));
    }
    if let Some(date) = opts.scheduled_date {
        parts.push(format!("scheduled date \"{date}\""));
    }
    if opts.into_outbox {
        parts.push("into \"outbox\"".to_owned());
    }

    Ok(format!(
        "tell application \"MoneyMoney\" to create direct debit {}",
        parts.join(" ")
    ))
}

/// Build the `create batch {transfer,direct debit}` `AppleScript` string.
pub fn build_batch_script(opts: &BatchTransferOptions) -> Result<String, MoneyMoneyError> {
    let path = opts.sepa_xml_path.to_string_lossy().into_owned();
    validate_text("file path", &path)?;
    let verb = if opts.direct_debit {
        "create batch direct debit"
    } else {
        "create batch transfer"
    };
    Ok(format!(
        "tell application \"MoneyMoney\" to {verb} from POSIX file \"{path}\""
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

    fn base_create() -> CreateTransferOptions {
        CreateTransferOptions {
            from: "ING/Girokonto".to_owned(),
            to_iban: "DE89370400440532013000".to_owned(),
            to_name: Some("Alice".to_owned()),
            amount: Decimal::from_str("12.34").unwrap(),
            purpose: Some("Rent".to_owned()),
            endtoend_reference: None,
            scheduled_date: None,
            into_outbox: false,
            aliases: HashMap::new(),
            format: None,
        }
    }

    #[test]
    fn create_transfer_script_matches_expected_shape() {
        let script =
            build_create_transfer_script("DE92500105175437633269", &base_create()).unwrap();
        assert_eq!(
            script,
            r#"tell application "MoneyMoney" to create bank transfer from account "DE92500105175437633269" iban "DE89370400440532013000" amount 12.34 to "Alice" purpose "Rent""#
        );
    }

    #[test]
    fn into_outbox_appends_clause() {
        let mut opts = base_create();
        opts.into_outbox = true;
        let script = build_create_transfer_script("DE92500105175437633269", &opts).unwrap();
        assert!(script.ends_with(r#"into "outbox""#));
    }

    #[test]
    fn quote_in_purpose_is_rejected() {
        let mut opts = base_create();
        opts.purpose = Some(r#"He said "hi""#.to_owned());
        let err = build_create_transfer_script("DE92500105175437633269", &opts).unwrap_err();
        match err {
            MoneyMoneyError::InvalidScriptInput { field, ch } => {
                assert_eq!(field, "purpose");
                assert_eq!(ch, '"');
            }
            other => panic!("expected InvalidScriptInput, got {other:?}"),
        }
    }

    #[test]
    fn direct_debit_emits_mandate() {
        let opts = CreateDirectDebitOptions {
            from: "ING/Girokonto".to_owned(),
            debtor_iban: "DE89370400440532013000".to_owned(),
            debtor_name: Some("Tenant".to_owned()),
            amount: Decimal::from_str("500.00").unwrap(),
            purpose: None,
            mandate_reference: "MANDATE-42".to_owned(),
            mandate_date: None,
            scheduled_date: None,
            into_outbox: true,
            aliases: HashMap::new(),
            format: None,
        };
        let script = build_direct_debit_script("DE92500105175437633269", &opts).unwrap();
        assert!(script.contains(r#"mandate reference "MANDATE-42""#));
        assert!(script.contains(r#"for "Tenant""#));
        assert!(script.ends_with(r#"into "outbox""#));
    }

    #[test]
    fn batch_script_uses_posix_file() {
        let opts = BatchTransferOptions {
            sepa_xml_path: PathBuf::from("/tmp/sepa.xml"),
            direct_debit: false,
            format: None,
        };
        let script = build_batch_script(&opts).unwrap();
        assert_eq!(
            script,
            r#"tell application "MoneyMoney" to create batch transfer from POSIX file "/tmp/sepa.xml""#
        );
    }
}
