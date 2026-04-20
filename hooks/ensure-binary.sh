#!/usr/bin/env bash
#
# SessionStart hook: keep ${CLAUDE_PLUGIN_DATA}/bin/mm in sync with the
# plugin version declared in .claude-plugin/plugin.json. Runs once per
# Claude Code session and fires-and-forgets so we never block session
# startup on a flaky network.
#
# Any download actually happens via bin/mm's own lazy path, so the
# worst case when this script fails is that the first `mm` invocation
# of the session pays the download cost instead of the session start.

set -euo pipefail

plugin_root="${CLAUDE_PLUGIN_ROOT:-}"
data_dir="${CLAUDE_PLUGIN_DATA:-$HOME/.claude/plugins/data/moneymoney}"
plugin_json="$plugin_root/.claude-plugin/plugin.json"

# Bail out quietly if the plugin isn't fully wired up (e.g., first run
# before the plugin finishes cloning).
if [[ -z "$plugin_root" || ! -f "$plugin_json" ]]; then
  exit 0
fi

# If the user has a real `mm` on PATH (cargo / brew / ...) we don't
# manage a plugin-owned copy — their install wins.
self_dir="$plugin_root/bin"
clean_path="$(echo "$PATH" | tr ':' '\n' | grep -vFx "$self_dir" | tr '\n' ':')"
if PATH="$clean_path" command -v mm >/dev/null 2>&1; then
  exit 0
fi

# Read .version from plugin.json. Prefer jq when available; fall back to a
# pure-bash sed extractor so the hook works on machines without jq.
if command -v jq >/dev/null 2>&1; then
  expected_version="$(jq -r '.version // empty' "$plugin_json" 2>/dev/null || true)"
else
  expected_version="$(
    sed -nE 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p' \
      "$plugin_json" | head -n1
  )"
fi
[[ -z "$expected_version" ]] && exit 0

installed_version_file="$data_dir/bin/mm.version"
installed_version="$(cat "$installed_version_file" 2>/dev/null || echo "")"
if [[ "$installed_version" == "$expected_version" ]]; then
  exit 0
fi

# Delegate the heavy lifting to bin/mm so the download logic lives in
# one place. `mm --version` is harmless and exercises exactly the code
# path we want.
#
# Run detached in the background. Any network failure is logged but
# shouldn't prevent session start; bin/mm will retry lazily on the
# first real invocation.
(
  if "$plugin_root/bin/mm" --version >/dev/null 2>&1; then
    :
  else
    echo "mm: background install of v$expected_version failed; will retry on next invocation" >&2
  fi
) &
disown || true

exit 0
