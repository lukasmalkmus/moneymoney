//! `MockOsascriptRunner` — records every script it's handed and returns a
//! canned stdout byte sequence. Used for golden-string tests of write
//! verbs without touching a live MoneyMoney install.

#![allow(
    dead_code,
    reason = "helper surface consumed lazily from per-command test modules"
)]

use std::sync::Mutex;

use super::OsascriptRunner;
use crate::moneymoney::MoneyMoneyError;

#[derive(Default)]
pub struct MockOsascriptRunner {
    pub scripts: Mutex<Vec<String>>,
    pub response: Mutex<Vec<u8>>,
}

impl MockOsascriptRunner {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_response(bytes: &[u8]) -> Self {
        Self {
            scripts: Mutex::new(Vec::new()),
            response: Mutex::new(bytes.to_vec()),
        }
    }

    pub fn last_script(&self) -> Option<String> {
        self.scripts.lock().ok()?.last().cloned()
    }
}

impl OsascriptRunner for MockOsascriptRunner {
    async fn run(&self, script: &str) -> Result<Vec<u8>, MoneyMoneyError> {
        if let Ok(mut s) = self.scripts.lock() {
            s.push(script.to_owned());
        }
        Ok(self.response.lock().map(|r| r.clone()).unwrap_or_default())
    }
}
