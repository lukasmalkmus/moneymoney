//! Optional TOML config loader.
//!
//! The config holds exactly one thing today: a map of user-defined account
//! aliases. `mm` has no credentials to store — MoneyMoney owns those in its
//! encrypted database.
//!
//! Lookup precedence (first existing path wins):
//!
//! 1. `$MM_CONFIG` (explicit override)
//! 2. `$XDG_CONFIG_HOME/mm/config.toml`
//! 3. `~/.config/mm/config.toml`
//! 4. `~/Library/Application Support/mm/config.toml` (macOS-native fallback)
//!
//! If no config file exists, [`load`] returns the default (empty) config.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Map from alias key (as typed by the user) to an account reference
    /// (UUID, IBAN, account number, `Bank/Name` path, or another alias).
    pub aliases: HashMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

/// Load the config from the first path that exists. Returns an empty
/// [`Config`] when no file is found.
pub fn load() -> Result<Config, ConfigError> {
    let Some(path) = locate() else {
        return Ok(Config::default());
    };
    let bytes = std::fs::read(&path).map_err(|source| ConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let raw = String::from_utf8_lossy(&bytes);
    toml::from_str::<Config>(&raw).map_err(|source| ConfigError::Parse { path, source })
}

fn locate() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("MM_CONFIG") {
        let p = PathBuf::from(explicit);
        if p.exists() {
            return Some(p);
        }
    }
    candidates().into_iter().find(|c| c.exists())
}

fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = dirs::config_dir() {
        out.push(p.join("mm").join("config.toml"));
    }
    // ~/.config/mm/config.toml even on macOS, for users following XDG.
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".config").join("mm").join("config.toml"));
        #[cfg(target_os = "macos")]
        out.push(
            home.join("Library")
                .join("Application Support")
                .join("mm")
                .join("config.toml"),
        );
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test fixture asserts on TOML parse"
)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases_table() {
        let toml = r#"
[aliases]
checking = "DE89370400440532013000"
depot = "Trade Republic/Wertpapierdepot"
pp = "PayPal"
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.aliases.len(), 3);
        assert_eq!(
            cfg.aliases.get("checking").map(String::as_str),
            Some("DE89370400440532013000")
        );
        assert_eq!(
            cfg.aliases.get("depot").map(String::as_str),
            Some("Trade Republic/Wertpapierdepot")
        );
    }

    #[test]
    fn defaults_to_empty() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.aliases.is_empty());
    }
}
