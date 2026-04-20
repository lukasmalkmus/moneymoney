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
}
