//! Domain types decoded from MoneyMoney's plist output.
//!
//! Field names mirror the plist keys exactly (see
//! `osascript -e 'tell application "MoneyMoney" to export accounts'` on a
//! real install). Unknown fields are tolerated by the `plist` crate.

use std::str::FromStr;
use std::time::SystemTime;

use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A single account row in MoneyMoney.
///
/// MoneyMoney returns a flat list of accounts plus group-parent rows (with
/// `group = true`). Parent/child relationships are encoded via `indentation`
/// and document order: an account with `indentation = n` is a child of the
/// most recent preceding account with `indentation < n`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Stable unique identifier. This is the only field guaranteed to be
    /// unique across the account list.
    pub uuid: String,

    /// Display name ("Girokonto", "ING", "PayPal"). Not unique across groups.
    pub name: String,

    /// Account owner (person's name). May be empty.
    pub owner: String,

    /// For bank accounts this holds the IBAN. For others (PayPal, offline
    /// accounts) it can hold an email address, a free-form identifier, or be
    /// empty for group-parent rows.
    pub account_number: String,

    /// National bank code. Empty for group parents and for non-SEPA accounts.
    pub bank_code: String,

    /// Account's default currency (ISO 4217, e.g., "EUR", "USD").
    pub currency: String,

    /// True if this row is a bank/group parent rather than a selectable
    /// account.
    pub group: bool,

    /// True if the account holds securities (Depot / portfolio).
    pub portfolio: bool,

    /// Nesting depth in the sidebar. 0 = top-level, 1+ = child.
    pub indentation: u32,

    /// Human-readable account-type string ("Account group", "Girokonto",
    /// "Wertpapierdepot", etc.) as displayed by MoneyMoney.
    #[serde(rename = "type")]
    pub account_type: String,

    /// One entry per currency held. Each entry is a `[amount, currency]`
    /// tuple — most accounts have a single entry matching
    /// [`currency`](Self::currency), but multi-currency accounts hold
    /// balances in several currencies simultaneously.
    #[serde(default)]
    pub balance: Vec<BalanceEntry>,
}

/// A single-currency balance entry from [`Account::balance`].
#[derive(Debug, Clone, Serialize)]
pub struct BalanceEntry {
    /// Amount in the entry's currency.
    pub amount: Decimal,
    /// ISO 4217 currency code for this entry.
    pub currency: String,
}

// Plist serializes balances as `<array><real/><string/></array>`. Deserialize
// via a 2-tuple, converting the `f64` to `Decimal` via its string
// representation to absorb plist float noise (e.g., `17227.630000000001`).
impl<'de> Deserialize<'de> for BalanceEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (raw_amount, currency) = <(f64, String)>::deserialize(deserializer)?;
        Ok(Self {
            amount: f64_to_decimal(raw_amount),
            currency,
        })
    }
}

/// Convert a plist `<real>` float into a [`Decimal`] without surfacing
/// float-representation noise.
///
/// Formats the float with enough decimal places to absorb IEEE-754 artefacts
/// (`17227.630000000001` → `"17227.6300000000"` → `17227.63`) and then
/// rounds back to four decimal places, which is precise enough for every
/// fiat currency MoneyMoney supports and safe for the most common crypto
/// pairs.
pub fn f64_to_decimal(value: f64) -> Decimal {
    let formatted = format!("{value:.10}");
    Decimal::from_str(&formatted)
        .unwrap_or_default()
        .round_dp(4)
}

fn deserialize_amount<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Decimal, D::Error> {
    let raw = f64::deserialize(deserializer)?;
    Ok(f64_to_decimal(raw))
}

#[allow(dead_code, reason = "reserved for budget / optional amount fields")]
fn deserialize_amount_opt<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Decimal>, D::Error> {
    let raw = Option::<f64>::deserialize(deserializer)?;
    Ok(raw.map(f64_to_decimal))
}

/// Serde helper: serialize `plist::Date` as RFC3339 UTC.
///
/// Signature is dictated by `serde`'s `serialize_with` attribute, which
/// passes a reference to the field. Hence the `&Option<…>` that clippy
/// would otherwise flag.
#[allow(clippy::ref_option, reason = "serde serialize_with signature")]
fn serialize_plist_date_opt<S: Serializer>(
    date: &Option<plist::Date>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match date {
        Some(d) => {
            let sys: SystemTime = (*d).into();
            let odt: OffsetDateTime = sys.into();
            s.serialize_str(&odt.format(&Rfc3339).map_err(serde::ser::Error::custom)?)
        }
        None => s.serialize_none(),
    }
}

/// Envelope returned by `export transactions ... as "plist"`. MoneyMoney
/// wraps the transaction list in `{creator, transactions: [...]}`.
#[derive(Debug, Clone, Deserialize)]
pub struct TransactionsEnvelope {
    /// MoneyMoney version string that produced the export.
    #[allow(
        dead_code,
        reason = "captured for completeness; not surfaced by callers yet"
    )]
    #[serde(default)]
    pub creator: String,
    /// Actual transaction records.
    #[serde(default)]
    pub transactions: Vec<Transaction>,
}

