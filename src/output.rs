//! Output formatting primitives: table / JSON / NDJSON, with optional field
//! filtering. Ported from the sister project `1nitetent` with tweaks for the
//! `mm` domain.

use std::io::{self, IsTerminal as _, Write as _};

use comfy_table::presets::ASCII_MARKDOWN;
use comfy_table::{ContentArrangement, Table};
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// Effective output format chosen by the CLI.
#[derive(Debug, Clone, Copy, Default, clap::ValueEnum, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Table,
    Json,
    Ndjson,
}

impl OutputFormat {
    /// Resolve the effective output format: explicit flag wins; otherwise
    /// `Table` when stdout is a TTY and `Json` when piped.
    #[must_use]
    pub fn resolve(explicit: Option<Self>) -> Self {
        explicit.unwrap_or_else(|| {
            if io::stdout().is_terminal() {
                Self::Table
            } else {
                Self::Json
            }
        })
    }
}

/// Types that can be rendered as a row in a table.
pub trait Tabular {
    /// Column headers in display order.
    fn headers() -> &'static [&'static str];
    /// One cell per header, stringified.
    fn row(&self) -> Vec<String>;
}

/// Types that can be rendered as a detail view (key/value pairs).
pub trait DetailView {
    /// Ordered `(field name, stringified value)` pairs.
    fn fields(&self) -> Vec<(&'static str, String)>;
}

/// Types that advertise a fixed set of field names so `FieldFilter` can
/// validate the user's `-F field,field,...` input.
pub trait FieldNames {
    fn valid_fields() -> &'static [&'static str];

    /// `(field, JSON key)` pairs for fields whose key differs by more than case.
    fn json_aliases() -> &'static [(&'static str, &'static str)] {
        &[]
    }
}

/// A user-supplied `-F` filter.
#[derive(Debug, Clone)]
pub struct FieldFilter {
    fields: Vec<String>,
    json_keys: Vec<String>,
}

/// Error returned when `-F` mentions a field not declared by `FieldNames`.
#[derive(Debug, Error)]
#[error("invalid field '{field}', valid fields: {}", valid.join(", "))]
pub struct FieldFilterError {
    pub field: String,
    pub valid: Vec<String>,
}

impl FieldFilter {
    /// Parse a comma-separated field list, rejecting unknown names.
    pub fn parse<T: FieldNames>(input: &str) -> Result<Self, FieldFilterError> {
        let valid = T::valid_fields();
        let mut fields: Vec<String> = Vec::new();
        for f in input.split(',').map(str::trim) {
            if !valid.contains(&f) {
                return Err(FieldFilterError {
                    field: f.to_owned(),
                    valid: valid.iter().map(|&s| s.to_owned()).collect(),
                });
            }
            if !fields.iter().any(|seen| seen == f) {
                fields.push(f.to_owned());
            }
        }
        let json_keys = fields
            .iter()
            .map(|f| {
                T::json_aliases()
                    .iter()
                    .find(|(field, _)| field == f)
                    .map_or_else(|| f.clone(), |(_, key)| (*key).to_owned())
            })
            .collect();
        Ok(Self { fields, json_keys })
    }

    fn filter_json_value(&self, value: &Value) -> Value {
        let selected = self
            .json_keys
            .iter()
            .map(|key| match member(value, key) {
                Some((k, v)) => (k.clone(), v.clone()),
                None => (key.clone(), Value::Null),
            })
            .collect();
        Value::Object(selected)
    }

    fn cells(&self, value: &Value) -> Vec<String> {
        self.json_keys
            .iter()
            .map(|key| match member(value, key) {
                Some((_, Value::String(s))) => s.clone(),
                Some((_, other)) => other.to_string(),
                None => "null".to_owned(),
            })
            .collect()
    }
}

fn member<'a>(value: &'a Value, key: &str) -> Option<(&'a String, &'a Value)> {
    value
        .as_object()?
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
}

/// Render a list of items with the standard envelope.
pub fn format_list<T: Tabular + Serialize>(
    items: &[T],
    total_count: usize,
    format: OutputFormat,
    filter: Option<&FieldFilter>,
) -> anyhow::Result<()> {
    write_list(&mut io::stdout().lock(), items, total_count, format, filter)
}

