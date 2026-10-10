#!/usr/bin/env bash
# Set the workspace version and the versions of the workspace's own crates in
# Cargo.toml to the given version, and update Cargo.lock to match.
#
# Usage: bump-version.sh 0.4.0
set -euo pipefail

next=${1:?usage: bump-version.sh <version without v>}
current() {
  cargo metadata --no-deps --format-version 1 |
    jq -r '[.packages[].version] | unique | join(" ")'
}

old=$(current)
# Only the workspace version and the path dependencies on firebrick-* crates.
sed -i -E "/^(version|firebrick-[a-z]+) = /s/\"$old\"/\"$next\"/" Cargo.toml
cargo update --workspace --quiet

if [ "$(current)" != "$next" ]; then
  echo "Cargo.toml still has versions $(current), expected $next" >&2
  exit 1
fi
grep -c "\"$next\"" Cargo.toml | xargs echo "Bumped $old -> $next in Cargo.toml lines:"
