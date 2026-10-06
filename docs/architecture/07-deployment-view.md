# Deployment view

## Release artifacts

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yaml`, which
publishes:

- An `anvil-<tag>-<target>.tar.gz` archive per target, with a `.sha256`
  checksum, attached to a GitHub release with generated release notes. Each
  archive holds one directory with the `anvil` and `anvild` binaries side by
  side, because the CLI starts the daemon from its own directory.
- The `ghcr.io/wmeints/anvil-base:<tag>` image, built from the `Dockerfile`
  for `linux/amd64` and `linux/arm64`.

Tags with a suffix, such as `v0.2.0-rc.1`, publish a pre-release.

| Target                      | Runner             |
| --------------------------- | ------------------ |
| `x86_64-unknown-linux-gnu`  | `ubuntu-latest`    |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `aarch64-apple-darwin`      | `macos-latest`     |

Windows has no release: the CLI and daemon talk over a unix socket and rely
on unix signals. See
[ADR 0001](decisions/0001-release-through-github-releases-and-ghcr.md).

## Host requirements

`anvild` runs sandboxes through the microsandbox runtime (`msb` and
`libkrunfw`), which needs KVM on Linux or Apple Silicon on macOS. The
release archives don't contain this runtime; it must be installed in
`~/.microsandbox` on the host.
