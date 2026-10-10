#!/usr/bin/env bash
# Propose the next 0.x release from the conventional commits since the last
# release tag. Prints the last tag, the bump, the next version and the reason.
#
# Usage: next-version.sh [ref]   (default: origin/main)
#
# Rules for 0.x versions: a breaking change (`type!:` or `BREAKING CHANGE`) or a
# `feat` commit bumps the minor version (0.3.1 -> 0.4.0); anything else bumps
# the patch version (0.3.0 -> 0.3.1). Exits 1 when there is nothing to release.
set -euo pipefail

ref=${1:-origin/main}
last=$(git describe --tags --abbrev=0 --match 'v[0-9]*.[0-9]*.[0-9]*' --exclude '*-*' "$ref")
log=$(git log --no-merges --format='%s%n%b%n--' "$last..$ref")
subjects=$(git log --no-merges --format='%s' "$last..$ref")

if [ -z "$subjects" ]; then
  echo "No commits since $last, nothing to release" >&2
  exit 1
fi

IFS=. read -r major minor patch <<< "${last#v}"
if [ "$major" != 0 ]; then
  echo "$last is past 0.x; decide the bump by hand" >&2
  exit 1
fi

breaking=$(grep -cE '^[a-z]+(\([^)]*\))?!:|^BREAKING[ -]CHANGE' <<< "$log" || true)
features=$(grep -cE '^feat(\([^)]*\))?:' <<< "$subjects" || true)

if [ "$breaking" -gt 0 ] || [ "$features" -gt 0 ]; then
  bump=minor
  next="0.$((minor + 1)).0"
else
  bump=patch
  next="0.$minor.$((patch + 1))"
fi

echo "last=$last"
echo "bump=$bump"
echo "next=v$next"
echo "reason=$breaking breaking, $features feat, $(wc -l <<< "$subjects") commits since $last"
