//! `mm categories` — list the category tree.

use crate::applescript::{OsascriptRunner, run_plist};
use crate::moneymoney::types::{Budget, Category};
use crate::output::{DetailView, FieldFilter, FieldNames, OutputFormat, Tabular, format_list};

const SCRIPT_EXPORT_CATEGORIES: &str = "tell application \"MoneyMoney\" to export categories";

/// Table cell for an optional budget: `"400 monthly, 123.45 available"`.
fn budget_cell(budget: Option<&Budget>) -> String {
    budget.map_or_else(String::new, |b| {
        format!("{} {}, {} available", b.amount, b.period, b.available)
    })
}

impl Tabular for Category {
    fn headers() -> &'static [&'static str] {
        &[
            "Name",
            "Group",
            "Indentation",
            "Default",
            "Currency",
            "Budget",
            "Rules",
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.group.to_string(),
            self.indentation.to_string(),
            self.default.to_string(),
            self.currency.clone(),
            budget_cell(self.budget.as_ref()),
            self.rules.clone(),
        ]
    }
}

impl DetailView for Category {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Name", self.name.clone()),
            ("UUID", self.uuid.clone()),
            ("Group", self.group.to_string()),
            ("Indentation", self.indentation.to_string()),
            ("Default", self.default.to_string()),
            ("Currency", self.currency.clone()),
            ("Budget", budget_cell(self.budget.as_ref())),
            ("Rules", self.rules.clone()),
        ]
    }
}

impl FieldNames for Category {
    fn valid_fields() -> &'static [&'static str] {
        &[
            "name",
            "uuid",
            "group",
            "indentation",
            "default",
            "currency",
            "budget",
            "rules",
        ]
    }
}

/// Options controlling `mm categories`.
pub struct ListOptions {
    pub format: Option<OutputFormat>,
    pub fields: Option<String>,
}

/// `mm categories` — emit the full category tree.
pub async fn run_list<R: OsascriptRunner>(runner: &R, opts: ListOptions) -> anyhow::Result<()> {
    let cats: Vec<Category> = run_plist(runner, SCRIPT_EXPORT_CATEGORIES).await?;
    let total = cats.len();
    let format = OutputFormat::resolve(opts.format);
    let filter = opts
        .fields
        .as_deref()
        .map(FieldFilter::parse::<Category>)
        .transpose()?;
    format_list(&cats, total, format, filter.as_ref())
}
