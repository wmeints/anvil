# 22. Keep a firebrick microsandbox home in the XDG state directory

## Status

Accepted

## Context

microsandbox keeps its runtime (`bin/msb`, `libkrunfw`), database (`db/msb.db`),
`config.json`, images and sandboxes in `$MSB_HOME`, or in `~/.microsandbox` when
`MSB_HOME` isn't set. A separately installed `msb` CLI or another microsandbox
SDK uses the same directory. Sharing it breaks both sides:

- A newer `msb` migrates `db/msb.db` in place, and every `fbkd` call then fails
  with "database schema is newer than this msb binary".
- `fbkd` replaces a runtime in the home whose version differs from the one it
  embeds (ADR 0002), so it overwrites the user's own `msb`.

microsandbox 0.7.7 can point the SDK at another home with
`LocalBackend::builder().home(..)` and `set_default_backend`. That only changes
the home of the `fbkd` process. The `msb` VM processes that microsandbox spawns
get no home on their command line: they inherit the environment and resolve
paths such as the TLS interception CA (`tls/`) from `MSB_HOME`, so they would
still write `~/.microsandbox`.

## Decision

`fbkd` keeps microsandbox's state in its own home,
`$XDG_STATE_HOME/firebrick/msb`, or `~/.local/state/firebrick/msb` when
`XDG_STATE_HOME` isn't set, next to its log files. `firebrick_utils::msb_home()`
returns the path.

When `MSB_HOME` is unset or empty, `main` creates that directory and sets
`MSB_HOME` to it before it starts the Tokio runtime or the log writer, while the
process still has a single thread. Setting the variable is `unsafe` in edition
2024 because another thread could read the environment at the same time; doing
it first thing in a synchronous `main` rules that out. microsandbox and every
process it spawns then use the same home.

`msb_home()` ignores empty and relative values of `XDG_STATE_HOME` and `HOME`,
as the XDG Base Directory spec requires, and has no temp-directory fallback:
when neither is an absolute path and `MSB_HOME` isn't set, `fbkd` refuses to
start. The home holds the `msb` binary that `fbkd` runs on the host, so a
relative home in the current directory (often a workspace mounted read/write
into a sandbox) or a predictable path in `/tmp` would let someone else replace
it. `fbkd` creates the home with mode `0700`.

The subdirectory is `msb` rather than `microsandbox` to keep unix socket paths
short; see the consequences.

A non-empty `MSB_HOME` still wins, so the cargo commands in the repository and
the `smoke-test` skill keep their isolated homes.

## Consequences

- An installed `msb` and `fbkd` no longer share a runtime, database or
  sandboxes, so neither can break the other.
- Sandboxes that an earlier `fbkd` created in `~/.microsandbox` stay there and
  no longer show up in `fbk ls`. Users remove them with `msb`.
- Images are pulled again into the new home.
- Running `msb` against firebrick's sandboxes needs
  `MSB_HOME=~/.local/state/firebrick/msb`.
- microsandbox creates its unix sockets under `<home>/run`, and their paths
  can't be longer than 107 bytes on Linux or 103 on macOS. The current runtime's
  longest one, `run/sandboxes/<24 hex>/control.sock`, adds 52 bytes to the home,
  independent of the sandbox name. With `~/.local/state/firebrick/msb` that fits
  user names up to 22 characters on Linux (`/home/<user>`) and 17 on macOS
  (`/Users/<user>`). A longer home, for example from a deep `XDG_STATE_HOME`,
  fails with "sandbox runtime socket path is too long"; set `MSB_HOME` to a
  shorter directory.
- If microsandbox later passes the backend's home to the processes it spawns,
  `fbkd` can install a default backend instead of setting `MSB_HOME`.
