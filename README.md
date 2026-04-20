# moneymoney

Agent-native CLI (`mm`) and MCP server for
[MoneyMoney](https://moneymoney.app/).

Query accounts, transactions, categories, portfolios, and bank
statements through a single Rust binary that works as:

- A standalone command-line tool (`mm accounts list`, `mm transactions --account …`, …)
- An MCP server over stdio (`mm mcp`) for Claude Desktop and other MCP hosts
- A Claude Code plugin that vendors both

## Install

| Path | Command | Use when |
|---|---|---|
| Plugin | `/plugin marketplace add lukasmalkmus/moneymoney`<br>`/plugin install moneymoney@moneymoney` | You use Claude Code and want a one-line setup. The plugin shim auto-downloads the right platform binary on first use. |
| Homebrew | `brew install lukasmalkmus/tap/mm` | You want `mm` on your PATH outside Claude Code too. |
| Cargo | `cargo install moneymoney --locked` | You have a Rust toolchain and prefer `cargo`. |
| From source | `git clone https://github.com/lukasmalkmus/moneymoney && cargo install --path moneymoney` | You're hacking on `mm`. |

Any locally-installed `mm` (via Cargo / Homebrew / elsewhere) takes
precedence over the plugin-managed download — the plugin shim strips
its own directory from `PATH` before looking for `mm`.

### Migration from v0.2 → v0.3

Nothing to do. If you installed v0.2 via `cargo install`, the v0.3
plugin shim still prefers that binary; the auto-download only kicks in
when `command -v mm` returns nothing. Bump at your own pace with
`cargo install moneymoney --locked --force`.

## 30-Second Tour

```bash
mm status                                 # is MoneyMoney running + unlocked?
mm accounts list                          # leaf accounts
mm accounts list --tree                   # full sidebar hierarchy
mm accounts get "ING/Girokonto"           # one account, full details
mm transactions --account "ING/Girokonto" --from 2026-01-01
mm portfolio --account "Trade Republic/Wertpapierdepot"
mm statements list --since 2026-01-01
```

Output defaults to **table** in a TTY and **JSON** when piped. Override
with `-o json|ndjson|table`. Filter fields with `-F bank,name,iban,balance`.
For SEPA accounts, `iban` is the normalized (no-whitespace, mod-97-verified)
IBAN; for PayPal or legacy accounts the field is absent.

## Writing

```bash
mm transfer create --from "ING/Girokonto" --to DE89... --amount 12.34 \
    --purpose "Rent" --into-outbox
mm transfer direct-debit --from "ING/Girokonto" --to DE89... \
    --amount 500 --mandate MANDATE-42
mm transfer batch path/to/sepa.xml --direct-debit
mm transaction add --account "Cash" --date 2026-04-20 \
    --name "Coffee" --amount -3.50 --category "Food\\Coffee"
mm transaction set 12345 --category "Food\\Groceries"
```

Transfers never move money silently: MoneyMoney opens a pre-filled
payment window (or parks the payment in the outbox with `--into-outbox`)
and you confirm + enter TAN in the GUI. Write subcommands are
deliberately omitted from the skill's `allowed-tools` so Claude Code
prompts for permission each time.

## Account References

`<REF>` is any of (priority order, first match wins):

1. **UUID** — always unique
2. **IBAN** — mod-97 validated via the `iban_validate` crate
3. **Account number** — PayPal emails, legacy digits
4. **Alias** — see config
5. **`Bank/Name`** path — e.g., `"ING/Girokonto"`
6. **Bare name** — only when unambiguous across banks

Ambiguous bare names (e.g., two "Girokonto" accounts) return an
`ambiguous_account` error listing both candidates as `Bank/Name` paths.

## Config (Optional)

`~/.config/mm/config.toml`:

```toml
[aliases]
checking = "ING/Girokonto"
depot    = "Trade Republic/Wertpapierdepot"
pp       = "PayPal"
```

`mm` itself holds **no credentials**. MoneyMoney owns every bank
secret in its encrypted store; `mm` just shells out to its AppleScript
surface.

## MCP

```bash
mm mcp                                    # starts the stdio MCP server
```

Register with Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "moneymoney": {
      "command": "/path/to/mm",
      "args": ["mcp"]
    }
  }
}
```

Read tools (`readOnlyHint: true`): `status`, `list_accounts`,
`get_account`, `list_transactions`, `list_categories`,
`get_portfolio`, `list_statements`, `get_statement`.

Write tools: `create_transfer`, `create_direct_debit`,
`create_batch_transfer`, `add_transaction` (not destructive — GUI + TAN
still intercept), and `set_transaction` (marked
`destructiveHint: true` — it silently overwrites checkmark / category /
comment).

## Platform Support

**macOS only at runtime.** AppleScript is the sanctioned MoneyMoney
interface and has no equivalent elsewhere. Builds are cross-platform
(Linux compiles cleanly) but runtime commands return `not_supported`
off macOS.

## Logging

All diagnostic output goes to **stderr**. stdout is reserved for
structured results (CLI) or JSON-RPC frames (MCP). Control verbosity
through `MM_LOG` (any `tracing_subscriber::EnvFilter` syntax):

```bash
MM_LOG=mm=debug mm accounts list
```

## Build & Test

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

**MSRV:** 1.94 (edition 2024).

## License

MIT
