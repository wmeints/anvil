#!/usr/bin/env bash
# Blocks tool calls that bypass quality gates or touch files we don't own.
# This is a speed bump against mistakes, not a security boundary.
# Exit code 2 blocks the call and feeds stderr back to Claude.
set -euo pipefail

block() {
  echo "Blocked by .claude/hooks/guard.sh: $1" >&2
  exit 2
}

# Fail closed: without jq the call can't be inspected.
command -v jq >/dev/null || block "jq is required for the hooks; run 'mise install'."

input=$(cat)

check_edit() {
  local file rel old new
  file=$(jq -r '.tool_input.file_path // empty' <<<"$input")
  rel=$(realpath -m --relative-to="${CLAUDE_PROJECT_DIR:-$PWD}" "$file")

  case "$rel" in
    third_party/*) block "third_party/ holds vendored submodules; change them upstream." ;;
    dist/*) block "dist/ is build output; run 'task build' instead." ;;
    go.sum|*/go.sum) block "go.sum is generated; run 'go mod tidy' instead." ;;
  esac

  [[ "$rel" == *.go ]] || return 0

  # Only block directives the edit adds, not ones it keeps as context.
  old=$(jq -r '.tool_input.old_string // empty' <<<"$input" | grep -c '//nolint' || true)
  new=$(jq -r '.tool_input.new_string // .tool_input.content // empty' <<<"$input" |
    grep -c '//nolint' || true)
  if (( new > old )); then
    block "//nolint suppresses the linter; fix the finding or ask the user to add it."
  fi
}

# check_git inspects one git invocation with quoted text already removed.
check_git() {
  local git=$1

  if grep -qE '\s--no-v' <<<"$git"; then
    block "skipping git hooks is not allowed; fix the failing check instead."
  fi
  if grep -qE '\scommit\b' <<<"$git" && grep -qE '\s-[a-zA-Z]*n' <<<"$git"; then
    block "'git commit -n' skips git hooks; fix the failing check instead."
  fi
  if grep -qE '\spush\b' <<<"$git" && grep -qE '\s(--force|-[a-zA-Z]*f|\+[^ ])' <<<"$git"; then
    block "force-pushing rewrites shared history; ask the user to do it."
  fi
}

check_bash() {
  local cmd git
  # Drop quoted text, such as commit messages, so it can't trigger the checks.
  cmd=$(jq -r '.tool_input.command // empty' <<<"$input" |
    sed -E "s/\"[^\"]*\"//g; s/'[^']*'//g")

  if grep -qE '(^|\s)LEFTHOOK=(0|false)|core\.hooksPath' <<<"$cmd"; then
    block "disabling git hooks is not allowed; fix the failing check instead."
  fi

  # Each git invocation at the start of a command, up to the next separator.
  while IFS= read -r git; do
    check_git "$git"
  done < <(grep -oE '(^|[;&|(])\s*([A-Za-z_]+=\S*\s+)*git\b[^;&|)]*' <<<"$cmd" || true)
}

case "$(jq -r '.tool_name' <<<"$input")" in
  Edit|Write) check_edit ;;
  Bash) check_bash ;;
esac
