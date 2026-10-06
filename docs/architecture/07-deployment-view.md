# Deployment view

## Release artifacts

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yaml`, which
publishes:

- An `anvil-<tag>-<target>.tar.gz` archive per target, with a `.sha256`
  checksum, attached to a GitHub release with generated release notes. Each
  archive holds one directory with the `anvil` and `anvild` binaries side by
  side, because the CLI starts the daemon from its own directory.
- The `ghcr.io/wmeints/anvil-base:<tag>` image, built from the `Dockerfile`
  for `linux/amd64` and `linux/arm64`. The image is pushed only after every
  archive has built. GHCR makes a package private when it's first published,
  so make `anvil-base` public once in its package settings before sandboxes
  can pull it without logging in.

Tags with a suffix, such as `v0.2.0-rc.1`, publish a pre-release.

| Target                      | Runner             |
| --------------------------- | ------------------ |
| `x86_64-unknown-linux-gnu`  | `ubuntu-22.04`     |
| `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` |
| `aarch64-apple-darwin`      | `macos-latest`     |

The Linux binaries are built on Ubuntu 22.04, so they need glibc 2.35 or
newer.

Windows has no release: the CLI and daemon talk over a unix socket and rely
on unix signals. See
[ADR 0001](decisions/0001-release-through-github-releases-and-ghcr.md).

## Host requirements

`anvild` runs sandboxes through the microsandbox runtime (`msb` and
`libkrunfw`), which needs KVM on Linux or Apple Silicon on macOS. The
release archives don't contain this runtime, and `anvild` doesn't install
it. Install it with the
[microsandbox installer](https://docs.microsandbox.dev/getting-started/quickstart)
before running anvil:

- The runtime version must match the `microsandbox` crate that `anvild` is
  built with (currently `0.7.6`); `anvild` refuses runtime versions it
  hasn't been tested against.
- The runtime lives in `~/.microsandbox`, or in the directory that the
  `MSB_HOME` environment variable points to.

The macOS binaries aren't signed. When the archive is downloaded through a
browser, macOS quarantines them; remove the quarantine with
`xattr -d com.apple.quarantine anvil anvild`.
