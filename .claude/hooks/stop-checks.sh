#!/usr/bin/env bash
# Stop hook: verifies the definition of done before Claude ends its turn.
# Runs only while Rust sources or build files have uncommitted changes.
# Exit code 2 blocks the stop and feeds the failing output back to Claude.
set -uo pipefail

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}" || exit 0

changed=$(git status --porcelain --untracked-files=all |
  grep -E '\.(rs|proto)$|Cargo\.toml$' || true)
[ -z "$changed" ] && exit 0

run_check() {
  local name=$1
  shift
  local output
  if ! output=$("$@" 2>&1); then
    {
      echo "Stop hook: '$name' failed ($*). Fix it before finishing the turn."
      echo "$output" | tail -n 80
    } >&2
    exit 2
  fi
}

run_check format cargo fmt --all --check
run_check clippy cargo clippy --workspace --all-targets --all-features -- -D warnings
run_check unit-tests cargo test --workspace
run_check integration-tests cargo test -p anvil-daemon --features vm-tests

exit 0
