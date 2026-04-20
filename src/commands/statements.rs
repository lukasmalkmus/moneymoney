//! `mm statements` — list and retrieve statement PDFs.

use std::io::Write as _;
use std::path::PathBuf;

use time::Date;

use crate::moneymoney::MoneyMoneyError;
use crate::output::{DetailView, FieldFilter, FieldNames, OutputFormat, Tabular, format_list};
use crate::statements::{self, Statement};

/// Options controlling `mm statements list`.
pub struct ListOptions {
    /// Optional account filter; matches the trailing digit group embedded
    /// in filenames against this account-number hint.
    pub account: Option<String>,
    /// Earliest statement date to include (YYYY-MM).
    pub since: Option<Date>,
    pub format: Option<OutputFormat>,
    pub fields: Option<String>,
}

/// Options controlling `mm statements get`.
pub struct GetOptions {
    pub filename: String,
    pub open: bool,
    pub stdout: bool,
}

impl Tabular for Statement {
    fn headers() -> &'static [&'static str] {
        &["Bank", "Date", "Filename", "Account", "Size"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.bank.clone(),
            self.date.map(|d| d.to_string()).unwrap_or_default(),
            self.filename.clone(),
            self.account_hint.clone().unwrap_or_default(),
            format_size(self.size),
        ]
    }
}

impl DetailView for Statement {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Bank", self.bank.clone()),
            ("Filename", self.filename.clone()),
            ("Path", self.path.display().to_string()),
            ("Date", self.date.map(|d| d.to_string()).unwrap_or_default()),
            ("Account", self.account_hint.clone().unwrap_or_default()),
            ("Size", format_size(self.size)),
        ]
    }
}

impl FieldNames for Statement {
    fn valid_fields() -> &'static [&'static str] {
        &["bank", "date", "filename", "path", "account", "size"]
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "display formatting; precision loss at > 9 PB is tolerable"
)]
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// `mm statements list` entrypoint.
pub fn run_list(opts: &ListOptions) -> anyhow::Result<()> {
    let root = statements::default_root();
    let mut items = statements::walk(&root)?;

    // Filter by account. See [`statements::matches_account`] for the
    // accepted forms (Bank/Name path, IBAN, digit suffix, bare bank).
    if let Some(account) = &opts.account {
        items.retain(|s| statements::matches_account(s, account));
    }

    if let Some(since) = opts.since {
        items.retain(|s| s.date.is_some_and(|d| d >= since));
    }

    let total = items.len();
    let format = OutputFormat::resolve(opts.format);
    let filter = opts
        .fields
        .as_deref()
        .map(FieldFilter::parse::<Statement>)
        .transpose()?;
    format_list(&items, total, format, filter.as_ref())
}

/// `mm statements get` entrypoint. Prints the absolute path (default) or the
/// PDF bytes (with `--stdout`). `--open` launches the default PDF viewer.
pub fn run_get(opts: &GetOptions) -> anyhow::Result<()> {
    let root = statements::default_root();
    let items = statements::walk(&root)?;
    let found = items
        .into_iter()
        .find(|s| s.filename == opts.filename)
        .ok_or_else(|| MoneyMoneyError::AccountNotFound(opts.filename.clone()))?;

    if opts.open {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .arg(&found.path)
                .status()?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            return Err(MoneyMoneyError::NotSupported.into());
        }
    } else if opts.stdout {
        let bytes = std::fs::read(&found.path)?;
        std::io::stdout().write_all(&bytes)?;
    } else {
        println!("{}", absolute_path(found.path)?.display());
    }
    Ok(())
}

fn absolute_path(p: PathBuf) -> std::io::Result<PathBuf> {
    if p.is_absolute() {
        Ok(p)
    } else {
        std::env::current_dir().map(|d| d.join(p))
    }
}
