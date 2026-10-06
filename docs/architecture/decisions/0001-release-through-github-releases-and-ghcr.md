# 1. Release through GitHub releases and GHCR

## Status

Accepted

## Context

Users need prebuilt `anvil` and `anvild` binaries and the `anvil-base`
sandbox image without building them from source. The CLI starts `anvild`
from its own directory, so both binaries must ship together. The code uses
unix sockets and unix signals, and microsandbox only runs on Linux with KVM,
Apple Silicon macOS and, in preview, Windows. This conflicts with the
constraint that the application must work on Mac, Linux and Windows
([Constraints](../02-constraints.md)).

## Decision

A tag matching `v*.*.*` triggers a GitHub Actions workflow that:

- Builds both binaries natively on GitHub-hosted runners for
  `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` and
  `aarch64-apple-darwin`, instead of cross-compiling, so the protobuf and
  microsandbox build scripts run on the platform they target. The Linux
  builds run on Ubuntu 22.04 so the binaries need glibc 2.35 at most.
- Packages each target as a `tar.gz` with both binaries in one directory.
- Pushes the `anvil-base` image to GitHub Container Registry, tagged with the
  git tag, for `linux/amd64` and `linux/arm64`, after every package has
  built.
- Creates a GitHub release with generated notes once all of the above pass.

Windows is left out until the CLI and daemon are ported off unix-only APIs.

## Consequences

- A release needs no secrets beyond the workflow's `GITHUB_TOKEN`.
- The release only appears when every package and the image succeed, and
  the image is only pushed when every package succeeds. An image push can
  still succeed while the release step fails.
- Users install the microsandbox runtime themselves, in the version that
  matches the `microsandbox` crate; the archives don't bundle it. Superseded
  by [ADR 0002](0002-embed-the-microsandbox-runtime-in-anvild.md): `anvild`
  now embeds the runtime.
- The `anvil-base` package must be made public once after its first push.
- Windows users can't install anvil from a release, which leaves the
  Windows constraint unmet (see
  [Risks and technical debt](../11-risks-and-technical-debt.md)).
