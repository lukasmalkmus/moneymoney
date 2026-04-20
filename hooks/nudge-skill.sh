#!/usr/bin/env bash
input=$(cat)
command=$(echo "$input" | jq -r '.tool_input.command // empty' 2>/dev/null)
if [[ -z "$command" ]] || ! echo "$command" | grep -qE '(^|[^a-zA-Z0-9-])mm($|[[:space:]])'; then
  exit 0
fi
session_id=$(echo "$input" | jq -r '.session_id // empty' 2>/dev/null)
marker="${TMPDIR:-/tmp}/.moneymoney-skill-nudge-${session_id:-$PPID}"
[ -f "$marker" ] && exit 0
touch "$marker"
nudge='<system-reminder>The "moneymoney" skill provides guided mm workflows. Invoke it with /moneymoney or the Skill tool.</system-reminder>'
jq -n --arg nudge "$nudge" '{
  hookSpecificOutput: {
    hookEventName: "PostToolUse",
    additionalContext: $nudge
  }
}'
