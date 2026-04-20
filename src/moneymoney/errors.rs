//! Error types for MoneyMoney operations.

use thiserror::Error;

/// Errors returned from the MoneyMoney AppleScript surface and related
/// filesystem / decoding steps.
#[derive(Debug, Error)]
pub enum MoneyMoneyError {
    /// MoneyMoney is running but its database is locked. The user must unlock
    /// the app GUI before any `export ...` verb will succeed.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only constructed by macOS stderr classifier")
    )]
    #[error("MoneyMoney database is locked; unlock the app and try again")]
    DatabaseLocked,

    /// MoneyMoney is not currently running. AppleScript will auto-launch it in
    /// some cases, but the user may prefer an explicit hint.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only constructed by macOS stderr classifier")
    )]
    #[error("MoneyMoney is not running")]
    NotRunning,

    /// MoneyMoney is not installed on this machine.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "only constructed by macOS stderr classifier")
    )]
    #[error("MoneyMoney is not installed")]
    NotInstalled,

    /// `mm` was built for a non-macOS target. AppleScript is not available.
    #[cfg_attr(
        target_os = "macos",
        allow(dead_code, reason = "constructed only on non-macOS builds")
    )]
    #[error("MoneyMoney integration is only supported on macOS")]
    NotSupported,

    /// AppleScript execution failed. Carries the stderr output from
    /// `osascript` for diagnosis.
    #[error("AppleScript error: {0}")]
    ScriptError(String),

    /// Launching `osascript` failed (e.g., binary missing on a broken macOS
    /// install).
    #[error("failed to invoke osascript: {0}")]
    Spawn(#[from] std::io::Error),

    /// The plist returned by MoneyMoney could not be decoded.
    #[error("failed to decode plist: {0}")]
    PlistDecode(#[from] plist::Error),

    /// No account matches the given reference.
    #[error("no account matches '{0}'")]
    AccountNotFound(String),

    /// Multiple accounts match the given reference. Carries the candidates
    /// formatted as `Bank/Name` paths.
    #[error("ambiguous account reference '{input}'; candidates: {}", candidates.join(", "))]
    AmbiguousAccount {
        input: String,
        candidates: Vec<String>,
    },

    /// The input looked like an IBAN (two letters + two digits prefix) but
    /// failed mod-97 validation.
    #[error("invalid IBAN: {0}")]
    InvalidIban(String),

    /// The reference resolved to a bank / group parent row rather than a
    /// selectable account.
    #[error("'{0}' is a bank group, not a selectable account")]
    AccountIsGroup(String),

    /// Alias expansion encountered a cycle.
    #[error("alias cycle starting at '{0}'")]
    AliasCycle(String),

    /// `mm transaction add` targeted a non-offline account. MoneyMoney only
    /// allows manual booking on offline-managed accounts.
    #[error("'{0}' is not an offline account; `add transaction` would fail")]
    AccountNotOffline(String),

    /// User input contained characters that can't be safely embedded in an
    /// AppleScript double-quoted string (e.g. `"`, newlines). Rejected at
    /// input time rather than escaped.
    #[error("input field {field} contains forbidden character: {ch:?}")]
    InvalidScriptInput { field: &'static str, ch: char },
}

/// Convenience [`Result`] alias used throughout the [`moneymoney`](crate::moneymoney)
/// module.
pub type Result<T, E = MoneyMoneyError> = std::result::Result<T, E>;
