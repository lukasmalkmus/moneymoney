# moneymoney

Agent-native CLI (`mm`) and MCP server for [MoneyMoney](https://moneymoney.app/).

**Repository:** `~/Code/Private/moneymoney`

## CLI Shape (target)

```
mm [--output json|ndjson|table] [-F fields] [--json-errors]

mm status
mm accounts list [--tree] [--include-groups]
mm accounts get <REF>
mm transactions --account <REF> [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--search term] [--limit N]
mm categories
mm portfolio --account <REF>
mm statements list [--account <iban>] [--since YYYY-MM]
mm statements get <filename>
mm transfer create --from <REF> --to <iban> --amount D.DD --purpose "..." [--into-outbox]
mm transfer direct-debit --from <REF> --to <iban> --amount ... --mandate ...
mm transfer batch <sepa.xml> [--into-outbox]
mm transaction add --account <REF> --amount ... --purpose ...
mm transaction set <id> [--checkmark] [--category "..."] [--comment "..."]
mm mcp
mm version
```

`REF` = account UUID, IBAN, account number, alias (from config), `Bank/Name`
path, or bare name (rejected with candidate list when ambiguous).

## Output

- **Table** (default in TTY), **JSON** (default when piped), **NDJSON**
- JSON envelope: `{"results": [...], "total_count": N, "showing": N, "has_more": bool}`
- Field filtering: `-F name,bank,iban`
- Structured errors: `{"error": "...", "code": "..."}` on stderr when
  `--json-errors` or stderr is non-TTY.

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Usage / field filter / ambiguous account reference |
| 3 | Not found (account, transaction id, statement) |
| 4 | AppleScript / I/O / MoneyMoney runtime error |
| 5 | MoneyMoney locked or not running |

## Logging

`MM_LOG=<env-filter>` controls tracing. Output always goes to stderr.

Defaults: `warn` in CLI mode, `info` in MCP mode (structured JSON).

## Build

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

**MSRV:** 1.94 (edition 2024)

## Commit Format

`scope: description` (e.g., `applescript: escape quotes in purpose strings`,
`resolver: add Bank/Name path matcher`, `commands/transfer: add --into-outbox`).

## Dependencies

| Crate | Purpose |
|-------|---------|
| `clap` | CLI argument parsing (derive) |
| `rmcp` | Model Context Protocol server (official SDK) |
| `plist` | Parse AppleScript `as "plist"` output |
| `rust_decimal` | Monetary arithmetic |
| `time` | Date/time handling |
| `tokio` | Async runtime (current-thread) |
| `tracing` / `tracing-subscriber` | Structured logging to stderr |
| `serde` / `serde_json` | Serialization |
| `toml` / `dirs` | Config file (aliases) |
| `regex` | Statement filename parsing |
| `comfy-table` | Table output |
| `owo-colors` | Terminal colors |
| `anyhow` / `thiserror` | Error handling |

## Skills

- `skills/mm/SKILL.md` — agent workflow guide (read-only `allowed-tools`)
