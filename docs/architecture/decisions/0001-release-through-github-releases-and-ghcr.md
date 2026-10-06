# 1. Release through GitHub releases and GHCR

## Status

Accepted

## Context

Users need prebuilt `anvil` and `anvild` binaries and the `anvil-base`
sandbox image without building them from source. The CLI starts `anvild`
from its own directory, so both binaries must ship together. The code uses
unix sockets and unix signals, and microsandbox only runs on Linux with KVM,
Apple Silicon macOS and, in preview, Windows.

## Decision

A tag matching `v*.*.*` triggers a GitHub Actions workflow that:

- Builds both binaries natively on GitHub-hosted runners for
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and
  `aarch64-apple-darwin`, instead of cross-compiling, so the protobuf and
  microsandbox build scripts run on the platform they target.
- Packages each target as a `tar.gz` with both binaries in one directory.
- Pushes the `anvil-base` image to GitHub Container Registry, tagged with the
  git tag, for `linux/amd64` and `linux/arm64`.
- Creates a GitHub release with generated notes once all of the above pass.

Windows is left out until the CLI and daemon are ported off unix-only APIs.

## Consequences

- A release needs no secrets beyond the workflow's `GITHUB_TOKEN`.
- The release only appears when every package and the image succeed.
- Users install the microsandbox runtime themselves; the archives don't
  bundle it.
- Windows users can't install anvil from a release.