fn write_list<T: Tabular + Serialize>(
    out: &mut impl io::Write,
    items: &[T],
    total_count: usize,
    format: OutputFormat,
    filter: Option<&FieldFilter>,
) -> anyhow::Result<()> {
    let showing = items.len();
    let has_more = showing < total_count;

    match format {
        OutputFormat::Json => {
            let json_items: Vec<Value> = items
                .iter()
                .map(|item| {
                    let v = serde_json::to_value(item)?;
                    Ok(filter.map_or_else(|| v.clone(), |f| f.filter_json_value(&v)))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let envelope = serde_json::json!({
                "results": json_items,
                "total_count": total_count,
                "showing": showing,
                "has_more": has_more,
            });
            serde_json::to_writer_pretty(&mut *out, &envelope)?;
            writeln!(out)?;
        }
        OutputFormat::Ndjson => {
            let meta = serde_json::json!({
                "_meta": true,
                "total_count": total_count,
                "showing": showing,
                "has_more": has_more,
            });
            serde_json::to_writer(&mut *out, &meta)?;
            writeln!(out)?;
            for item in items {
                let v = serde_json::to_value(item)?;
                let v = filter.map_or_else(|| v.clone(), |f| f.filter_json_value(&v));
                serde_json::to_writer(&mut *out, &v)?;
                writeln!(out)?;
            }
        }
        OutputFormat::Table => render_table(out, items, filter, has_more, showing, total_count)?,
    }
    Ok(())
}

fn render_table<T: Tabular + Serialize>(
    out: &mut impl io::Write,
    items: &[T],
    filter: Option<&FieldFilter>,
    has_more: bool,
    showing: usize,
    total_count: usize,
) -> anyhow::Result<()> {
    let mut table = Table::new();
    table.load_style(ASCII_MARKDOWN);
    table.set_content_arrangement(ContentArrangement::Dynamic);

    if let Some(f) = filter {
        table.set_header(&f.fields);
        for item in items {
            table.add_row(f.cells(&serde_json::to_value(item)?));
        }
    } else {
        table.set_header(T::headers());
        for item in items {
            table.add_row(item.row());
        }
    }

    writeln!(out, "{table}")?;
    if has_more {
        writeln!(out, "\nShowing {showing} of {total_count} results")?;
    }
    Ok(())
}

/// Render a single detail view.
pub fn format_detail<T: DetailView + Serialize>(
    item: &T,
    format: OutputFormat,
    filter: Option<&FieldFilter>,
) -> anyhow::Result<()> {
    write_detail(&mut io::stdout().lock(), item, format, filter)
}

fn write_detail<T: DetailView + Serialize>(
    out: &mut impl io::Write,
    item: &T,
    format: OutputFormat,
    filter: Option<&FieldFilter>,
) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            let v = serde_json::to_value(item)?;
            let v = filter.map_or_else(|| v.clone(), |f| f.filter_json_value(&v));
            if matches!(format, OutputFormat::Json) {
                serde_json::to_writer_pretty(&mut *out, &v)?;
            } else {
                serde_json::to_writer(&mut *out, &v)?;
            }
            writeln!(out)?;
        }
        OutputFormat::Table => {
            let mut table = Table::new();
            table.load_style(ASCII_MARKDOWN);
            table.set_content_arrangement(ContentArrangement::Dynamic);
            table.set_header(["Field", "Value"]);
            if let Some(f) = filter {
                let values = f.cells(&serde_json::to_value(item)?);
                for (name, value) in f.fields.iter().zip(&values) {
                    table.add_row([name, value]);
                }
            } else {
                for (name, value) in &item.fields() {
                    table.add_row([*name, value.as_str()]);
                }
            }
            writeln!(out, "{table}")?;
        }
    }
    Ok(())
}

