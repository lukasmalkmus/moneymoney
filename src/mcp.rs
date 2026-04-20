//! MCP server — stdio tools exposing the same surface as the `mm` CLI.
//!
//! Read tools are marked `readOnlyHint: true`. Write tools
//! (`create_transfer`, `create_direct_debit`, `create_batch_transfer`,
//! `add_transaction`, `set_transaction`) are NOT read-only; `set_transaction`
//! is additionally marked `destructiveHint: true` because it silently
//! overwrites metadata. Transfer verbs are safe by construction — the user
//! confirms them in the MoneyMoney GUI and enters a TAN.

use std::collections::HashMap;
use std::sync::Arc;

use rmcp::ServerHandler;
use rmcp::ServiceExt;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ProtocolVersion, ServerCapabilities, ServerInfo};
use rmcp::transport::stdio;
use rmcp::{tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::applescript::{OsascriptRunner, TokioOsascriptRunner};
use crate::commands::accounts::{annotate_with_bank, fetch_all};
use crate::commands::status;
use crate::commands::transaction_edit::{AddOptions as TxAddOptions, SetOptions as TxSetOptions};
use crate::commands::transactions::build_export_script;
use crate::commands::transfer::{
    BatchTransferOptions, CreateDirectDebitOptions, CreateTransferOptions,
};
use crate::moneymoney::resolver::Resolver;
use crate::moneymoney::types::{Category, Transaction};
use crate::statements;

/// Parameters for the `get_account` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetAccountArgs {
    /// Account reference (UUID, IBAN, account number, alias, `Bank/Name`, or
    /// unambiguous name).
    pub account: String,
}

/// Parameters for the `list_transactions` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListTransactionsArgs {
    /// Account reference.
    pub account: String,
    /// Inclusive start date (YYYY-MM-DD). Defaults to 90 days ago.
    #[serde(default)]
    pub from: Option<String>,
    /// Inclusive end date (YYYY-MM-DD). Defaults to today.
    #[serde(default)]
    pub to: Option<String>,
    /// Case-insensitive substring filter against name/purpose/category/comment.
    #[serde(default)]
    pub search: Option<String>,
    /// Cap on results returned.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Parameters for the `get_portfolio` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetPortfolioArgs {
    /// Portfolio account reference.
    pub account: String,
}

/// Parameters for the `list_statements` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ListStatementsArgs {
    /// Optional account filter (IBAN, account number, or digit suffix).
    #[serde(default)]
    pub account: Option<String>,
    /// Earliest statement date (YYYY-MM-DD).
    #[serde(default)]
    pub since: Option<String>,
}

/// Parameters for the `get_statement` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetStatementArgs {
    /// Exact filename as returned by `list_statements`.
    pub filename: String,
}

/// Parameters for the `create_transfer` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateTransferArgs {
    /// Source account reference (UUID, IBAN, account number, alias,
    /// `Bank/Name`, or unambiguous name).
    pub from: String,
    /// Recipient IBAN.
    pub to_iban: String,
    /// Recipient name displayed in the payment window.
    #[serde(default)]
    pub to_name: Option<String>,
    /// Amount as a dot-decimal string (e.g. `"12.34"`).
    pub amount: String,
    /// Purpose / remittance text (SEPA reference line).
    #[serde(default)]
    pub purpose: Option<String>,
    /// SEPA end-to-end reference.
    #[serde(default)]
    pub endtoend_reference: Option<String>,
    /// Scheduled execution date `YYYY-MM-DD`.
    #[serde(default)]
    pub scheduled_date: Option<String>,
    /// If `true`, the payment lands silently in the outbox; user still has
    /// to release and enter a TAN via the GUI. If `false` (default),
    /// MoneyMoney opens a pre-filled payment window.
    #[serde(default)]
    pub into_outbox: bool,
}

/// Parameters for the `create_direct_debit` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateDirectDebitArgs {
    pub from: String,
    pub debtor_iban: String,
    #[serde(default)]
    pub debtor_name: Option<String>,
    pub amount: String,
    #[serde(default)]
    pub purpose: Option<String>,
    pub mandate_reference: String,
    #[serde(default)]
    pub mandate_date: Option<String>,
    #[serde(default)]
    pub scheduled_date: Option<String>,
    #[serde(default)]
    pub into_outbox: bool,
}

