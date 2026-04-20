//! Tracing subscriber setup.
//!
//! All output goes to stderr regardless of mode. In CLI mode, stdout carries
//! command results (table/JSON/NDJSON); in MCP mode, stdout carries JSON-RPC
//! frames. Any diagnostic output on stdout would corrupt pipes or the MCP
//! protocol.

use tracing_subscriber::EnvFilter;

/// Runtime mode the process is operating in.
#[derive(Debug, Clone, Copy)]
pub enum Mode {
    /// Standard CLI invocation.
    Cli,
    /// MCP server over stdio. Logs use JSON format so MCP hosts that capture
    /// stderr get structured records.
    Mcp,
}

impl Mode {
    fn default_filter(self) -> &'static str {
        match self {
            Self::Cli => "warn",
            Self::Mcp => "info",
        }
    }
}

/// Initialize the global tracing subscriber.
///
/// Honors the `MM_LOG` environment variable (accepts any
/// [`EnvFilter`] expression, e.g., `mm=debug,rmcp=info`).
///
/// This is idempotent in the sense that calling it twice will fail the second
/// call silently; `mm` only calls it once, at startup in `main`.
pub fn init(mode: Mode) {
    let env =
        EnvFilter::try_from_env("MM_LOG").unwrap_or_else(|_| EnvFilter::new(mode.default_filter()));

    let builder = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(env);

    match mode {
        Mode::Cli => {
            let _ = builder.compact().try_init();
        }
        Mode::Mcp => {
            let _ = builder.json().try_init();
        }
    }
}
