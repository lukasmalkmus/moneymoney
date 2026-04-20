//! `mm transactions` — list transactions for an account over a date range.

use std::collections::HashMap;

use time::{Date, OffsetDateTime, UtcOffset};

use crate::applescript::{OsascriptRunner, run_plist};
use crate::commands::accounts::{annotate_with_bank, fetch_all};
use crate::moneymoney::resolver::Resolver;
use crate::moneymoney::types::{Transaction, TransactionsEnvelope};
use crate::output::{DetailView, FieldFilter, FieldNames, OutputFormat, Tabular, format_list};

/// Options controlling `mm transactions`.
pub struct ListOptions {
    pub reference: String,
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub search: Option<String>,
    pub limit: Option<usize>,
    pub format: Option<OutputFormat>,
    pub fields: Option<String>,
    pub aliases: HashMap<String, String>,
}

impl Tabular for Transaction {
    fn headers() -> &'static [&'static str] {
        &["Date", "Name", "Amount", "Currency", "Purpose"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.booking_date
                .map(|d| {
                    let sys: std::time::SystemTime = d.into();
                    let odt: OffsetDateTime = sys.into();
                    odt.date().to_string()
                })
                .unwrap_or_default(),
            self.name.clone(),
            self.amount.to_string(),
            self.currency.clone(),
            truncate(&self.purpose, 80),
        ]
    }
}

impl DetailView for Transaction {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("ID", self.id.to_string()),
            ("Account", self.account_uuid.clone()),
            ("Name", self.name.clone()),
            ("Amount", self.amount.to_string()),
            ("Currency", self.currency.clone()),
            (
                "BookingDate",
                self.booking_date.map_or_else(String::new, format_date),
            ),
            (
                "ValueDate",
                self.value_date.map_or_else(String::new, format_date),
            ),
            ("BookingText", self.booking_text.clone()),
            ("Purpose", self.purpose.clone()),
            ("CategoryUUID", self.category_uuid.clone()),
            ("Comment", self.comment.clone()),
            ("Checkmark", self.checkmark.to_string()),
            ("Booked", self.booked.to_string()),
            ("CounterpartyAccount", self.account_number.clone()),
            ("CounterpartyBankCode", self.bank_code.clone()),
        ]
    }
}

impl FieldNames for Transaction {
    fn valid_fields() -> &'static [&'static str] {
        &[
            "id",
            "account",
            "name",
            "amount",
            "currency",
            "bookingdate",
            "valuedate",
            "bookingtext",
            "purpose",
            "categoryuuid",
            "comment",
            "checkmark",
            "booked",
            "counterpartyaccount",
            "counterpartybankcode",
        ]
    }
}

fn format_date(d: plist::Date) -> String {
    let sys: std::time::SystemTime = d.into();
    let odt: OffsetDateTime = sys.into();
    odt.date().to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// `mm transactions` entrypoint.
pub async fn run_list<R: OsascriptRunner>(runner: &R, opts: ListOptions) -> anyhow::Result<()> {
    // Resolve account first (uses the cached accounts plist).
    let raw = fetch_all(runner).await?;
    let rows = annotate_with_bank(raw);
    let resolver = Resolver::new(rows, opts.aliases);
    let account_row = resolver.resolve(&opts.reference)?;

    // Default range: last 90 days to today.
    let today = OffsetDateTime::now_utc().to_offset(UtcOffset::UTC).date();
    let from = opts
        .from
        .unwrap_or_else(|| today - time::Duration::days(90));
    let to = opts.to.unwrap_or(today);

    let script = build_export_script(&account_row.account.account_number, from, to);
    let envelope: TransactionsEnvelope = run_plist(runner, &script).await?;

    // Apply search filter (case-insensitive; matches name + purpose +
    // comment — categoryUuid rarely makes a useful haystack).
    let needle = opts.search.as_deref().map(str::to_lowercase);
    let filtered: Vec<Transaction> = envelope
        .transactions
        .into_iter()
        .filter(|t| match &needle {
            Some(q) => {
                let hay = [&t.name, &t.purpose, &t.comment];
                hay.iter().any(|h| h.to_lowercase().contains(q.as_str()))
            }
            None => true,
        })
        .collect();
    let total = filtered.len();
    let truncated: Vec<Transaction> = match opts.limit {
        Some(n) => filtered.into_iter().take(n).collect(),
        None => filtered,
    };

    let format = OutputFormat::resolve(opts.format);
    let field_filter = opts
        .fields
        .as_deref()
        .map(FieldFilter::parse::<Transaction>)
        .transpose()?;

    format_list(&truncated, total, format, field_filter.as_ref())
}

/// Build the AppleScript string for `export transactions` over a date
/// range. Exposed so the MCP tool can reuse it without going through the
/// CLI options struct.
#[must_use]
pub fn build_export_script(account: &str, from: Date, to: Date) -> String {
    // MoneyMoney's AppleScript date format is locale-neutral ISO:
    // `date "YYYY-MM-DD"` is accepted.
    format!(
        "tell application \"MoneyMoney\" to export transactions from account \"{account}\" from date \"{from}\" to date \"{to}\" as \"plist\""
    )
}
