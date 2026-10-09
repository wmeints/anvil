#!/usr/bin/env bash
# PostToolUse hook: formats a Markdown file with dprint after Claude edits it.
# Uses the settings in dprint.json. Formatting failures are reported back to
# Claude with exit code 2.
set -uo pipefail

file=$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("tool_input", {}).get("file_path", ""))')
[[ "$file" == *.md && -f "$file" ]] || exit 0

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}" || exit 0

# Files outside the project, such as plans in ~/.claude, aren't ours to format.
[[ "$(realpath "$file")" == "$PWD"/* ]] || exit 0

if ! output=$(dprint fmt -- "$file" 2>&1); then
  {
    echo "Format hook: dprint fmt failed for $file."
    echo "$output" | tail -n 40
  } >&2
  exit 2
fi