/// Parameters for the `create_batch_transfer` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BatchTransferArgs {
    /// Absolute path to the SEPA XML file (must live inside MoneyMoney's
    /// sandbox).
    pub file: String,
    /// Treat the batch as direct debits rather than transfers.
    #[serde(default)]
    pub direct_debit: bool,
}

/// Parameters for the `add_transaction` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct AddTransactionArgs {
    /// Offline account reference.
    pub account: String,
    /// Booking date `YYYY-MM-DD`.
    pub date: String,
    /// Debitor or creditor name.
    pub name: String,
    /// Signed amount as a dot-decimal string.
    pub amount: String,
    #[serde(default)]
    pub purpose: Option<String>,
    /// Category path; use `\` to separate nested names
    /// (e.g. `"Food\\Coffee"`).
    #[serde(default)]
    pub category: Option<String>,
}

/// Parameters for the `set_transaction` tool.
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetTransactionArgs {
    /// Transaction id (integer), obtained via `list_transactions`.
    pub id: i64,
    /// Toggle the user checkmark.
    #[serde(default)]
    pub checkmark: Option<bool>,
    /// Category path (backslash-separated for nesting).
    #[serde(default)]
    pub category: Option<String>,
    /// Comment text.
    #[serde(default)]
    pub comment: Option<String>,
}

#[derive(Clone)]
pub struct Server {
    runner: Arc<TokioOsascriptRunner>,
    // Serializes concurrent tool calls; MoneyMoney's AppleScript surface
    // isn't concurrency-safe.
    guard: Arc<Mutex<()>>,
    aliases: Arc<HashMap<String, String>>,
    tool_router: ToolRouter<Self>,
}

