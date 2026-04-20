//! Filesystem-level access to the bank statement PDFs MoneyMoney stores
//! under its app container.
//!
//! MoneyMoney places downloaded statements at
//! `~/Library/Containers/com.moneymoney-app.retail/Data/Library/Application Support/MoneyMoney/Statements/`.
//! The directory structure is a single level of bank-named folders
//! containing PDFs. Filenames follow the general pattern
//! `<type>_<account-or-blank>_<doctype>_YYYYMMDD.pdf`, but the account
//! segment is optional (some statements are bank-wide).
//!
//! There is no AppleScript verb for statements (verified against the
//! official docs), so we read directly from disk.

use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Serialize;
use thiserror::Error;
use time::Date;
use time::macros::format_description;

/// One statement PDF on disk.
#[derive(Debug, Clone, Serialize)]
pub struct Statement {
    /// Bank folder the PDF was found under (e.g., "ING").
    pub bank: String,
    /// File name without path (e.g. `Girokonto_5437633269_Kontoauszug_20260108.pdf`).
    pub filename: String,
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Parsed statement date (derived from the trailing YYYYMMDD in the
    /// filename). `None` when the pattern doesn't match.
    pub date: Option<Date>,
    /// Account number suffix embedded in the filename, if present.
    /// MoneyMoney stores the last ~10 digits of the IBAN for per-account
    /// statements; bank-wide documents have this field empty.
    pub account_hint: Option<String>,
    /// File size in bytes.
    pub size: u64,
}

#[derive(Debug, Error)]
pub enum StatementsError {
    #[error("statements directory not found at {0}")]
    DirectoryMissing(PathBuf),

    #[error("failed to read statements directory: {0}")]
    Io(#[from] std::io::Error),
}

/// Default location of the Statements folder.
#[must_use]
pub fn default_root() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        return home
            .join("Library")
            .join("Containers")
            .join("com.moneymoney-app.retail")
            .join("Data")
            .join("Library")
            .join("Application Support")
            .join("MoneyMoney")
            .join("Statements");
    }
    PathBuf::new()
}

static FILENAME_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)_(?P<account>\d{6,20})_.*_(?P<date>\d{8})\.pdf$")
        .unwrap_or_else(|_| unreachable!("static regex compiles"))
});
static TRAILING_DATE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"_(?P<date>\d{8})\.pdf$").unwrap_or_else(|_| unreachable!("static regex compiles"))
});

/// Walk the statements tree and return every PDF as a [`Statement`].
pub fn walk(root: &Path) -> Result<Vec<Statement>, StatementsError> {
    if !root.exists() {
        return Err(StatementsError::DirectoryMissing(root.to_path_buf()));
    }
    let mut out = Vec::new();
    for bank_entry in std::fs::read_dir(root)? {
        let bank_entry = bank_entry?;
        if !bank_entry.file_type()?.is_dir() {
            continue;
        }
        let bank = bank_entry.file_name().to_string_lossy().into_owned();
        for file_entry in std::fs::read_dir(bank_entry.path())? {
            let file_entry = file_entry?;
            if !file_entry.file_type()?.is_file() {
                continue;
            }
            let filename = file_entry.file_name().to_string_lossy().into_owned();
            if !filename.to_lowercase().ends_with(".pdf") {
                continue;
            }
            let metadata = file_entry.metadata()?;
            let (account_hint, date) = parse_filename(&filename);

            out.push(Statement {
                bank: bank.clone(),
                filename,
                path: file_entry.path(),
                date,
                account_hint,
                size: metadata.len(),
            });
        }
    }

    // Newest first; stable within a day via filename.
    out.sort_by(|a, b| match (b.date, a.date) {
        (Some(bd), Some(ad)) => bd.cmp(&ad).then_with(|| a.filename.cmp(&b.filename)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.filename.cmp(&b.filename),
    });

    Ok(out)
}

/// Whether a [`Statement`] matches a user-supplied `--account` reference.
///
/// Accepted forms (first match wins):
///
/// 1. **`Bank/Name`** (e.g. `"ING/Girokonto"`): bank folder must equal the
///    left side case-insensitively AND the filename must start with the
///    right side followed by `_` (matches `Girokonto_5437633269_…`). This
///    is the form the skill recommends for account resolution and it MUST
///    work here too.
/// 2. **Account number / IBAN / digit suffix** (6+ digits anywhere in the
///    needle): match the trailing digits of the needle against the
///    filename's `account_hint`. Works for bare numbers (`5437633269`) and
///    full IBANs (`DE…5437633269`).
/// 3. **Bare bank name** (e.g. `"ING"`): substring match against the bank
///    folder name, case-insensitive.
///
/// Leading/trailing whitespace in `needle` is ignored.
#[must_use]
pub fn matches_account(statement: &Statement, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return true;
    }

    // Form 1: "Bank/Name" path — require exact bank + filename-prefix.
    if let Some((bank_part, name_part)) = needle.split_once('/') {
        let bank_ok = statement.bank.eq_ignore_ascii_case(bank_part.trim());
        let name_trim = name_part.trim();
        let filename_prefix = format!("{name_trim}_");
        let name_ok = statement
            .filename
            .to_lowercase()
            .starts_with(&filename_prefix.to_lowercase());
        return bank_ok && name_ok;
    }

    // Form 2: digit-bearing ref (account number, IBAN with trailing digits).
    // Extract the trailing digit run; if it's 6+ digits, compare against the
    // filename's account_hint.
    let trailing_digits: String = needle
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if trailing_digits.len() >= 6 {
        if let Some(hint) = statement.account_hint.as_deref() {
            // IBANs end with the account number; bank-supplied hints truncate
            // the leading zeros. Accept either hint == trailing or
            // trailing.ends_with(hint) (IBAN suffix) or hint.ends_with(trailing)
            // (bare number that is longer than the hint).
            if hint == trailing_digits
                || trailing_digits.ends_with(hint)
                || hint.ends_with(&trailing_digits)
            {
                return true;
            }
        }
        // Fall through: a digit ref with no hint match is a miss even if the
        // bank name happens to contain the digits (unlikely but defensible).
        return false;
    }

    // Form 3: bare bank name (or anything else) — substring match on bank.
    statement
        .bank
        .to_lowercase()
        .contains(&needle.to_lowercase())
}

