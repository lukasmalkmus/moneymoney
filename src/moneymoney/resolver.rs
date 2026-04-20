//! Account reference resolution.
//!
//! Accepts user-supplied references in any of six forms (UUID, IBAN, account
//! number, alias, `Bank/Name` path, bare name) and returns the matching
//! account. Handles ambiguity by returning a structured error with the full
//! candidate list.
//!
//! Names are NOT unique across banks ("Girokonto" appears under both ING and
//! Trade Republic in the author's data). The resolver is built to surface
//! that rather than silently pick one.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;

// The package is `iban_validate` on crates.io but the library name is `iban`.
use iban::{BaseIban, Iban, IbanLike};

use crate::commands::accounts::AccountRow;
use crate::moneymoney::MoneyMoneyError;

/// Looks up accounts from an annotated [`AccountRow`] list using the
/// author-controlled aliases layered on top.
///
/// Only leaf (non-group) rows are considered selectable.
pub struct Resolver {
    rows: Vec<AccountRow>,
    aliases: HashMap<String, String>,
}

impl Resolver {
    /// Build a resolver from a freshly fetched account list plus an alias
    /// map loaded from the config file.
    #[must_use]
    pub fn new(rows: Vec<AccountRow>, aliases: HashMap<String, String>) -> Self {
        Self { rows, aliases }
    }

    /// The full, annotated account list (including group parents) the
    /// resolver was constructed with.
    #[allow(dead_code, reason = "consumed by MCP tools / future commands")]
    #[must_use]
    pub fn rows(&self) -> &[AccountRow] {
        &self.rows
    }

    /// Resolve a user-supplied reference to a single account row.
    ///
    /// Resolution priority (first match wins):
    ///
    /// 1. UUID (exact match on `Account::uuid`).
    /// 2. IBAN (structural parse, then mod-97 validated by `iban_validate`).
    /// 3. Account number (exact, case-sensitive).
    /// 4. Alias (config file key); value is re-resolved recursively.
    /// 5. `Bank/Name` path (case-sensitive, split on `/`).
    /// 6. Bare name (case-sensitive; errors on ambiguity).
    pub fn resolve(&self, input: &str) -> Result<&AccountRow, MoneyMoneyError> {
        let mut visited = HashSet::new();
        self.resolve_inner(input, &mut visited)
    }

    fn resolve_inner<'a>(
        &'a self,
        input: &str,
        visited: &mut HashSet<String>,
    ) -> Result<&'a AccountRow, MoneyMoneyError> {
        // 1) UUID exact match.
        if let Some(row) = self.find_leaf(|r| r.account.uuid == input) {
            return Ok(row);
        }

        // 2) IBAN. `BaseIban::from_str` accepts anything that parses
        // structurally AND passes the mod-97 checksum; `Iban::from_str` adds
        // country-specific length and format rules. We only need structural
        // correctness (MoneyMoney already stores valid IBANs), but if the
        // input parses as BaseIban its checksum is already confirmed.
        //
        // An input that only "looks like" an IBAN (starts with two letters,
        // two digits) but fails the checksum is worth flagging explicitly
        // rather than falling through to AccountNotFound.
        if looks_like_iban(input) {
            match BaseIban::from_str(input) {
                Ok(base) => {
                    let normalized = base.electronic_str().to_owned();
                    if let Some(row) = self.find_leaf(|r| r.account.account_number == normalized) {
                        return Ok(row);
                    }
                    return Err(MoneyMoneyError::AccountNotFound(input.to_owned()));
                }
                Err(_) => {
                    // One more attempt with `Iban` (country-aware) just in
                    // case we were too strict; otherwise surface as invalid.
                    if Iban::from_str(input).is_err() {
                        return Err(MoneyMoneyError::InvalidIban(input.to_owned()));
                    }
                }
            }
        }

        // 3) Plain account number exact match (PayPal emails, legacy numbers).
        if let Some(row) = self.find_leaf(|r| r.account.account_number == input) {
            return Ok(row);
        }

        // 4) Alias. Re-resolve the value to allow chaining.
        if let Some(value) = self.aliases.get(input) {
            if !visited.insert(input.to_owned()) {
                return Err(MoneyMoneyError::AliasCycle(input.to_owned()));
            }
            return self.resolve_inner(value, visited);
        }

        // 5) Bank/Name path.
        if let Some((bank, name)) = input.split_once('/') {
            if let Some(row) = self.find_leaf(|r| r.bank == bank && r.account.name == name) {
                return Ok(row);
            }
            return Err(MoneyMoneyError::AccountNotFound(input.to_owned()));
        }

        // 6) Bare name — may be ambiguous across banks.
        let matches: Vec<&AccountRow> = self
            .rows
            .iter()
            .filter(|r| !r.account.group && r.account.name == input)
            .collect();
        match matches.as_slice() {
            [] => {
                if self
                    .rows
                    .iter()
                    .any(|r| r.account.group && r.account.name == input)
                {
                    Err(MoneyMoneyError::AccountIsGroup(input.to_owned()))
                } else {
                    Err(MoneyMoneyError::AccountNotFound(input.to_owned()))
                }
            }
            [single] => Ok(*single),
            many => Err(MoneyMoneyError::AmbiguousAccount {
                input: input.to_owned(),
                candidates: many
                    .iter()
                    .map(|r| format!("{}/{}", r.bank, r.account.name))
                    .collect(),
            }),
        }
    }

    fn find_leaf<F: Fn(&AccountRow) -> bool>(&self, pred: F) -> Option<&AccountRow> {
        self.rows.iter().find(|r| !r.account.group && pred(r))
    }
}