/// Print a structured error record to stderr.
pub fn print_json_error(error: &anyhow::Error, code: &str) {
    let msg = serde_json::json!({
        "error": format!("{error:#}"),
        "code": code,
    });
    let _ = writeln!(
        io::stderr(),
        "{}",
        serde_json::to_string(&msg).unwrap_or_default()
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests assert on static JSON fixtures")]
pub(crate) mod tests {
    use std::path::PathBuf;

    use rust_decimal::Decimal;
    use serde_json::json;

    use super::*;
    use crate::commands::accounts::AccountRow;
    use crate::moneymoney::types::{BalanceEntry, Category, Security, Transaction};
    use crate::statements::Statement;

    fn selected_keys<T: FieldNames + Serialize>(item: &T, fields: &str) -> Vec<String> {
        let filter = FieldFilter::parse::<T>(fields).unwrap();
        let value = filter.filter_json_value(&serde_json::to_value(item).unwrap());
        value.as_object().unwrap().keys().cloned().collect()
    }

    fn list<T: Tabular + FieldNames + Serialize>(
        items: &[T],
        format: OutputFormat,
        fields: Option<&str>,
    ) -> String {
        let filter = fields.map(|f| FieldFilter::parse::<T>(f).unwrap());
        let mut out = Vec::new();
        write_list(&mut out, items, items.len(), format, filter.as_ref()).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn detail<T: DetailView + FieldNames + Serialize>(
        item: &T,
        format: OutputFormat,
        fields: Option<&str>,
    ) -> String {
        let filter = fields.map(|f| FieldFilter::parse::<T>(f).unwrap());
        let mut out = Vec::new();
        write_detail(&mut out, item, format, filter.as_ref()).unwrap();
        String::from_utf8(out).unwrap()
    }

    fn cell(table: &str, line: usize) -> &str {
        table.lines().nth(line).unwrap().trim_matches('|').trim()
    }

    fn assert_selects_only<T: FieldNames>(record: &Value, field: &str, may_be_null: bool) {
        let object = record.as_object().unwrap();
        assert_eq!(object.len(), 1, "-F {field}");
        let (key, value) = object.iter().next().unwrap();
        let expected = T::json_aliases()
            .iter()
            .find(|(name, _)| *name == field)
            .map_or(field, |(_, key)| key);
        assert!(key.eq_ignore_ascii_case(expected), "-F {field}: {key}");
        assert!(may_be_null || !value.is_null(), "-F {field}");
    }

    fn assert_every_field_appears_in_every_format<T>(item: &T, may_be_null: bool)
    where
        T: Tabular + DetailView + FieldNames + Serialize,
    {
        let items = std::slice::from_ref(item);
        for field in T::valid_fields() {
            let selected = Some(*field);

            let json = list(items, OutputFormat::Json, selected);
            let json: Value = serde_json::from_str(&json).unwrap();
            assert_selects_only::<T>(&json["results"][0], field, may_be_null);

            let ndjson = list(items, OutputFormat::Ndjson, selected);
            let record = ndjson.lines().nth(1).unwrap();
            let record: Value = serde_json::from_str(record).unwrap();
            assert_selects_only::<T>(&record, field, may_be_null);

            let json = detail(item, OutputFormat::Json, selected);
            let json: Value = serde_json::from_str(&json).unwrap();
            assert_selects_only::<T>(&json, field, may_be_null);

            let ndjson = detail(item, OutputFormat::Ndjson, selected);
            assert_eq!(ndjson.lines().count(), 1, "-F {field}");
            let record: Value = serde_json::from_str(&ndjson).unwrap();
            assert_selects_only::<T>(&record, field, may_be_null);

            let table = list(items, OutputFormat::Table, selected);
            assert_eq!(cell(&table, 0), *field);

            let table = detail(item, OutputFormat::Table, selected);
            let row = table.lines().nth(2).unwrap();
            assert!(row.starts_with(&format!("| {field} ")), "{row}");
        }
    }

    pub(crate) fn account_row() -> AccountRow {
        AccountRow {
            bank: "ING".to_owned(),
            iban: Some("DE89370400440532013000".to_owned()),
            account: serde_json::from_value(json!({
                "uuid": "u",
                "name": "Giro",
                "owner": "o",
                "accountNumber": "n",
                "bankCode": "b",
                "currency": "EUR",
                "group": false,
                "portfolio": false,
                "indentation": 1,
                "type": "t",
            }))
            .unwrap(),
        }
    }

    fn transaction() -> Transaction {
        serde_json::from_value(json!({})).unwrap()
    }

    pub(crate) fn category() -> Category {
        serde_json::from_value(json!({"name": "Food"})).unwrap()
    }

    pub(crate) fn security() -> Security {
        serde_json::from_value(json!({})).unwrap()
    }

    pub(crate) fn statement() -> Statement {
        Statement {
            bank: "ING".to_owned(),
            filename: "f.pdf".to_owned(),
            path: PathBuf::from("/f.pdf"),
            date: None,
            account_hint: None,
            size: 0,
        }
    }

    fn account_row_without_iban() -> AccountRow {
        AccountRow {
            iban: None,
            ..account_row()
        }
    }

    fn transaction_with_dates() -> Transaction {
        let date = plist::Date::from(std::time::SystemTime::UNIX_EPOCH);
        Transaction {
            booking_date: Some(date),
            value_date: Some(date),
            ..transaction()
        }
    }

    fn statement_with_date_and_hint() -> Statement {
        Statement {
            date: Some(time::macros::date!(2026 - 01 - 08)),
            account_hint: Some("5437633269".to_owned()),
            ..statement()
        }
    }

    #[test]
    fn every_field_appears_in_every_format() {
        assert_every_field_appears_in_every_format(&account_row(), false);
        assert_every_field_appears_in_every_format(&account_row_without_iban(), true);
        assert_every_field_appears_in_every_format(&transaction_with_dates(), false);
        assert_every_field_appears_in_every_format(&transaction(), true);
        assert_every_field_appears_in_every_format(&category(), false);
        assert_every_field_appears_in_every_format(&security(), false);
        assert_every_field_appears_in_every_format(&statement_with_date_and_hint(), false);
        assert_every_field_appears_in_every_format(&statement(), true);
    }

    #[test]
    fn table_cells_show_the_selected_value_or_null() {
        let table = list(&[account_row()], OutputFormat::Table, Some("bic"));
        assert_eq!(cell(&table, 2), "b");

        let table = list(
            &[account_row_without_iban()],
            OutputFormat::Table,
            Some("iban"),
        );
        assert_eq!(cell(&table, 2), "null");

        let mut row = account_row();
        row.account.balance = vec![BalanceEntry {
            amount: Decimal::new(1234, 2),
            currency: "EUR".to_owned(),
        }];
        let table = list(&[row], OutputFormat::Table, Some("balance"));
        assert_eq!(cell(&table, 2), r#"[{"amount":"12.34","currency":"EUR"}]"#);
    }

    #[test]
    fn table_columns_follow_the_requested_order_once_each() {
        let table = list(
            &[account_row()],
            OutputFormat::Table,
            Some("name,bank,name"),
        );
        assert_eq!(table.lines().next(), Some("| name | bank |"));
    }

    #[test]
    fn selection_applies_to_an_empty_list() {
        let table = list::<Security>(&[], OutputFormat::Table, Some("isin,market"));
        assert_eq!(table.lines().next(), Some("| isin | market |"));
    }

    #[test]
    fn selection_fills_a_missing_key_with_null() {
        let json = detail(
            &account_row_without_iban(),
            OutputFormat::Json,
            Some("iban"),
        );
        let json: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(json, json!({"iban": null}));
    }

    #[test]
    fn output_without_selection_keeps_its_shape() {
        let items = [account_row_without_iban()];
        let table = list(&items, OutputFormat::Table, None);
        let header: Vec<&str> = cell(&table, 0).split('|').map(str::trim).collect();
        assert_eq!(header, AccountRow::headers());

        let json = list(&items, OutputFormat::Json, None);
        let json: Value = serde_json::from_str(&json).unwrap();
        assert!(!json["results"][0].as_object().unwrap().contains_key("iban"));

        let table = detail(&account_row(), OutputFormat::Table, None);
        assert!(table.contains("| BIC "), "{table}");
    }

    #[test]
    fn multi_word_fields_select_camel_case_keys() {
        assert_eq!(
            selected_keys(&transaction(), "bookingdate"),
            ["bookingDate"]
        );
        assert_eq!(
            selected_keys(&security(), "name,assetclassuuid"),
            ["assetClassUuid", "name"]
        );
    }

    #[test]
    fn aliased_fields_select_their_json_key() {
        assert_eq!(selected_keys(&account_row(), "account"), ["accountNumber"]);
        assert_eq!(selected_keys(&account_row(), "bic"), ["bankCode"]);
        assert_eq!(selected_keys(&transaction(), "account"), ["accountUuid"]);
        assert_eq!(
            selected_keys(&transaction(), "counterpartyaccount"),
            ["accountNumber"]
        );
        assert_eq!(
            selected_keys(&transaction(), "counterpartybankcode"),
            ["bankCode"]
        );
        assert_eq!(selected_keys(&statement(), "account"), ["account_hint"]);
    }
}
