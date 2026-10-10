# 12. Rename Anvil to Firebrick

## Status

Accepted

## Context

The project was called Anvil, with the crates `anvil-cli`, `anvil-daemon`,
`anvil-spec` and `anvil-utils` and the binaries `anvil` and `anvild`. Those
crate names are already taken on crates.io, so the project can't be published
there under its name. The name also shows up in every user-facing path: the
project spec file, the daemon socket, the log and data directories, the SSH host
suffix, the editor config override, the proto package and the base image.

## Decision

Rename the project to Firebrick. The crates become `firebrick-cli`,
`firebrick-daemon`, `firebrick-spec` and `firebrick-utils`, the CLI binary is
`fbk` and the daemon binary is `fbkd`. Every other name follows:
`.firebrick.yml`, `fbkd.sock`, `$XDG_STATE_HOME/firebrick/fbkd.log`,
`$XDG_DATA_HOME/firebrick/`, the `<leaf>.fbk` SSH host, the `firebrick.hostname`
sandbox label, `FIREBRICK_EDITOR_CONFIG_ROOT`, `package firebrick;` and
`ghcr.io/wmeints/firebrick-base`.

This is a clean break: nothing reads the old names as a fallback. The README
explains how to move an existing setup over by hand.

## Consequences

- The crates can be published on crates.io under names that were free on
  2026-10-10.
- The short binary names `fbk` and `fbkd` keep commands as short to type as
  before.
- Existing users have to rename `.anvil.yml`, move `~/.local/share/anvil`, stop
  the old `anvild` and remove the old `*.anvil` entries from their SSH, VS Code
  and Zed config. The daemon doesn't clean up entries it no longer owns.
- No migration code to maintain or test.
- The GitHub repository and the file names of earlier decision records keep the
  old name.
