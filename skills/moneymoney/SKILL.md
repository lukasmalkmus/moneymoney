---
name: moneymoney
description: |
  Query MoneyMoney accounts, transactions, categories, portfolio, and bank
  statements. Use when asked about personal finance, bank balances, recent
  transactions, spending by category, portfolio holdings, or bank statements
  from MoneyMoney.
user-invocable: true
argument-hint: <question-or-query>
allowed-tools: Bash(mm status), Bash(mm accounts *), Bash(mm transactions *), Bash(mm categories *), Bash(mm portfolio *), Bash(mm statements *), Bash(mm version), Bash(mm mcp *), Read
---

# MoneyMoney Query

Query MoneyMoney accounts, transactions, categories, portfolios, and bank
statement PDFs via the `mm` CLI.

Process: `$ARGUMENTS`

## Prerequisite

**MoneyMoney must be running AND unlocked.** Run `mm status` first when in
doubt; if `unlocked: false`, ask the user to unlock the app GUI before
retrying.

## Decision Tree

```
User wants…
  ├── App health ─────────────── mm status
  ├── Account overview ────────── mm accounts list [--tree]
  │     └── Detail ──────────── mm accounts get <REF>
  ├── Recent transactions ────── mm transactions --account <REF> [--from YYYY-MM-DD]
  │     └── Filtered ────────── mm transactions --account <REF> --search "Supermarket"
  ├── Category tree ──────────── mm categories
  ├── Portfolio holdings ─────── mm portfolio --account <DEPOT-REF>
  └── Bank statement PDFs ────── mm statements list [--account <REF>] [--since YYYY-MM-DD]
        └── Retrieve a PDF ───── mm statements get <FILENAME>
```

## Bank Statements — Always Examine PDF Content

Questions like "what's in my latest statement?", "check my Trade
Republic statements", or "did my rent debit show up on the January
statement?" are **never** answered by filenames alone. Statements are
PDFs stored on disk; their filename is an index, not the content.

**The required workflow:**

```
1. mm statements list --account <REF> [--since YYYY-MM-DD] -o json
         ↓ pick the relevant statement(s) by bank/date/account_hint
2. mm statements get <EXACT_FILENAME>
         ↓ prints the absolute path to the PDF
3. Read <ABSOLUTE_PATH>
         ↓ Claude Code's Read tool accepts PDFs directly (page-aware)
```

The filename pattern `<AccountType>_<digits>_<DocType>_<YYYYMMDD>.pdf`
often gives you the account, the document kind (e.g., `Kontoauszug`,
`Entgeltaufstellung`, `Ertraegnisaufstellung`, `Depotauszug`,
`Information`), and a date — but never the numbers, charges, or
transaction lines the user actually wants.

Worked example — "what did my ING statement from January look like?":

```bash
mm statements list --account ING --since 2026-01-01 -o json | jq '.results'
#   → picks Girokonto_5437633269_Kontoauszug_20260108.pdf
mm statements get Girokonto_5437633269_Kontoauszug_20260108.pdf
#   → /Users/you/Library/Containers/.../Statements/ING/...pdf
#   Then: Read that path to see balances, transactions, fees, interest.
```

**Special cases:**

- **Bank-wide documents** (`Information_YYYYMMDD.pdf`, etc.) have no
  account hint and affect every account at that bank — always worth
  reading when the user asks "did anything change?".
- **Large statement sets**: use `--since` to bound the range rather
  than reading everything. Read only the PDFs plausibly relevant to
  the question.
- **Non-textual statements** (Trade Republic sometimes ships scanned
  PDFs): Read will return page-level image content; describe what's
  visible rather than pretending to have parsed text.

## Account References

`<REF>` is any of (first match wins in priority order):

1. **UUID** — stable, always unique
2. **IBAN** — mod-97 validated
3. **Account number** — PayPal email, legacy numbers
4. **Alias** — from `~/.config/mm/config.toml`
5. **`Bank/Name` path** — e.g., `"ING/Girokonto"`, `"Trade Republic/Girokonto"`
6. **Bare name** — only when unambiguous

**Names are NOT unique across banks.** Two "Girokonto" accounts under
different banks are common. If `mm accounts get Girokonto` errors with
`ambiguous_account`, disambiguate with a `Bank/Name` path:

```bash
mm accounts get "ING/Girokonto"
mm accounts get "Trade Republic/Girokonto"
```

## Output Formats

Defaults to **table** in terminal, **JSON** when piped (for agents). Override
with `--output json|ndjson|table`. Use `-F name,account,balance` to filter
fields.

## JSON Envelope

Lists return:

```json
{"results": [...], "total_count": N, "showing": N, "has_more": bool}
```

Individual records (from `mm accounts get`) return the record object
directly.

## Write Subcommands

`mm transfer *` and `mm transaction *` exist but are **deliberately omitted
from `allowed-tools`**. Using them prompts the user for permission. All
transfers go through MoneyMoney's GUI + TAN intercept, so nothing leaves
the bank without explicit user interaction.

## Common Pitfalls

| Wrong | Right | Why |
|-------|-------|-----|
| `mm accounts get Girokonto` (when ambiguous) | `mm accounts get "ING/Girokonto"` | Names aren't unique |
| Parsing `mm accounts list` table output | `mm accounts list --output json` | JSON is stable |
| Running commands while app is locked | `mm status` first, then retry | Database access fails silently otherwise |
| Searching transactions without a date range | Always pass `--from` / `--to` | Default range is last 90 days |
| Answering a statement question from filenames alone | `mm statements get` + `Read` the PDF | Statements are PDFs — their content is the answer |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Usage / field filter / ambiguous account reference |
| 3 | Not found (account, transaction, statement) |
| 4 | AppleScript / I/O error |
| 5 | MoneyMoney locked or not running |