impl Server {
    #[must_use]
    pub fn new(aliases: HashMap<String, String>) -> Self {
        Self {
            runner: Arc::new(TokioOsascriptRunner),
            guard: Arc::new(Mutex::new(())),
            aliases: Arc::new(aliases),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router(router = tool_router)]
impl Server {
    /// Check whether MoneyMoney is running and unlocked.
    #[tool(
        description = "Report whether MoneyMoney is running and its database is unlocked. Returns {running, unlocked, version}.",
        annotations(read_only_hint = true)
    )]
    async fn status(&self) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let report = status::check(&*self.runner).await.map_err(to_mcp_err)?;
        serde_json::to_string(&report).map_err(to_mcp_err)
    }

    /// List all accounts (leaf only by default).
    #[tool(
        description = "List MoneyMoney accounts as an array of {uuid, bank, name, accountNumber (IBAN for bank accounts), balance, currency, portfolio, group, ...}. Non-group rows only.",
        annotations(read_only_hint = true)
    )]
    async fn list_accounts(&self) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let raw = fetch_all(&*self.runner).await.map_err(to_mcp_err)?;
        let rows: Vec<_> = annotate_with_bank(raw)
            .into_iter()
            .filter(|r| !r.account.group)
            .collect();
        serde_json::to_string(&rows).map_err(to_mcp_err)
    }

    /// Resolve a reference and return full details for a single account.
    #[tool(
        description = "Resolve a reference (UUID / IBAN / account number / alias / 'Bank/Name' / name) and return full details for one account. Ambiguous names return an error listing candidates as 'Bank/Name' paths.",
        annotations(read_only_hint = true)
    )]
    async fn get_account(
        &self,
        args: Parameters<GetAccountArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let raw = fetch_all(&*self.runner).await.map_err(to_mcp_err)?;
        let rows = annotate_with_bank(raw);
        let resolver = Resolver::new(rows, (*self.aliases).clone());
        let row = resolver.resolve(&args.0.account).map_err(to_mcp_err)?;
        serde_json::to_string(row).map_err(to_mcp_err)
    }

    /// List transactions for an account over a date range.
    #[tool(
        description = "List transactions for an account. Accepts 'account' (ref), 'from'/'to' (YYYY-MM-DD; defaults to last 90 days), optional 'search' substring (name/purpose/category/comment), and 'limit'.",
        annotations(read_only_hint = true)
    )]
    async fn list_transactions(
        &self,
        args: Parameters<ListTransactionsArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let raw = fetch_all(&*self.runner).await.map_err(to_mcp_err)?;
        let rows = annotate_with_bank(raw);
        let resolver = Resolver::new(rows, (*self.aliases).clone());
        let row = resolver.resolve(&args.0.account).map_err(to_mcp_err)?;

        let fmt = time::macros::format_description!("[year]-[month]-[day]");
        let today = time::OffsetDateTime::now_utc().date();
        let from = match &args.0.from {
            Some(s) => time::Date::parse(s, &fmt).map_err(to_mcp_err)?,
            None => today - time::Duration::days(90),
        };
        let to = match &args.0.to {
            Some(s) => time::Date::parse(s, &fmt).map_err(to_mcp_err)?,
            None => today,
        };

        let script = build_export_script(&row.account.account_number, from, to);
        let envelope: crate::moneymoney::types::TransactionsEnvelope =
            crate::applescript::run_plist(&*self.runner, &script)
                .await
                .map_err(to_mcp_err)?;

        let needle = args.0.search.as_deref().map(str::to_lowercase);
        let filtered: Vec<Transaction> = envelope
            .transactions
            .into_iter()
            .filter(|t| match &needle {
                Some(q) => [&t.name, &t.purpose, &t.comment]
                    .iter()
                    .any(|h| h.to_lowercase().contains(q.as_str())),
                None => true,
            })
            .collect();
        let out: Vec<Transaction> = match args.0.limit {
            Some(n) => filtered.into_iter().take(n).collect(),
            None => filtered,
        };
        serde_json::to_string(&out).map_err(to_mcp_err)
    }

    /// Return the full category tree.
    #[tool(
        description = "List the full MoneyMoney category tree. Returns an array of {uuid, name, group, indentation, budget, currency}. Hierarchy is encoded via indentation and document order.",
        annotations(read_only_hint = true)
    )]
    async fn list_categories(&self) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let cats: Vec<Category> = crate::applescript::run_plist(
            &*self.runner,
            "tell application \"MoneyMoney\" to export categories",
        )
        .await
        .map_err(to_mcp_err)?;
        serde_json::to_string(&cats).map_err(to_mcp_err)
    }

    /// List securities held in a portfolio account.
    #[tool(
        description = "List securities held in a portfolio / Depot account (`portfolio = true` accounts). Returns {name, isin, wkn, quantity, price, amount, currencyOfQuotation, priceDate, purchasePrice, tradingPlace, ...}.",
        annotations(read_only_hint = true)
    )]
    async fn get_portfolio(
        &self,
        args: Parameters<GetPortfolioArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let raw = fetch_all(&*self.runner).await.map_err(to_mcp_err)?;
        let rows = annotate_with_bank(raw);
        let resolver = Resolver::new(rows, (*self.aliases).clone());
        let row = resolver.resolve(&args.0.account).map_err(to_mcp_err)?;
        let script = format!(
            "tell application \"MoneyMoney\" to export portfolio from account \"{}\" as \"plist\"",
            row.account.account_number
        );
        let envelope: crate::moneymoney::types::PortfolioEnvelope =
            crate::applescript::run_plist(&*self.runner, &script)
                .await
                .map_err(to_mcp_err)?;
        serde_json::to_string(&envelope.portfolio).map_err(to_mcp_err)
    }

    /// List bank statement PDFs on disk.
    #[tool(
        description = "List bank statement PDFs from MoneyMoney's on-disk store. Optional 'account' (digit suffix / IBAN) and 'since' (YYYY-MM-DD) filters. Returns {bank, filename, path, date, accountHint, size}. IMPORTANT: filenames alone rarely answer the user's question; pair with get_statement and read the PDF content.",
        annotations(read_only_hint = true)
    )]
    async fn list_statements(
        &self,
        args: Parameters<ListStatementsArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let root = statements::default_root();
        let mut items = statements::walk(&root).map_err(to_mcp_err)?;
        if let Some(needle) = &args.0.account {
            let n = needle.trim();
            let n_lower = n.to_lowercase();
            items.retain(|s| {
                let by_hint = s
                    .account_hint
                    .as_ref()
                    .is_some_and(|h| n.ends_with(h.as_str()) || h.contains(n));
                let by_bank = s.bank.to_lowercase().contains(&n_lower);
                by_hint || by_bank
            });
        }
        if let Some(since_s) = &args.0.since {
            let fmt = time::macros::format_description!("[year]-[month]-[day]");
            let since = time::Date::parse(since_s, &fmt).map_err(to_mcp_err)?;
            items.retain(|s| s.date.is_some_and(|d| d >= since));
        }
        serde_json::to_string(&items).map_err(to_mcp_err)
    }

    /// Look up a statement by exact filename and return its absolute path.
    #[tool(
        description = "Look up a statement by exact filename (as returned by list_statements). Returns the absolute path. MCP hosts should open/read the PDF content to answer questions — don't stop at the filename.",
        annotations(read_only_hint = true)
    )]
    async fn get_statement(
        &self,
        args: Parameters<GetStatementArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let root = statements::default_root();
        let items = statements::walk(&root).map_err(to_mcp_err)?;
        let found = items
            .into_iter()
            .find(|s| s.filename == args.0.filename)
            .ok_or_else(|| {
                rmcp::ErrorData::invalid_params(
                    format!("no statement named '{}'", args.0.filename),
                    None,
                )
            })?;
        Ok(found.path.display().to_string())
    }

    /// Create a SEPA bank transfer. Safe by construction: MoneyMoney always
    /// opens a window or parks the payment in the outbox; the user enters
    /// the TAN via the GUI.
    #[tool(
        description = "Draft a SEPA bank transfer. MoneyMoney opens a pre-filled payment window (default) or puts the payment into the outbox (into_outbox=true). In both cases the user must confirm and enter a TAN in the GUI before money actually moves — no silent sends.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_transfer(
        &self,
        args: Parameters<CreateTransferArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let opts = CreateTransferOptions {
            from: args.0.from,
            to_iban: args.0.to_iban,
            to_name: args.0.to_name,
            amount: parse_amount(&args.0.amount)?,
            purpose: args.0.purpose,
            endtoend_reference: args.0.endtoend_reference,
            scheduled_date: parse_date_opt(args.0.scheduled_date.as_deref())?,
            into_outbox: args.0.into_outbox,
            aliases: (*self.aliases).clone(),
            format: None,
        };
        let raw = fetch_all(&*self.runner).await.map_err(to_mcp_err)?;
        let rows = annotate_with_bank(raw);
        let resolver = Resolver::new(rows, opts.aliases.clone());
        let row = resolver.resolve(&opts.from).map_err(to_mcp_err)?;
        let script = crate::commands::transfer::build_create_transfer_script(
            &row.account.account_number,
            &opts,
        )
        .map_err(to_mcp_err)?;
        self.runner.run(&script).await.map_err(to_mcp_err)?;
        Ok(confirmation_json(opts.into_outbox, "bank transfer"))
    }

    /// Create a SEPA direct debit. Same safety envelope as `create_transfer`.
    #[tool(
        description = "Draft a SEPA direct debit. Requires mandate_reference. Same GUI + TAN confirmation flow as create_transfer.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_direct_debit(
        &self,
        args: Parameters<CreateDirectDebitArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let opts = CreateDirectDebitOptions {
            from: args.0.from,
            debtor_iban: args.0.debtor_iban,
            debtor_name: args.0.debtor_name,
            amount: parse_amount(&args.0.amount)?,
            purpose: args.0.purpose,
            mandate_reference: args.0.mandate_reference,
            mandate_date: parse_date_opt(args.0.mandate_date.as_deref())?,
            scheduled_date: parse_date_opt(args.0.scheduled_date.as_deref())?,
            into_outbox: args.0.into_outbox,
            aliases: (*self.aliases).clone(),
            format: None,
        };
        let raw = fetch_all(&*self.runner).await.map_err(to_mcp_err)?;
        let rows = annotate_with_bank(raw);
        let resolver = Resolver::new(rows, opts.aliases.clone());
        let row = resolver.resolve(&opts.from).map_err(to_mcp_err)?;
        let script = crate::commands::transfer::build_direct_debit_script(
            &row.account.account_number,
            &opts,
        )
        .map_err(to_mcp_err)?;
        self.runner.run(&script).await.map_err(to_mcp_err)?;
        Ok(confirmation_json(opts.into_outbox, "direct debit"))
    }

    /// Import a SEPA XML batch. Must live inside MoneyMoney's app sandbox.
    #[tool(
        description = "Load a SEPA XML batch file. Set direct_debit=true for direct-debit batches. The XML file must be located within MoneyMoney's app sandbox. User confirms the batch in the GUI.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn create_batch_transfer(
        &self,
        args: Parameters<BatchTransferArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let opts = BatchTransferOptions {
            sepa_xml_path: std::path::PathBuf::from(args.0.file),
            direct_debit: args.0.direct_debit,
            format: None,
        };
        let script = crate::commands::transfer::build_batch_script(&opts).map_err(to_mcp_err)?;
        self.runner.run(&script).await.map_err(to_mcp_err)?;
        let verb = if opts.direct_debit {
            "batch direct debit"
        } else {
            "batch transfer"
        };
        Ok(confirmation_json(false, verb))
    }

    /// Add a manual entry to an offline account. No bank contact.
    #[tool(
        description = "Append a manual transaction to an offline-managed account. No bank contact; local mutation only. Category paths use backslashes for nesting.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    async fn add_transaction(
        &self,
        args: Parameters<AddTransactionArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let date = parse_date(&args.0.date)?;
        let opts = TxAddOptions {
            account: args.0.account,
            date,
            name: args.0.name,
            amount: parse_amount(&args.0.amount)?,
            purpose: args.0.purpose,
            category: args.0.category,
            aliases: (*self.aliases).clone(),
            format: None,
        };
        crate::commands::transaction_edit::run_add(&*self.runner, &opts)
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?;
        Ok(serde_json::json!({"message": "transaction added"}).to_string())
    }

    /// Modify metadata on an existing transaction. Silent overwrite.
    #[tool(
        description = "Set checkmark / category / comment on an existing transaction. SILENT overwrite — no GUI prompt, no undo. The 'id' is the integer transaction id from list_transactions.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    async fn set_transaction(
        &self,
        args: Parameters<SetTransactionArgs>,
    ) -> Result<String, rmcp::ErrorData> {
        let _g = self.guard.lock().await;
        let opts = TxSetOptions {
            id: args.0.id,
            checkmark: args.0.checkmark,
            category: args.0.category,
            comment: args.0.comment,
            format: None,
        };
        let script = crate::commands::transaction_edit::build_set_transaction_script(&opts)
            .map_err(to_mcp_err)?;
        self.runner.run(&script).await.map_err(to_mcp_err)?;
        Ok(serde_json::json!({"message": "transaction metadata updated"}).to_string())
    }
}

