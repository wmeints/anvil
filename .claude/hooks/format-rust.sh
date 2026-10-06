#!/usr/bin/env bash
# PostToolUse hook: formats a Rust file with rustfmt after Claude edits it.
# Uses the workspace's rustfmt settings via `cargo fmt`. Formatting failures
# (usually syntax errors) are reported back to Claude with exit code 2.
set -uo pipefail

file=$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("tool_input", {}).get("file_path", ""))')
[[ "$file" == *.rs && -f "$file" ]] || exit 0

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}" || exit 0

if ! output=$(cargo fmt -- "$file" 2>&1); then
  {
    echo "Format hook: cargo fmt failed for $file."
    echo "$output" | tail -n 40
  } >&2
  exit 2
fi