fn parse_filename(filename: &str) -> (Option<String>, Option<Date>) {
    let fmt = format_description!("[year][month][day]");
    if let Some(caps) = FILENAME_RE.captures(filename) {
        let account = caps.name("account").map(|m| m.as_str().to_owned());
        let date = caps
            .name("date")
            .and_then(|m| Date::parse(m.as_str(), &fmt).ok());
        return (account, date);
    }
    let date = TRAILING_DATE_RE.captures(filename).and_then(|caps| {
        caps.name("date")
            .and_then(|m| Date::parse(m.as_str(), &fmt).ok())
    });
    (None, date)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests assert on static regex fixtures")]
mod tests {
    use super::*;

    #[test]
    fn parses_account_and_date() {
        let (account, date) = parse_filename("Girokonto_5437633269_Kontoauszug_20260108.pdf");
        assert_eq!(account.as_deref(), Some("5437633269"));
        assert_eq!(date.unwrap().to_string(), "2026-01-08");
    }

    #[test]
    fn parses_bank_wide_with_only_date() {
        let (account, date) = parse_filename("Information_20260219.pdf");
        assert_eq!(account, None);
        assert_eq!(date.unwrap().to_string(), "2026-02-19");
    }

    fn stmt(bank: &str, filename: &str) -> Statement {
        let (account_hint, date) = parse_filename(filename);
        Statement {
            bank: bank.to_owned(),
            filename: filename.to_owned(),
            path: PathBuf::from(filename),
            date,
            account_hint,
            size: 0,
        }
    }

    #[test]
    fn matches_bank_slash_name() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(matches_account(&s, "ING/Girokonto"));
        assert!(matches_account(&s, "ing/girokonto")); // case-insensitive
    }

    #[test]
    fn bank_slash_name_rejects_wrong_bank() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(!matches_account(&s, "Trade Republic/Girokonto"));
    }

    #[test]
    fn bank_slash_name_rejects_wrong_name() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(!matches_account(&s, "ING/Depot"));
    }

    #[test]
    fn matches_bare_bank() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(matches_account(&s, "ING"));
        assert!(matches_account(&s, "ing"));
    }

    #[test]
    fn matches_account_number() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(matches_account(&s, "5437633269"));
    }

    #[test]
    fn matches_iban_trailing_digits() {
        // IBAN ending in the account's digit hint.
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(matches_account(&s, "DE89370400445437633269"));
    }

    #[test]
    fn digit_ref_does_not_match_unrelated_account() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(!matches_account(&s, "9999999999"));
    }

    #[test]
    fn bank_wide_statements_only_match_bank() {
        let s = stmt("ING", "Information_20260219.pdf");
        assert!(matches_account(&s, "ING"));
        assert!(!matches_account(&s, "5437633269"));
        assert!(!matches_account(&s, "ING/Girokonto"));
    }

    #[test]
    fn empty_needle_keeps_everything() {
        let s = stmt("ING", "Girokonto_5437633269_Kontoauszug_20250601.pdf");
        assert!(matches_account(&s, ""));
        assert!(matches_account(&s, "   "));
    }
}
