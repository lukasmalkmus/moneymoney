use std::io::IsTerminal as _;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

mod applescript;
mod commands;
mod config;
mod logging;
mod mcp;
mod moneymoney;
mod output;
mod statements;

use applescript::TokioOsascriptRunner;
use moneymoney::MoneyMoneyError;
use output::{FieldFilterError, OutputFormat};

#[derive(Parser)]
#[command(
    name = "mm",
    version,
    about = "Agent-native CLI and MCP server for MoneyMoney"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Output errors as JSON to stderr.
    #[arg(long, global = true, env = "MM_JSON_ERRORS")]
    json_errors: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Print version information.
    Version,

    /// Report whether MoneyMoney is running and unlocked.
    Status,

    /// Inspect MoneyMoney accounts.
    #[command(subcommand)]
    Accounts(AccountsCommand),

    /// List transactions for an account.
    Transactions {
        /// Account reference (UUID / IBAN / account# / alias / Bank/Name / name).
        #[arg(short, long)]
        account: String,

        /// Start date (inclusive), YYYY-MM-DD. Defaults to 90 days ago.
        #[arg(long, value_parser = parse_ymd)]
        from: Option<time::Date>,

        /// End date (inclusive), YYYY-MM-DD. Defaults to today.
        #[arg(long, value_parser = parse_ymd)]
        to: Option<time::Date>,

        /// Substring filter applied to name / purpose / category / comment.
        #[arg(long)]
        search: Option<String>,

        /// Maximum number of results.
        #[arg(long)]
        limit: Option<usize>,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// List the full category tree.
    Categories {
        #[command(flatten)]
        output: OutputArgs,
    },

    /// List securities held in a portfolio account.
    Portfolio {
        /// Portfolio account reference.
        #[arg(short, long)]
        account: String,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Work with bank statement PDFs stored by MoneyMoney on disk.
    #[command(subcommand)]
    Statements(StatementsCommand),

    /// Run the MCP server over stdio (consumed by hosts like Claude
    /// Desktop).
    Mcp,

    /// Initiate a SEPA bank transfer or direct debit.
    ///
    /// Transfers are never sent silently — MoneyMoney either opens a
    /// pre-filled window (default) or places the payment into the outbox
    /// (`--into-outbox`), and the user must confirm and enter a TAN before
    /// money moves.
    #[command(subcommand)]
    Transfer(TransferCommand),

    /// Work with transactions (offline-account entries and metadata edits).
    #[command(subcommand)]
    Transaction(TransactionCommand),
}

#[derive(Subcommand)]
enum TransferCommand {
    /// Create a single SEPA bank transfer.
    Create {
        /// Source account reference.
        #[arg(long)]
        from: String,
        /// Recipient IBAN.
        #[arg(long = "to")]
        to_iban: String,
        /// Recipient name (shown in the payment window).
        #[arg(long)]
        name: Option<String>,
        /// Amount in source-account currency (dot-decimal).
        #[arg(long, value_parser = parse_amount)]
        amount: rust_decimal::Decimal,
        /// Purpose / remittance text.
        #[arg(long)]
        purpose: Option<String>,
        /// SEPA end-to-end reference.
        #[arg(long = "endtoend")]
        endtoend_reference: Option<String>,
        /// Scheduled execution date (YYYY-MM-DD).
        #[arg(long = "scheduled", value_parser = parse_ymd)]
        scheduled_date: Option<time::Date>,
        /// Drop the payment into the MoneyMoney outbox silently (user
        /// still has to release + TAN from the GUI).
        #[arg(long)]
        into_outbox: bool,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Create a single SEPA direct debit.
    DirectDebit {
        #[arg(long)]
        from: String,
        #[arg(long = "to")]
        debtor_iban: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, value_parser = parse_amount)]
        amount: rust_decimal::Decimal,
        #[arg(long)]
        purpose: Option<String>,
        #[arg(long = "mandate")]
        mandate_reference: String,
        #[arg(long = "mandate-date", value_parser = parse_ymd)]
        mandate_date: Option<time::Date>,
        #[arg(long = "scheduled", value_parser = parse_ymd)]
        scheduled_date: Option<time::Date>,
        #[arg(long)]
        into_outbox: bool,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Import a SEPA XML batch (must live inside MoneyMoney's sandbox).
    Batch {
        /// Path to the SEPA XML file.
        file: std::path::PathBuf,
        /// Treat the batch as direct debits rather than transfers.
        #[arg(long)]
        direct_debit: bool,

        #[command(flatten)]
        output: OutputArgs,
    },
}

#[derive(Subcommand)]
enum TransactionCommand {
    /// Append a manual entry to an offline-managed account.
    Add {
        /// Offline account reference.
        #[arg(long)]
        account: String,
        /// Booking date (YYYY-MM-DD).
        #[arg(long, value_parser = parse_ymd)]
        date: time::Date,
        /// Debitor or creditor name.
        #[arg(long)]
        name: String,
        /// Amount in the account's currency (dot-decimal).
        #[arg(long, value_parser = parse_amount)]
        amount: rust_decimal::Decimal,
        /// Purpose / remittance text.
        #[arg(long)]
        purpose: Option<String>,
        /// Category. Use a backslash to separate nested names (e.g.
        /// `Food\Coffee`). If omitted, auto-categorization runs.
        #[arg(long)]
        category: Option<String>,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Modify metadata on an existing transaction.
    Set {
        /// Transaction id (integer; obtain via `mm transactions --output json`).
        id: i64,
        /// Set or clear the user checkmark.
        #[arg(long)]
        checkmark: Option<bool>,
        /// Category to assign.
        #[arg(long)]
        category: Option<String>,
        /// Comment to set.
        #[arg(long)]
        comment: Option<String>,

        #[command(flatten)]
        output: OutputArgs,
    },
}

#[derive(Subcommand)]
enum StatementsCommand {
    /// List statements with optional filters.
    List {
        /// Account filter (account-number suffix, IBAN, or bare digits).
        #[arg(long)]
        account: Option<String>,

        /// Earliest statement date to include.
        #[arg(long, value_parser = parse_ymd)]
        since: Option<time::Date>,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Retrieve a statement by filename. Defaults to printing the path;
    /// `--open` launches the macOS default PDF viewer; `--stdout` writes
    /// PDF bytes to stdout.
    Get {
        /// Exact filename of a statement as returned by `statements list`.
        filename: String,
        /// Open in the default PDF viewer (macOS only).
        #[arg(long, conflicts_with = "stdout")]
        open: bool,
        /// Write the PDF bytes to stdout instead of printing the path.
        #[arg(long)]
        stdout: bool,
    },
}

#[derive(Args, Default)]
struct OutputArgs {
    /// Output format: table, json, ndjson.
    #[arg(short, long, global = true)]
    output: Option<OutputFormat>,

    /// Comma-separated list of fields to include in output.
    #[arg(short = 'F', long, global = true)]
    fields: Option<String>,
}

#[derive(Subcommand)]
enum AccountsCommand {
    /// List all accounts. Leaf accounts only by default.
    List {
        /// Visualize the sidebar hierarchy (implies `--include-groups`).
        #[arg(long)]
        tree: bool,

        /// Include bank / group parent rows in the output.
        #[arg(long)]
        include_groups: bool,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Resolve a reference (UUID, IBAN, account number, alias, `Bank/Name`,
    /// or name) and show the account's full details.
    Get {
        /// Account reference.
        reference: String,

        #[command(flatten)]
        output: OutputArgs,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_errors = cli.json_errors || !std::io::stderr().is_terminal();

    // stdout is sacred in MCP mode (JSON-RPC frames); log routing is
    // identical except for format + default verbosity.
    let mode = if matches!(cli.command, Command::Mcp) {
        logging::Mode::Mcp
    } else {
        logging::Mode::Cli
    };
    logging::init(mode);

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("failed to start async runtime: {err}");
            return ExitCode::from(1);
        }
    };

    // Load aliases once up-front; propagate parse errors to the user rather
    // than silently falling back to an empty map.
    let aliases = match config::load() {
        Ok(cfg) => cfg.aliases,
        Err(err) => {
            eprintln!("Warning: {err}");
            std::collections::HashMap::new()
        }
    };

    let result = runtime.block_on(run(cli.command, aliases));

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let code = exit_code(&err);
            if json_errors {
                output::print_json_error(&err, error_code(&err));
            } else {
                eprintln!("Error: {err:#}");
            }
            ExitCode::from(code)
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "wide match on a growing subcommand tree; splitting obscures the dispatch"
)]
async fn run(
    command: Command,
    aliases: std::collections::HashMap<String, String>,
) -> anyhow::Result<()> {
    match command {
        Command::Version => {
            println!("mm {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Status => commands::status::run(&TokioOsascriptRunner).await,
        Command::Accounts(sub) => match sub {
            AccountsCommand::List {
                tree,
                include_groups,
                output,
            } => {
                commands::accounts::run_list(
                    &TokioOsascriptRunner,
                    commands::accounts::ListOptions {
                        format: output.output,
                        fields: output.fields,
                        tree,
                        include_groups,
                    },
                )
                .await
            }
            AccountsCommand::Get { reference, output } => {
                commands::accounts::run_get(
                    &TokioOsascriptRunner,
                    commands::accounts::GetOptions {
                        reference,
                        format: output.output,
                        fields: output.fields,
                        aliases: aliases.clone(),
                    },
                )
                .await
            }
        },
        Command::Transactions {
            account,
            from,
            to,
            search,
            limit,
            output,
        } => {
            commands::transactions::run_list(
                &TokioOsascriptRunner,
                commands::transactions::ListOptions {
                    reference: account,
                    from,
                    to,
                    search,
                    limit,
                    format: output.output,
                    fields: output.fields,
                    aliases: aliases.clone(),
                },
            )
            .await
        }
        Command::Categories { output } => {
            commands::categories::run_list(
                &TokioOsascriptRunner,
                commands::categories::ListOptions {
                    format: output.output,
                    fields: output.fields,
                },
            )
            .await
        }
        Command::Portfolio { account, output } => {
            commands::portfolio::run(
                &TokioOsascriptRunner,
                commands::portfolio::GetOptions {
                    reference: account,
                    format: output.output,
                    fields: output.fields,
                    aliases: aliases.clone(),
                },
            )
            .await
        }
        Command::Statements(sub) => match sub {
            StatementsCommand::List {
                account,
                since,
                output,
            } => commands::statements::run_list(&commands::statements::ListOptions {
                account,
                since,
                format: output.output,
                fields: output.fields,
            }),
            StatementsCommand::Get {
                filename,
                open,
                stdout,
            } => commands::statements::run_get(&commands::statements::GetOptions {
                filename,
                open,
                stdout,
            }),
        },
        Command::Mcp => mcp::run(aliases).await,
        Command::Transfer(sub) => match sub {
            TransferCommand::Create {
                from,
                to_iban,
                name,
                amount,
                purpose,
                endtoend_reference,
                scheduled_date,
                into_outbox,
                output,
            } => {
                commands::transfer::run_create(
                    &TokioOsascriptRunner,
                    &commands::transfer::CreateTransferOptions {
                        from,
                        to_iban,
                        to_name: name,
                        amount,
                        purpose,
                        endtoend_reference,
                        scheduled_date,
                        into_outbox,
                        aliases: aliases.clone(),
                        format: output.output,
                    },
                )
                .await
            }
            TransferCommand::DirectDebit {
                from,
                debtor_iban,
                name,
                amount,
                purpose,
                mandate_reference,
                mandate_date,
                scheduled_date,
                into_outbox,
                output,
            } => {
                commands::transfer::run_direct_debit(
                    &TokioOsascriptRunner,
                    &commands::transfer::CreateDirectDebitOptions {
                        from,
                        debtor_iban,
                        debtor_name: name,
                        amount,
                        purpose,
                        mandate_reference,
                        mandate_date,
                        scheduled_date,
                        into_outbox,
                        aliases: aliases.clone(),
                        format: output.output,
                    },
                )
                .await
            }
            TransferCommand::Batch {
                file,
                direct_debit,
                output,
            } => {
                commands::transfer::run_batch(
                    &TokioOsascriptRunner,
                    &commands::transfer::BatchTransferOptions {
                        sepa_xml_path: file,
                        direct_debit,
                        format: output.output,
                    },
                )
                .await
            }
        },
        Command::Transaction(sub) => match sub {
            TransactionCommand::Add {
                account,
                date,
                name,
                amount,
                purpose,
                category,
                output,
            } => {
                commands::transaction_edit::run_add(
                    &TokioOsascriptRunner,
                    &commands::transaction_edit::AddOptions {
                        account,
                        date,
                        name,
                        amount,
                        purpose,
                        category,
                        aliases: aliases.clone(),
                        format: output.output,
                    },
                )
                .await
            }
            TransactionCommand::Set {
                id,
                checkmark,
                category,
                comment,
                output,
            } => {
                commands::transaction_edit::run_set(
                    &TokioOsascriptRunner,
                    &commands::transaction_edit::SetOptions {
                        id,
                        checkmark,
                        category,
                        comment,
                        format: output.output,
                    },
                )
                .await
            }
        },
    }
}

fn parse_ymd(s: &str) -> Result<time::Date, String> {
    let fmt = time::macros::format_description!("[year]-[month]-[day]");
    time::Date::parse(s, &fmt).map_err(|e| format!("invalid date '{s}': {e}"))
}

fn parse_amount(s: &str) -> Result<rust_decimal::Decimal, String> {
    use std::str::FromStr as _;
    let trimmed = s.trim();
    let d = rust_decimal::Decimal::from_str(trimmed)
        .map_err(|e| format!("invalid amount '{s}': {e}"))?;
    if d.is_zero() {
        return Err("amount must be non-zero".to_owned());
    }
    if d.scale() > 4 {
        return Err(format!("amount '{s}' has too many decimal places (max 4)"));
    }
    Ok(d)
}

fn exit_code(err: &anyhow::Error) -> u8 {
    if err.downcast_ref::<FieldFilterError>().is_some() {
        return 2;
    }
    if let Some(mm_err) = err.downcast_ref::<MoneyMoneyError>() {
        return match mm_err {
            MoneyMoneyError::DatabaseLocked | MoneyMoneyError::NotRunning => 5,
            MoneyMoneyError::NotInstalled
            | MoneyMoneyError::NotSupported
            | MoneyMoneyError::ScriptError(_)
            | MoneyMoneyError::Spawn(_)
            | MoneyMoneyError::PlistDecode(_) => 4,
            MoneyMoneyError::AccountNotFound(_) => 3,
            MoneyMoneyError::AmbiguousAccount { .. }
            | MoneyMoneyError::InvalidIban(_)
            | MoneyMoneyError::AccountIsGroup(_)
            | MoneyMoneyError::AliasCycle(_)
            | MoneyMoneyError::AccountNotOffline(_)
            | MoneyMoneyError::InvalidScriptInput { .. } => 2,
        };
    }
    1
}

fn error_code(err: &anyhow::Error) -> &'static str {
    if err.downcast_ref::<FieldFilterError>().is_some() {
        return "usage_error";
    }
    if let Some(mm_err) = err.downcast_ref::<MoneyMoneyError>() {
        return match mm_err {
            MoneyMoneyError::DatabaseLocked => "database_locked",
            MoneyMoneyError::NotRunning => "not_running",
            MoneyMoneyError::NotInstalled => "not_installed",
            MoneyMoneyError::NotSupported => "not_supported",
            MoneyMoneyError::ScriptError(_) => "script_error",
            MoneyMoneyError::Spawn(_) => "spawn_error",
            MoneyMoneyError::PlistDecode(_) => "plist_decode_error",
            MoneyMoneyError::AccountNotFound(_) => "account_not_found",
            MoneyMoneyError::AmbiguousAccount { .. } => "ambiguous_account",
            MoneyMoneyError::InvalidIban(_) => "invalid_iban",
            MoneyMoneyError::AccountIsGroup(_) => "account_is_group",
            MoneyMoneyError::AliasCycle(_) => "alias_cycle",
            MoneyMoneyError::AccountNotOffline(_) => "account_not_offline",
            MoneyMoneyError::InvalidScriptInput { .. } => "invalid_script_input",
        };
    }
    "general_error"
}
