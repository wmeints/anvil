# 2. Embed the microsandbox runtime in anvild

## Status

Accepted

## Context

`anvild` needs the microsandbox runtime (`msb` and `libkrunfw`) in the
version that matches the `microsandbox` crate it's built with. Until now,
users installed it themselves with the microsandbox installer before running
anvil ([ADR 0001](0001-release-through-github-releases-and-ghcr.md)). A
missing or mismatched runtime only showed up when the first sandbox failed to
start.

The `microsandbox` crate offers `setup::ensure_runtime`, which resolves an
installed runtime and installs one when it's absent. It can install from a
release download at run time, or from an archive embedded through the
`embed-binaries` feature.

The CLI spawns `anvild` and waits at most 5 seconds for its socket, so
anything `anvild` does before it listens must be quick.

## Decision

`anvild` enables the `embed-binaries` feature of `microsandbox`. On startup,
before it listens on its socket, it calls `setup::ensure_runtime` with
`InstallSource::EmbeddedArchive`:

- A missing runtime is extracted from the embedded archive into the
  microsandbox home (`~/.microsandbox` or `$MSB_HOME`).
- `anvild` reads the installed `msb` version with
  `setup::resolve_runtime_version`, without running the binary. A runtime
  in the microsandbox home with another version, or without version
  information, is replaced by the embedded one through `install_runtime`
  with `force: true`.
- A runtime the user configured explicitly (`MSB_PATH`, `paths.msb`) is
  never replaced; when its version differs, `anvild` refuses to start.
- A partial or invalid runtime stops `anvild` with an error.

We don't download the runtime at run time: that needs network access on
first start and would exceed the CLI's 5 second wait.

## Consequences

- Users don't install the microsandbox runtime themselves, and the installed
  runtime matches the crate version on a fresh machine.
- The runtime archive (about 28 MB) is downloaded at build time by the
  `microsandbox` build script, for the compile target, and makes `anvild`
  correspondingly larger. Offline builds can supply the archive through
  `MSB_EMBED_RUNTIME_BUNDLE_PATH` or `MSB_EMBED_ARTIFACTS_DIR`.
- Extracting the runtime takes well under a second, so the CLI's start
  timeout stays at 5 seconds.
- Upgrading anvil to a new `microsandbox` version also upgrades the runtime
  in the microsandbox home on the next daemon start. Other microsandbox
  tools that share that home get the runtime version anvil needs.
- Sandboxes that run during an upgrade keep their old `msb` process; only
  sandboxes started afterwards use the new runtime.