/// A transaction record as exported by MoneyMoney's AppleScript surface.
///
/// `#[serde(default)]` on every field means MoneyMoney may omit any key
/// (the plist crate would otherwise fail deserialization).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    /// Numeric transaction id (stable per-account). Use this with
    /// `mm transaction set` or the MCP `set_transaction` tool.
    #[serde(default)]
    pub id: i64,

    /// UUID of the account this transaction belongs to.
    #[serde(default)]
    pub account_uuid: String,

    /// Payee or payer name as stored by MoneyMoney.
    #[serde(default)]
    pub name: String,

    /// Counter-party account number (IBAN for SEPA transactions).
    #[serde(default)]
    pub account_number: String,

    /// Counter-party bank code (BIC for SEPA transactions).
    #[serde(default)]
    pub bank_code: String,

    /// Signed amount. Negative = outgoing.
    #[serde(deserialize_with = "deserialize_amount", default)]
    pub amount: Decimal,

    /// Transaction currency (ISO 4217).
    #[serde(default)]
    pub currency: String,

    /// Date the bank booked the transaction.
    #[serde(serialize_with = "serialize_plist_date_opt", default)]
    pub booking_date: Option<plist::Date>,

    /// Value date (interest-bearing date).
    #[serde(serialize_with = "serialize_plist_date_opt", default)]
    pub value_date: Option<plist::Date>,

    /// Booking type as shown in the bank statement ("Überweisung",
    /// "Lastschrift", ...).
    #[serde(default)]
    pub booking_text: String,

    /// SEPA reference / purpose line.
    #[serde(default)]
    pub purpose: String,

    /// UUID of the assigned category. Resolve against
    /// [`Category::uuid`] from `export categories` to get the name.
    #[serde(default)]
    pub category_uuid: String,

    /// User-editable comment.
    #[serde(default)]
    pub comment: String,

    /// User's checkmark state.
    #[serde(default)]
    pub checkmark: bool,

    /// True for finalized entries, false for pending ones.
    #[serde(default)]
    pub booked: bool,
}

/// A category record. Categories form a tree through `indentation` and
/// document order, same as accounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    /// Stable unique identifier.
    #[serde(default)]
    pub uuid: String,

    /// Display name.
    pub name: String,

    /// True for intermediate nodes in the category tree.
    #[serde(default)]
    pub group: bool,

    /// Nesting depth. 0 = top-level.
    #[serde(default)]
    pub indentation: u32,

    /// True for the uncategorized default bucket.
    #[serde(default)]
    pub default: bool,

    /// Currency used when the category has a budget.
    #[serde(default)]
    pub currency: String,
}

/// Envelope returned by `export portfolio ... as "plist"`.
#[derive(Debug, Clone, Deserialize)]
pub struct PortfolioEnvelope {
    #[allow(dead_code, reason = "captured for completeness")]
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub portfolio: Vec<Security>,
}

/// A security held in a portfolio account (`portfolio = true`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Security {
    /// Security name.
    #[serde(default)]
    pub name: String,

    /// International Securities Identification Number.
    #[serde(default)]
    pub isin: String,

    /// Bank-specific security number / WKN.
    #[serde(default)]
    pub security_number: String,

    /// Security-specific id assigned by MoneyMoney.
    #[serde(default)]
    pub id: i64,

    /// Units held.
    #[serde(default, deserialize_with = "deserialize_amount")]
    pub quantity: Decimal,

    /// Currency for the quantity (typically empty for unit-based holdings).
    #[serde(default)]
    pub currency_of_quantity: String,

    /// Current market value in the account's currency.
    #[serde(default, deserialize_with = "deserialize_amount")]
    pub amount: Decimal,

    /// Currency of [`Self::amount`].
    #[serde(default)]
    pub currency_of_amount: String,

    /// Most recent price per unit.
    #[serde(default, deserialize_with = "deserialize_amount")]
    pub price: Decimal,

    /// Currency the price is quoted in.
    #[serde(default)]
    pub currency_of_price: String,

    /// Purchase price per unit, if known.
    #[serde(default, deserialize_with = "deserialize_amount")]
    pub purchase_price: Decimal,

    /// Currency of [`Self::purchase_price`].
    #[serde(default)]
    pub currency_of_purchase_price: String,

    /// Absolute profit since purchase, in `currencyOfProfit`.
    #[serde(default, deserialize_with = "deserialize_amount")]
    pub absolute_profit: Decimal,

    /// Currency of [`Self::absolute_profit`].
    #[serde(default)]
    pub currency_of_profit: String,

    /// Relative profit since purchase (as a decimal fraction, e.g., 0.05 = 5%).
    #[serde(default, deserialize_with = "deserialize_amount")]
    pub relative_profit: Decimal,

    /// Trading venue / market.
    #[serde(default)]
    pub market: String,

    /// Security type (e.g., "ETF", "Aktie").
    #[serde(rename = "type", default)]
    pub security_type: String,

    /// UUID of the MoneyMoney asset class, if assigned.
    #[serde(default)]
    pub asset_class_uuid: String,

    /// UUID of the owning portfolio account.
    #[serde(default)]
    pub account_uuid: String,

    /// Timestamp of the price used.
    #[serde(serialize_with = "serialize_plist_date_opt", default)]
    pub trade_timestamp: Option<plist::Date>,
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::excessive_precision,
    clippy::unreadable_literal,
    reason = "tests assert on precise constants"
)]
mod tests {
    use super::*;

    #[test]
    fn decimal_absorbs_float_noise() {
        assert_eq!(
            f64_to_decimal(17227.630000000001),
            Decimal::from_str("17227.63").unwrap()
        );
        assert_eq!(
            f64_to_decimal(9260.91),
            Decimal::from_str("9260.91").unwrap()
        );
        assert_eq!(f64_to_decimal(-12.34), Decimal::from_str("-12.34").unwrap());
    }

    #[test]
    fn balance_entry_from_plist_array() {
        // Synthesize a minimal plist: <array><real>12.34</real><string>EUR</string></array>
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array><real>12.34</real><string>EUR</string></array>
</plist>"#;
        let entry: BalanceEntry = plist::from_bytes(xml.as_bytes()).unwrap();
        assert_eq!(entry.amount, Decimal::from_str("12.34").unwrap());
        assert_eq!(entry.currency, "EUR");
    }
}
