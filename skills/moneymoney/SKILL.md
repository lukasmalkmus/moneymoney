---
name: moneymoney
description: |
  Query and act on MoneyMoney data via the `mm` CLI. Accounts, transactions,
  categories, portfolio, and bank statement PDFs, plus drafting SEPA
  transfers, direct debits, batch transfers, and offline-account entries
  through MoneyMoney. Use when asked about personal finance, bank balances, recent
  transactions, spending, portfolio holdings, or bank statements, OR wants to
  make or draft a payment. Triggers on "Überweisung", "überweisen",
  "Lastschrift", "SEPA", "transfer", "direct debit", "send money", "pay this
  invoice", "zahle diese Rechnung".
compatibility: Requires macOS and MoneyMoney. Plugin installation requires Bash, curl, and tar. MoneyMoney must be running and unlocked for application operations.
allowed-tools: Bash(mm status) Bash(mm accounts *) Bash(mm transactions *) Bash(mm categories *) Bash(mm portfolio *) Bash(mm statements *) Bash(mm version) Bash(mm mcp) Bash(mm mcp *) Read
---

# MoneyMoney Query

Query MoneyMoney accounts, transactions, categories, portfolios, and bank
statement PDFs via the `mm` CLI.

Process the user's question or request.

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
  ├── Bank statement PDFs ────── mm statements list [--account <REF>] [--since YYYY-MM-DD]
  │     └── Retrieve a PDF ──── mm statements get <FILENAME>
  ├── Send money / pay invoice ─ mm transfer create --from <REF> --to <IBAN> --amount X.XX --purpose "..."
  │     └── Hold for later ──── add --into-outbox (lands in Ausgangskorb)
  ├── Direct debit ───────────── mm transfer direct-debit --from <REF> --to <IBAN> --amount X.XX --mandate <MANDATE-ID>
  ├── SEPA XML batch ─────────── mm transfer batch <file.xml>
  ├── Add offline entry ──────── mm transaction add --account <OFFLINE-REF> --amount X.XX --purpose "..."
  └── Edit transaction meta ─── mm transaction set <UUID> [--checkmark] [--category "..."] [--comment "..."]
```

## Actions and Confirmation

`mm transfer *` and `mm transaction *` are available and intentionally
not preapproved by this skill's `allowed-tools`. Tool approval depends on the
host's policy. The field does not restrict available tools or guarantee a
permission prompt. Confirm the action and its parameters with the user before
any write. MoneyMoney handles its own payment review and authorization flow.

If the user says "Überweisung", "überweisen", "Lastschrift", "SEPA",
"transfer", "send money", or "pay this invoice", do **not** reply "I
have no tools for that." Draft the command and confirm the parameters
with the user. Follow the host's approval and financial-action policies.
The user must complete any payment authorization in MoneyMoney.

### Worked example — pay an invoice PDF

```
1. Read the PDF → extract recipient name, IBAN, amount, purpose /
   reference number.
2. Echo the parsed fields back to the user and get a "go ahead".
3. Pick a source account (usually the main Girokonto unless the
   user says otherwise).
4. mm transfer create \
     --from "ING/Girokonto" \
     --to   "DE17500400000076139950" \
     --name "Anke Irma Johannmeier" \
     --amount 701.74 \
     --purpose "Rechnung 80500-01-2026"
5. MoneyMoney opens a pre-filled payment window. User reviews,
   enters TAN, and releases the transfer.
```

### Worked example — queue for later (`--into-outbox`)

```
mm transfer create --from "ING/Girokonto" --to "DE..." \
  --amount 50.00 --purpose "Rent April" --into-outbox
```

The payment lands in MoneyMoney's Ausgangskorb and stays there until
the user releases it manually (useful for collecting several and
releasing them as a batch).

### Silent mutators — warn before running

`mm transaction set` overwrites `--comment` and `--category` without
prompting a second time and without an undo. Before calling it,
confirm the target transaction UUID and the new value(s) with the
user. `--checkmark` is reversible (just run `set` again with the
opposite value).

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
3. Open <ABSOLUTE_PATH> with the host's PDF reader
         ↓ inspect the document content, using page ranges where needed
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
  PDFs): use a PDF reader that supports page images and describe what's
  visible rather than pretending to have parsed text.
- **Long PDFs**: MoneyMoney's monthly `Kontoauszug` is often 20–30 pages.
  Read it in page ranges if the host's PDF reader limits pages per call.
  In Claude Code, retry a *"too many pages"* error with `pages: "1-20"`,
  then `pages: "21-40"` for the rest.
- **`mm statements list` returns empty when you expect results**:
  the filename filter is heuristic. Fall back to a direct filesystem
  listing of the Statements folder and pick PDFs by name:

  ```bash
  ls "$HOME/Library/Containers/com.moneymoney-app.retail/Data/Library/Application Support/MoneyMoney/Statements/<Bank>/"
  ```

  Then `Read` the absolute path directly — same result as `mm
  statements get`, minus the filter.

## When `mm transactions` returns empty for historical periods

Banks (notably ING) purge synced transactions after roughly 90 days;
MoneyMoney only has what it pulled while active. **For anything older
than ~3 months, statement PDFs are the authoritative source**, not
`mm transactions`. If the user asks about last year's spending or a
specific transaction from months ago and `mm transactions --from …`
returns zero or suspiciously few rows, switch to `mm statements list`
and read the monthly `Kontoauszug` PDFs instead — don't conclude
"there's no data."

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
with `--output json|ndjson|table`. Use `-F name,iban,balance` to filter
fields. For SEPA accounts `iban` is the normalized IBAN; for PayPal /
legacy accounts it's absent.

## JSON Envelope

Lists return:

```json
{"results": [...], "total_count": N, "showing": N, "has_more": bool}
```

Individual records (from `mm accounts get`) return the record object
directly.

## Common Pitfalls

| Wrong | Right | Why |
|-------|-------|-----|
| `mm accounts get Girokonto` (when ambiguous) | `mm accounts get "ING/Girokonto"` | Names aren't unique |
| Parsing `mm accounts list` table output | `mm accounts list --output json` | JSON is stable |
| Running commands while app is locked | `mm status` first, then retry | Database access fails silently otherwise |
| Searching transactions without a date range | Always pass `--from` / `--to` | Default range is last 90 days |
| Answering a statement question from filenames alone | `mm statements get` + `Read` the PDF | Statements are PDFs — their content is the answer |
| "I can't make transfers — I have no tools for that" | Draft the command, confirm parameters, and follow host policy | Write verbs exist, but approval behavior belongs to the host and payment authorization belongs to MoneyMoney |
| A PDF reader fails with "too many pages" | Retry with a page range (in Claude Code, `pages: "1-20"`) | PDF-reader limits vary by host |
| `mm transactions --from 2024-…` returns zero and you conclude "no data" | Switch to `mm statements list` + Read the monthly PDFs | Banks purge transactions after ~90 days; PDFs always contain the full history |
| `mm statements list --account "ING/Girokonto"` returns empty | `ls "$HOME/Library/Containers/com.moneymoney-app.retail/Data/Library/Application Support/MoneyMoney/Statements/ING/"` and Read the paths directly | Filename filter is heuristic; bypass it when it misses |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | General error |
| 2 | Usage / field filter / ambiguous account reference |
| 3 | Not found (account, transaction, statement) |
| 4 | AppleScript / I/O error |
| 5 | MoneyMoney locked or not running |
