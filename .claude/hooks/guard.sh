#!/usr/bin/env bash
# Blocks tool calls that bypass quality gates or touch files we don't own.
# Exit code 2 blocks the call and feeds stderr back to Claude.
set -euo pipefail

input=$(cat)
tool=$(jq -r '.tool_name' <<<"$input")

block() {
  echo "Blocked by .claude/hooks/guard.sh: $1" >&2
  exit 2
}

case "$tool" in
  Edit|Write)
    file=$(jq -r '.tool_input.file_path // empty' <<<"$input")
    rel=${file#"${CLAUDE_PROJECT_DIR:-$PWD}/"}

    case "$rel" in
      third_party/*) block "third_party/ holds vendored submodules; change them upstream." ;;
      dist/*) block "dist/ is build output; run 'task build' instead." ;;
      go.sum|*/go.sum) block "go.sum is generated; run 'go mod tidy' instead." ;;
    esac

    text=$(jq -r '.tool_input.new_string // .tool_input.content // empty' <<<"$input")
    if [[ "$rel" == *.go ]] && grep -q '//nolint' <<<"$text"; then
      block "//nolint suppresses the linter; fix the finding or ask the user first."
    fi
    ;;
  Bash)
    cmd=$(jq -r '.tool_input.command // empty' <<<"$input")

    if grep -qE 'git\b[^|;&]*(--no-verify|\bcommit\b[^|;&]*\s-n\b)' <<<"$cmd"; then
      block "skipping git hooks is not allowed; fix the failing check instead."
    fi
    if grep -qE 'git\b[^|;&]*\bpush\b[^|;&]*(--force|\s-f\b)' <<<"$cmd"; then
      block "force-pushing rewrites shared history; ask the user to do it."
    fi
    ;;
esac
