//! `AppleScript` dispatcher.
//!
//! All interaction with MoneyMoney goes through [`OsascriptRunner::run`],
//! which spawns `osascript -e <script>` and returns the raw stdout bytes.
//! [`run_plist`] is the typed helper: it calls the runner and decodes the
//! bytes as a plist into any `Deserialize` type.
//!
//! Errors are classified from the `osascript` stderr output into
//! [`MoneyMoneyError`] variants so callers can react to lock state / missing
//! app / protocol errors without string-matching themselves.

use std::future::Future;

use serde::de::DeserializeOwned;
use tokio::process::Command;
use tracing::debug;

use crate::moneymoney::MoneyMoneyError;

#[cfg(test)]
pub mod mock;

/// Trait wrapping the one-shot `osascript -e <script>` invocation.
///
/// Split out so tests can substitute a mock runner that returns canned plist
/// bytes without spawning any subprocess.
pub trait OsascriptRunner: Send + Sync {
    /// Run a single AppleScript snippet and return its stdout bytes.
    fn run(&self, script: &str) -> impl Future<Output = Result<Vec<u8>, MoneyMoneyError>> + Send;
}

/// Default runner that shells out to `osascript` via `tokio::process`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TokioOsascriptRunner;

#[cfg(target_os = "macos")]
impl OsascriptRunner for TokioOsascriptRunner {
    async fn run(&self, script: &str) -> Result<Vec<u8>, MoneyMoneyError> {
        debug!(bytes = script.len(), "invoking osascript");
        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(classify_stderr(&stderr));
        }
        Ok(output.stdout)
    }
}

#[cfg(not(target_os = "macos"))]
impl OsascriptRunner for TokioOsascriptRunner {
    async fn run(&self, _script: &str) -> Result<Vec<u8>, MoneyMoneyError> {
        Err(MoneyMoneyError::NotSupported)
    }
}

/// Run a script and deserialize its stdout as a plist.
pub async fn run_plist<T, R>(runner: &R, script: &str) -> Result<T, MoneyMoneyError>
where
    T: DeserializeOwned,
    R: OsascriptRunner,
{
    let bytes = runner.run(script).await?;
    Ok(plist::from_bytes(&bytes)?)
}

/// Run a script and return its stdout as a trimmed UTF-8 string.
///
/// Use this for AppleScript calls that don't emit plist (e.g., returning a
/// bare version string or a boolean).
pub async fn run_text<R: OsascriptRunner>(
    runner: &R,
    script: &str,
) -> Result<String, MoneyMoneyError> {
    let bytes = runner.run(script).await?;
    let text = String::from_utf8_lossy(&bytes).trim().to_owned();
    Ok(text)
}

/// Map `osascript` stderr to a [`MoneyMoneyError`] variant.
///
/// Recognized patterns (case-sensitive):
///
/// | Pattern | Mapped to |
/// |---|---|
/// | `Locked database` / `(-2720)` | [`DatabaseLocked`](MoneyMoneyError::DatabaseLocked) |
/// | `isn't running` / `-600` | [`NotRunning`](MoneyMoneyError::NotRunning) |
/// | `Can't get application "MoneyMoney"` / `-10814` | [`NotInstalled`](MoneyMoneyError::NotInstalled) |
/// | anything else | [`ScriptError`](MoneyMoneyError::ScriptError) carrying the stderr text |
fn classify_stderr(stderr: &str) -> MoneyMoneyError {
    if stderr.contains("Locked database") || stderr.contains("-2720") {
        MoneyMoneyError::DatabaseLocked
    } else if stderr.contains("isn't running")
        || stderr.contains("is not running")
        || stderr.contains("(-600)")
    {
        MoneyMoneyError::NotRunning
    } else if stderr.contains("-10814") || stderr.contains("Can't get application") {
        MoneyMoneyError::NotInstalled
    } else {
        MoneyMoneyError::ScriptError(stderr.trim().to_owned())
    }
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests need to panic on unexpected variants")]
mod tests {
    use super::*;

    #[test]
    fn classify_locked_database() {
        let err = classify_stderr(
            "33:48: execution error: MoneyMoney got an error: Locked database. (-2720)",
        );
        assert!(matches!(err, MoneyMoneyError::DatabaseLocked));
    }

    #[test]
    fn classify_not_running() {
        let err = classify_stderr(
            "0:0: execution error: MoneyMoney got an error: Application isn't running. (-600)",
        );
        assert!(matches!(err, MoneyMoneyError::NotRunning));
    }

    #[test]
    fn classify_fallback() {
        let err = classify_stderr("some unexpected text");
        match err {
            MoneyMoneyError::ScriptError(s) => assert_eq!(s, "some unexpected text"),
            other => panic!("expected ScriptError, got {other:?}"),
        }
    }
}
