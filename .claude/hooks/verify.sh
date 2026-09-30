#!/usr/bin/env bash
# Runs lint and unit tests before Claude finishes a turn that changed Go code.
# Exit code 2 prevents stopping and feeds the failures back to Claude.
set -uo pipefail

input=$(cat)

# Claude already got one chance to fix the failures; let it stop and report.
if [[ "$(jq -r '.stop_hook_active' <<<"$input")" == "true" ]]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$PWD}"

changed=$(git diff --name-only HEAD -- '*.go' go.mod .golangci.yml)
untracked=$(git ls-files --others --exclude-standard -- '*.go')

if [[ -z "$changed$untracked" ]]; then
  exit 0
fi

if ! out=$( (task lint && task test) 2>&1 ); then
  echo "Lint or unit tests fail. Fix them before finishing:" >&2
  echo "$out" | tail -n 80 >&2
  exit 2
fi
