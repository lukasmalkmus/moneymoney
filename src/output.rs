//! Output formatting primitives: table / JSON / NDJSON, with optional field
//! filtering. Ported from the sister project `1nitetent` with tweaks for the
//! `mm` domain.

use std::io::{self, IsTerminal as _, Write as _};

use comfy_table::presets::ASCII_MARKDOWN;
use comfy_table::{ContentArrangement, Table};
use serde::Serialize;
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
}

/// A user-supplied `-F` filter.
#[derive(Debug, Clone)]
pub struct FieldFilter {
    fields: Vec<String>,
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
        let fields: Vec<String> = input.split(',').map(|s| s.trim().to_owned()).collect();
        let valid = T::valid_fields();
        for f in &fields {
            if !valid.contains(&f.as_str()) {
                return Err(FieldFilterError {
                    field: f.clone(),
                    valid: valid.iter().map(|&s| s.to_owned()).collect(),
                });
            }
        }
        Ok(Self { fields })
    }

    fn filter_json_value(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let filtered: serde_json::Map<String, serde_json::Value> = map
                    .iter()
                    .filter(|(k, _)| self.fields.contains(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                serde_json::Value::Object(filtered)
            }
            other => other.clone(),
        }
    }

    fn has_field(&self, name: &str) -> bool {
        self.fields.iter().any(|f| f == name)
    }
}

/// Render a list of items with the standard envelope.
pub fn format_list<T: Tabular + Serialize>(
    items: &[T],
    total_count: usize,
    format: OutputFormat,
    filter: Option<&FieldFilter>,
) -> anyhow::Result<()> {
    let showing = items.len();
    let has_more = showing < total_count;

    match format {
        OutputFormat::Json => {
            let json_items: Vec<serde_json::Value> = items
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
            let mut stdout = io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &envelope)?;
            writeln!(stdout)?;
        }
        OutputFormat::Ndjson => {
            let mut stdout = io::stdout().lock();
            let meta = serde_json::json!({
                "_meta": true,
                "total_count": total_count,
                "showing": showing,
                "has_more": has_more,
            });
            serde_json::to_writer(&mut stdout, &meta)?;
            writeln!(stdout)?;
            for item in items {
                let v = serde_json::to_value(item)?;
                let v = filter.map_or_else(|| v.clone(), |f| f.filter_json_value(&v));
                serde_json::to_writer(&mut stdout, &v)?;
                writeln!(stdout)?;
            }
        }
        OutputFormat::Table => render_table(items, filter, has_more, showing, total_count),
    }
    Ok(())
}

fn render_table<T: Tabular>(
    items: &[T],
    filter: Option<&FieldFilter>,
    has_more: bool,
    showing: usize,
    total_count: usize,
) {
    let headers = T::headers();
    let mut table = Table::new();
    table.load_preset(ASCII_MARKDOWN);
    table.set_content_arrangement(ContentArrangement::Dynamic);

    if let Some(f) = filter {
        let filtered_headers: Vec<&&str> = headers
            .iter()
            .filter(|h| f.has_field(&h.to_lowercase()))
            .collect();
        table.set_header(filtered_headers.iter().map(|h| **h));
        for item in items {
            let row = item.row();
            let filtered_row: Vec<&String> = row
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    headers
                        .get(*i)
                        .is_some_and(|h| f.has_field(&h.to_lowercase()))
                })
                .map(|(_, v)| v)
                .collect();
            table.add_row(filtered_row);
        }
    } else {
        table.set_header(headers);
        for item in items {
            table.add_row(item.row());
        }
    }

    println!("{table}");
    if has_more {
        println!("\nShowing {showing} of {total_count} results");
    }
}

/// Render a single detail view.
pub fn format_detail<T: DetailView + Serialize>(
    item: &T,
    format: OutputFormat,
    filter: Option<&FieldFilter>,
) -> anyhow::Result<()> {
    match format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            let v = serde_json::to_value(item)?;
            let v = filter.map_or_else(|| v.clone(), |f| f.filter_json_value(&v));
            let mut stdout = io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, &v)?;
            writeln!(stdout)?;
        }
        OutputFormat::Table => {
            let fields = item.fields();
            let mut table = Table::new();
            table.load_preset(ASCII_MARKDOWN);
            table.set_content_arrangement(ContentArrangement::Dynamic);
            table.set_header(["Field", "Value"]);
            for (name, value) in &fields {
                if filter.is_none_or(|f| f.has_field(&name.to_lowercase())) {
                    table.add_row([*name, value.as_str()]);
                }
            }
            println!("{table}");
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
