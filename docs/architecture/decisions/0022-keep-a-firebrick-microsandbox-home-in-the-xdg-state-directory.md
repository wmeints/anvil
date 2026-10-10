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
`$XDG_STATE_HOME/firebrick/microsandbox`, or
`~/.local/state/firebrick/microsandbox` when `XDG_STATE_HOME` isn't set, next to
its log files. `firebrick_utils::msb_home()` returns the path.

When `MSB_HOME` is unset or empty, `main` creates that directory and sets
`MSB_HOME` to it before it starts the Tokio runtime or the log writer, while the
process still has a single thread. Setting the variable is `unsafe` in edition
2024 because another thread could read the environment at the same time; doing
it first thing in a synchronous `main` rules that out. microsandbox and every
process it spawns then use the same home.

A non-empty `MSB_HOME` still wins, so the cargo commands in the repository and
the `smoke-test` skill keep their isolated homes.

## Consequences

- An installed `msb` and `fbkd` no longer share a runtime, database or
  sandboxes, so neither can break the other.
- Sandboxes that an earlier `fbkd` created in `~/.microsandbox` stay there and
  no longer show up in `fbk ls`. Users remove them with `msb`.
- Images are pulled again into the new home.
- Running `msb` against firebrick's sandboxes needs
  `MSB_HOME=~/.local/state/firebrick/microsandbox`.
- If microsandbox later passes the backend's home to the processes it spawns,
  `fbkd` can install a default backend instead of setting `MSB_HOME`.