/// Cheap shape check: "two letters, two digits, then some alphanumerics,
/// ignoring whitespace." Used to gate the full IBAN parse so arbitrary
/// strings (PayPal emails, UUIDs) don't get flagged as invalid IBANs.
fn looks_like_iban(input: &str) -> bool {
    let cleaned: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() < 15 || cleaned.len() > 34 {
        return false;
    }
    let bytes = cleaned.as_bytes();
    bytes[0].is_ascii_alphabetic()
        && bytes[1].is_ascii_alphabetic()
        && bytes[2].is_ascii_digit()
        && bytes[3].is_ascii_digit()
        && bytes[4..].iter().all(u8::is_ascii_alphanumeric)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests construct well-known fixture rows and assert on variants"
)]
mod tests {
    use std::str::FromStr;

    use rust_decimal::Decimal;

    use super::*;
    use crate::moneymoney::types::{Account, BalanceEntry};

    fn row(bank: &str, name: &str, uuid: &str, account_number: &str, group: bool) -> AccountRow {
        AccountRow {
            bank: bank.to_owned(),
            account: Account {
                uuid: uuid.to_owned(),
                name: name.to_owned(),
                owner: String::new(),
                account_number: account_number.to_owned(),
                bank_code: String::new(),
                currency: "EUR".to_owned(),
                group,
                portfolio: false,
                indentation: u32::from(!group),
                account_type: "Giro account".to_owned(),
                balance: vec![BalanceEntry {
                    amount: Decimal::from_str("0").unwrap(),
                    currency: "EUR".to_owned(),
                }],
            },
        }
    }

    fn fixture() -> Resolver {
        // Mirrors the author's real sidebar: two "Girokonto" accounts under
        // different banks, plus a standalone PayPal.
        let rows = vec![
            row("ING", "ING", "ing-uuid", "", true),
            row(
                "ING",
                "Girokonto",
                "ing-giro",
                "DE89370400440532013000",
                false,
            ),
            row(
                "ING",
                "Extra-Konto",
                "ing-extra",
                "DE48500105175807974241",
                false,
            ),
            row("Trade Republic", "Trade Republic", "tr-uuid", "", true),
            row(
                "Trade Republic",
                "Girokonto",
                "tr-giro",
                "DE62100123450677821901",
                false,
            ),
            row("PayPal", "PayPal", "pp-uuid", "mail@example.com", false),
        ];
        let aliases = HashMap::from([
            ("checking".to_owned(), "ING/Girokonto".to_owned()),
            ("pp".to_owned(), "PayPal".to_owned()),
            ("a".to_owned(), "b".to_owned()),
            ("b".to_owned(), "a".to_owned()),
        ]);
        Resolver::new(rows, aliases)
    }

    #[test]
    fn resolves_by_uuid() {
        assert_eq!(
            fixture().resolve("ing-giro").unwrap().account.uuid,
            "ing-giro"
        );
    }

    #[test]
    fn resolves_by_iban() {
        let r = fixture();
        assert_eq!(
            r.resolve("DE89370400440532013000").unwrap().account.uuid,
            "ing-giro"
        );
        // Whitespace tolerated by iban_validate's parser.
        assert_eq!(
            r.resolve("DE89 3704 0044 0532 0130 00")
                .unwrap()
                .account
                .uuid,
            "ing-giro"
        );
    }

    #[test]
    fn rejects_invalid_iban_checksum() {
        match fixture().resolve("DE00370400440532013000") {
            Err(MoneyMoneyError::InvalidIban(_)) => {}
            other => panic!("expected InvalidIban, got {other:?}"),
        }
    }

    #[test]
    fn resolves_by_account_number() {
        assert_eq!(
            fixture().resolve("mail@example.com").unwrap().account.uuid,
            "pp-uuid"
        );
    }

    #[test]
    fn resolves_by_alias() {
        let r = fixture();
        assert_eq!(r.resolve("checking").unwrap().account.uuid, "ing-giro");
        assert_eq!(r.resolve("pp").unwrap().account.uuid, "pp-uuid");
    }

    #[test]
    fn detects_alias_cycle() {
        match fixture().resolve("a") {
            Err(MoneyMoneyError::AliasCycle(_)) => {}
            other => panic!("expected AliasCycle, got {other:?}"),
        }
    }

    #[test]
    fn resolves_by_bank_path() {
        let r = fixture();
        assert_eq!(r.resolve("ING/Girokonto").unwrap().account.uuid, "ing-giro");
        assert_eq!(
            r.resolve("Trade Republic/Girokonto").unwrap().account.uuid,
            "tr-giro"
        );
    }

    #[test]
    fn resolves_unambiguous_bare_name() {
        let r = fixture();
        assert_eq!(r.resolve("Extra-Konto").unwrap().account.uuid, "ing-extra");
        assert_eq!(r.resolve("PayPal").unwrap().account.uuid, "pp-uuid");
    }

    #[test]
    fn ambiguous_name_lists_candidates() {
        match fixture().resolve("Girokonto") {
            Err(MoneyMoneyError::AmbiguousAccount { input, candidates }) => {
                assert_eq!(input, "Girokonto");
                assert!(candidates.contains(&"ING/Girokonto".to_owned()));
                assert!(candidates.contains(&"Trade Republic/Girokonto".to_owned()));
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected AmbiguousAccount, got {other:?}"),
        }
    }

    #[test]
    fn group_name_yields_group_error() {
        match fixture().resolve("ING") {
            Err(MoneyMoneyError::AccountIsGroup(_)) => {}
            other => panic!("expected AccountIsGroup, got {other:?}"),
        }
    }

    #[test]
    fn unknown_name_yields_not_found() {
        match fixture().resolve("Nope") {
            Err(MoneyMoneyError::AccountNotFound(_)) => {}
            other => panic!("expected AccountNotFound, got {other:?}"),
        }
    }
}
