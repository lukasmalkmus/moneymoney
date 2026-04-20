//! `mm portfolio` — list securities held in a portfolio account.

use std::collections::HashMap;

use crate::applescript::{OsascriptRunner, run_plist};
use crate::commands::accounts::{annotate_with_bank, fetch_all};
use crate::moneymoney::resolver::Resolver;
use crate::moneymoney::types::{PortfolioEnvelope, Security};
use crate::output::{DetailView, FieldFilter, FieldNames, OutputFormat, Tabular, format_list};

/// Options controlling `mm portfolio`.
pub struct GetOptions {
    pub reference: String,
    pub format: Option<OutputFormat>,
    pub fields: Option<String>,
    pub aliases: HashMap<String, String>,
}

impl Tabular for Security {
    fn headers() -> &'static [&'static str] {
        &["Name", "ISIN", "Quantity", "Price", "Amount", "Currency"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.name.clone(),
            self.isin.clone(),
            self.quantity.to_string(),
            self.price.to_string(),
            self.amount.to_string(),
            self.currency_of_amount.clone(),
        ]
    }
}

impl DetailView for Security {
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Name", self.name.clone()),
            ("ISIN", self.isin.clone()),
            ("SecurityNumber", self.security_number.clone()),
            ("Type", self.security_type.clone()),
            ("Quantity", self.quantity.to_string()),
            ("Price", self.price.to_string()),
            ("CurrencyOfPrice", self.currency_of_price.clone()),
            ("Amount", self.amount.to_string()),
            ("CurrencyOfAmount", self.currency_of_amount.clone()),
            ("PurchasePrice", self.purchase_price.to_string()),
            (
                "CurrencyOfPurchasePrice",
                self.currency_of_purchase_price.clone(),
            ),
            ("AbsoluteProfit", self.absolute_profit.to_string()),
            ("RelativeProfit", self.relative_profit.to_string()),
            ("Market", self.market.clone()),
            ("AccountUUID", self.account_uuid.clone()),
            ("AssetClassUUID", self.asset_class_uuid.clone()),
        ]
    }
}

impl FieldNames for Security {
    fn valid_fields() -> &'static [&'static str] {
        &[
            "name",
            "isin",
            "securitynumber",
            "type",
            "quantity",
            "price",
            "currencyofprice",
            "amount",
            "currencyofamount",
            "purchaseprice",
            "currencyofpurchaseprice",
            "absoluteprofit",
            "relativeprofit",
            "market",
            "accountuuid",
            "assetclassuuid",
        ]
    }
}

/// `mm portfolio --account REF` entrypoint.
pub async fn run<R: OsascriptRunner>(runner: &R, opts: GetOptions) -> anyhow::Result<()> {
    let raw = fetch_all(runner).await?;
    let rows = annotate_with_bank(raw);
    let resolver = Resolver::new(rows, opts.aliases);
    let account_row = resolver.resolve(&opts.reference)?;

    let script = format!(
        "tell application \"MoneyMoney\" to export portfolio from account \"{}\" as \"plist\"",
        account_row.account.account_number
    );
    let envelope: PortfolioEnvelope = run_plist(runner, &script).await?;

    let total = envelope.portfolio.len();
    let format = OutputFormat::resolve(opts.format);
    let filter = opts
        .fields
        .as_deref()
        .map(FieldFilter::parse::<Security>)
        .transpose()?;
    format_list(&envelope.portfolio, total, format, filter.as_ref())
}
