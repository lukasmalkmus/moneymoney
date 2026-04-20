//! `mm status` — report whether MoneyMoney is running and unlocked.
//!
//! Always exits with `0` once it produces a report; the report itself conveys
//! the state. Scripts and MCP clients can inspect the JSON payload to decide
//! whether further calls will succeed.

use serde::Serialize;

use crate::applescript::{OsascriptRunner, run_text};
use crate::moneymoney::MoneyMoneyError;

const SCRIPT_IS_RUNNING: &str =
    r#"tell application "System Events" to (name of processes) contains "MoneyMoney""#;
const SCRIPT_VERSION: &str = r#"tell application "MoneyMoney" to get version"#;
const SCRIPT_UNLOCK_PROBE: &str = r#"tell application "MoneyMoney" to export categories"#;

/// Observable state of the MoneyMoney app from the outside.
#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    /// True if the MoneyMoney process is currently running.
    pub running: bool,
    /// True if the database is unlocked. Undefined (always `false`) when
    /// `running` is `false`.
    pub unlocked: bool,
    /// Application version string, as reported by MoneyMoney. `None` when the
    /// app isn't running or the version couldn't be retrieved.
    pub version: Option<String>,
}

/// Query MoneyMoney's current state.
pub async fn check<R: OsascriptRunner>(runner: &R) -> Result<StatusReport, MoneyMoneyError> {
    let running = is_running(runner).await?;
    if !running {
        return Ok(StatusReport {
            running: false,
            unlocked: false,
            version: None,
        });
    }

    let version = run_text(runner, SCRIPT_VERSION).await.ok();

    // A cheap unlocked probe: `export categories` returns quickly on an
    // unlocked DB and fails with `DatabaseLocked` when locked.
    let unlocked = match runner.run(SCRIPT_UNLOCK_PROBE).await {
        Ok(_) => true,
        Err(MoneyMoneyError::DatabaseLocked) => false,
        Err(other) => return Err(other),
    };

    Ok(StatusReport {
        running: true,
        unlocked,
        version,
    })
}

async fn is_running<R: OsascriptRunner>(runner: &R) -> Result<bool, MoneyMoneyError> {
    let text = run_text(runner, SCRIPT_IS_RUNNING).await?;
    Ok(text == "true")
}

/// `mm status` CLI entrypoint. Prints the report as JSON to stdout.
pub async fn run<R: OsascriptRunner>(runner: &R) -> anyhow::Result<()> {
    let report = check(runner).await?;
    let json = serde_json::to_string_pretty(&report)?;
    println!("{json}");
    Ok(())
}
