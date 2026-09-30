#!/usr/bin/env bash
# Formats a Go file right after Claude edits it, so style never reaches the linter.
set -euo pipefail

file=$(jq -r '.tool_input.file_path // empty')

if [[ "$file" == *.go && -f "$file" ]]; then
  gofmt -w "$file"
fi
