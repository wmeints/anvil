#!/usr/bin/env bash
# Stop hook: verifies the definition of done before Claude ends its turn.
# Runs only while Rust sources or build files have uncommitted changes, and
# runs the vm-tests only when a crate other than the CLI, the proto or the Cargo
# files changed.
# Exit code 2 blocks the stop and feeds the failing output back to Claude. A
# stop that this hook already blocked once isn't blocked again: a failure Claude
# couldn't fix is reported to the user instead, so the hook can't loop.
set -uo pipefail

active=$(python3 -c 'import json, sys; print(json.load(sys.stdin).get("stop_hook_active", False))' 2>/dev/null)

cd "${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel)}" || exit 0

changed=$(git status --porcelain --untracked-files=all |
  grep -E '\.(rs|proto)$|Cargo\.(toml|lock)$|\.cargo/config\.toml$' || true)
[ -z "$changed" ] && exit 0

# Prints stdin as a message to the user. Exit 0 afterwards so the stop isn't
# blocked; `report` runs in a pipeline, so it can't exit the script itself.
report() {
  python3 -c 'import json, sys; print(json.dumps({"systemMessage": sys.stdin.read()}))'
}

run_check() {
  local name=$1
  shift
  local output
  output=$("$@" 2>&1) && return
  if [ "$active" = True ]; then
    {
      echo "Stop hook: '$name' still fails ($*) after one retry; not blocking again."
      echo "$output" | tail -n 40
    } | report
    exit 0
  fi
  {
    echo "Stop hook: '$name' failed ($*). Fix it before finishing the turn."
    echo "$output" | tail -n 80
  } >&2
  exit 2
}

run_check format cargo fmt --all --check
run_check clippy cargo clippy --workspace --all-targets --all-features -- -D warnings
run_check unit-tests cargo test --workspace

if grep -qE ' (crates/[^/]+/|proto/)' <<<"$(grep -v ' crates/cli/' <<<"$changed")" ||
  grep -qE 'Cargo\.(toml|lock)$|\.cargo/config\.toml$' <<<"$changed"; then
  run_check integration-tests cargo test -p anvil-daemon --features vm-tests
fi

exit 0
