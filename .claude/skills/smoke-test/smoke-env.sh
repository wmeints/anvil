#!/usr/bin/env bash
# Creates and removes an isolated environment for running `fbk` and `fbkd`
# end to end without touching the user's daemon, sandboxes or config.
#
#   smoke-env.sh up      creates the environment and prints its env file
#   smoke-env.sh down F  stops its sandboxes and daemon and deletes it
#
# Source the env file in every shell command that runs `fbk`.
set -uo pipefail

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)

up() {
  # A short path: unix socket paths are limited to 108 bytes, and the
  # scratchpad path is too long for the sockets fbkd and microsandbox create.
  local root
  root=$(mktemp -d /tmp/firebrick-smoke.XXXX) || exit 1
  mkdir -p "$root"/{run,msb,data,state,config,home,work}
  chmod 700 "$root/run"

  cat >"$root/env" <<EOF
export SMOKE_ROOT="$root"
export XDG_RUNTIME_DIR="$root/run"
export MSB_HOME="$root/msb"
export XDG_DATA_HOME="$root/data"
export XDG_STATE_HOME="$root/state"
export XDG_CONFIG_HOME="$root/config"
export HOME="$root/home"
export SMOKE_WORK="$root/work"
export FIREBRICK="$repo/target/debug/fbk"
export MSB="$root/msb/bin/msb"
export SMOKE_SSH_CONFIG="$root/data/firebrick/ssh/config"
EOF
  echo "$root/env"
}

# Prints the names of the sandboxes the isolated daemon knows about.
sandbox_names() {
  "$FIREBRICK" ls --format json 2>/dev/null | grep -o '"name": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/'
}

down() {
  local env_file=${1:?usage: smoke-env.sh down <env file>}
  [[ "$env_file" == /tmp/firebrick-smoke.*/env ]] || {
    echo "refusing to clean up $env_file: not a smoke-test environment" >&2
    exit 1
  }
  # shellcheck source=/dev/null
  source "$env_file"

  if [ -S "$XDG_RUNTIME_DIR/fbkd.sock" ]; then
    for name in $(sandbox_names); do
      "$FIREBRICK" stop "$name" >/dev/null 2>&1
      "$FIREBRICK" rm "$name" >/dev/null 2>&1 || echo "couldn't remove sandbox $name" >&2
    done
    # Only the daemon listening on this environment's socket.
    for pid in $(lsof -t "$XDG_RUNTIME_DIR/fbkd.sock" 2>/dev/null); do
      kill -TERM "$pid"
    done
    sleep 1
  fi

  rm -rf "$SMOKE_ROOT"
  echo "removed $SMOKE_ROOT"
}

case "${1:-}" in
  up) up ;;
  down) down "${2:-}" ;;
  *)
    echo "usage: smoke-env.sh up | down <env file>" >&2
    exit 2
    ;;
esac
