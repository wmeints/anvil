# Deployment view

## Release artifacts

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yaml`, which
publishes:

- An `firebrick-<tag>-<target>.tar.gz` archive per target, with a `.sha256`
  checksum, attached to a GitHub release with generated release notes. Each
  archive holds one directory with the `fbk` and `fbkd` binaries side by side,
  because the CLI starts the daemon from its own directory.
- The `ghcr.io/wmeints/firebrick-base:<tag>` image, built from the `Dockerfile`
  for `linux/amd64` and `linux/arm64`. The image is pushed only after every
  archive has built. GHCR makes a package private when it's first published, so
  make `firebrick-base` public once in its package settings before sandboxes can
  pull it without logging in.

Tags with a suffix, such as `v0.2.0-rc.1`, publish a pre-release.

| Target                      | Runner             |
| --------------------------- | ------------------ |
| `x86_64-unknown-linux-gnu`  | `ubuntu-22.04`     |
| `aarch64-unknown-linux-gnu` | `ubuntu-22.04-arm` |
| `aarch64-apple-darwin`      | `macos-latest`     |

The Linux binaries are built on Ubuntu 22.04, so they need glibc 2.35 or newer.

Windows has no release: the CLI and daemon talk over a unix socket and rely on
unix signals. See
[ADR 0001](decisions/0001-release-through-github-releases-and-ghcr.md).

## Host requirements

`fbkd` runs sandboxes through the microsandbox runtime (`msb` and `libkrunfw`),
which needs KVM on Linux or Apple Silicon on macOS. `fbkd` embeds the runtime
that matches the `microsandbox` crate it's built with (currently `0.7.6`). On
startup it extracts that runtime when none is installed, and replaces an
installed runtime with another version. See
[ADR 0002](decisions/0002-embed-the-microsandbox-runtime-in-anvild.md).

- The runtime lives in `~/.microsandbox`, or in the directory that the
  `MSB_HOME` environment variable points to. `MSB_PATH` and `MSB_LIBKRUNFW_PATH`
  point `fbkd` at a runtime elsewhere.
- `fbkd` reads the version from the `msb` binary without running it. A runtime
  without version information predates it and counts as outdated, and so does an
  `msb` in the microsandbox home whose version can't be read.
- A runtime that `MSB_PATH` or `paths.msb` points to is never replaced. When its
  version differs from the embedded one, `fbkd` refuses to start.
- A partial runtime (one of the two files missing) stops `fbkd` from starting.
  Remove both files to let `fbkd` reinstall the runtime.

The macOS binaries aren't signed. When the archive is downloaded through a
browser, macOS quarantines them; remove the quarantine with `xattr -d
com.apple.quarantine fbk fbkd`.