fn parse_amount(s: &str) -> Result<rust_decimal::Decimal, rmcp::ErrorData> {
    use std::str::FromStr as _;
    rust_decimal::Decimal::from_str(s.trim())
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid amount '{s}': {e}"), None))
}

fn parse_date(s: &str) -> Result<time::Date, rmcp::ErrorData> {
    let fmt = time::macros::format_description!("[year]-[month]-[day]");
    time::Date::parse(s, &fmt)
        .map_err(|e| rmcp::ErrorData::invalid_params(format!("invalid date '{s}': {e}"), None))
}

fn parse_date_opt(s: Option<&str>) -> Result<Option<time::Date>, rmcp::ErrorData> {
    s.map(parse_date).transpose()
}

fn confirmation_json(into_outbox: bool, verb: &str) -> String {
    serde_json::json!({
        "action": verb,
        "delivery": if into_outbox { "outbox" } else { "window" },
        "message": if into_outbox {
            "queued into the MoneyMoney outbox; user must release + TAN in the GUI"
        } else {
            "opened in a MoneyMoney payment window; user must confirm + TAN in the GUI"
        }
    })
    .to_string()
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::V_2025_06_18;
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = Implementation::from_build_env();
        "moneymoney".clone_into(&mut info.server_info.name);
        env!("CARGO_PKG_VERSION").clone_into(&mut info.server_info.version);
        info.instructions = Some(
            "Read-only access to MoneyMoney accounts, transactions, categories, portfolios, and on-disk bank statements. MoneyMoney must be running and unlocked. Use `status` first when debugging.".to_owned(),
        );
        info
    }
}

fn to_mcp_err<E: std::fmt::Display>(err: E) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(err.to_string(), None)
}

/// `mm mcp` entrypoint — spawn the stdio MCP server and block until the
/// client disconnects.
pub async fn run(aliases: HashMap<String, String>) -> anyhow::Result<()> {
    let server = Server::new(aliases);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
