# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-04-21

### Fixed

- Plugin shim `bin/mm` and SessionStart hook `hooks/ensure-binary.sh`
  now fall back to a pure-bash `sed` extractor when `jq` is not on
  PATH, so the first `mm` invocation no longer fails on stock macOS,
  Alpine, and slim Linux containers.

## [0.4.0] - 2026-04-20

### Fixed

- `mm statements list --account "Bank/Name"` (e.g.
  `"ING/Girokonto"`) now matches. Previously the filter only
  accepted bare bank names or account-number digits; the
  `Bank/Name` form — the one the skill recommends — silently
  returned zero results because the needle didn't match either
  the bank folder or the filename's digit hint.
- IBAN-form account refs (`DE…5437633269`) are now recognized by
  the statements filter via trailing-digit matching against the
  filename's `account_hint`.

### Changed

- Extracted the statements account filter into
  `statements::matches_account` and deduplicated CLI + MCP
  call sites. Both now share the same form-aware matcher.

### Skill

- "Bank Statements" section now warns about the 20-page Read
  limit and the `pages: "1-20"` workaround.
- New section "When `mm transactions` returns empty for
  historical periods" — tells the agent to fall through to
  statement PDFs when bank sync has gaps.
- New fallback note for `mm statements list` returning empty:
  direct `ls` of the Statements folder bypasses the heuristic
  filter.
- Three new Common Pitfalls rows covering the failure modes
  above.

## [0.3.0] - 2026-04-20

### Added

- `iban` field on account output (table column, JSON key, `-F iban`
  filter). Populated for SEPA accounts whose `account_number` passes
  mod-97; absent for PayPal and legacy numbers.
- Skill decision tree and frontmatter now cover write intents:
  "Überweisung", "Lastschrift", "SEPA", "transfer", "direct debit",
  "send money", "pay this invoice". The skill triggers on those
  phrases and walks through drafting `mm transfer create` with the
  permission prompt + GUI/TAN as the safety gates.

### Changed

- Skill body reorganized: a prominent "Actions (Permission-Prompted)"
  section moves ahead of statement handling, with worked examples for
  paying an invoice PDF and queueing into MoneyMoney's Ausgangskorb.
- "Write Subcommands" tail section removed (content moved up).

## [0.2.0] - 2026-04-20

### Added

- `mm transfer create` / `direct-debit` / `batch` — initiate SEPA
  payments. Safe by construction: MoneyMoney either opens a pre-filled
  payment window (default) or places the payment into the outbox
  (`--into-outbox`); user confirms and enters TAN in the GUI.
- `mm transaction add` — append manual entries to offline-managed
  accounts (non-offline targets are rejected at resolve time).
- `mm transaction set` — mutate checkmark / category / comment on an
  existing transaction by numeric id.
- MCP write tools mirroring the CLI verbs, with correct
  `readOnlyHint` / `destructiveHint` annotations per the MCP
  2025-06-18 spec.
- AppleScript-string hardening: reject `"` and newlines in user input
  at the clap / parameter layer, escape `\` during script generation
  (MoneyMoney uses `\` to separate nested category names).
- `MockOsascriptRunner` for golden-string tests of write verbs without
  touching a live MoneyMoney install.
- Release workflow building release archives for
  `{x86_64,aarch64}-unknown-linux-gnu` and
  `{x86_64,aarch64}-apple-darwin` on tag push.
- First public release artifacts at `v0.2.0`.

## [0.1.0] - 2026-04-20

### Added

- Initial scaffold.
- AppleScript dispatcher with `DatabaseLocked` / `NotRunning` /
  `NotInstalled` error classification.
- Domain types for `Account`, `Transaction`, `Category`, `Security`,
  decoded from MoneyMoney's plist output with float-noise-free
  `rust_decimal` conversion.
- Account resolver accepting UUID / IBAN (mod-97 validated via
  `iban_validate`) / account number / alias / `Bank/Name` path / bare
  name, with explicit ambiguity detection.
- Optional TOML config loader at XDG paths for alias definitions.
- Read subcommands: `mm status`, `mm accounts list/get` (with `--tree`
  and `--include-groups`), `mm transactions`, `mm categories`,
  `mm portfolio`, `mm statements list/get`, `mm version`.
- Output module (table / JSON / NDJSON) with field filtering and
  structured JSON errors.
- MCP server (`mm mcp`) over stdio via `rmcp` with read-only tools.
- `stderr`-only tracing subscriber (compact for CLI, JSON for MCP).
- Claude Code plugin metadata (`.claude-plugin/`), PostToolUse skill
  nudge hook, and `bin/mm` shim that defers to a user-installed binary.
- GitHub Actions CI covering fmt, clippy, MSRV, tests (macOS + Linux),
  audit, and rustdoc.
