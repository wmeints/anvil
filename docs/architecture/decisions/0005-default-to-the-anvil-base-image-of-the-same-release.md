# 5. Default to the firebrick-base image of the same release

## Status

Accepted

## Context

A sandbox whose `.firebrick.yml` doesn't name an `image` ran `ubuntu:26.04`,
which runs everything as `root`. The `firebrick-base` image runs as the
unprivileged `agent` user and adds the tools agents need, so it's a safer
default. The release workflow publishes it as
`ghcr.io/wmeints/firebrick-base:<tag>` and doesn't push a `latest` tag.

We considered three tags for the default:

- **`latest`** - always the newest image, but the release workflow doesn't
  publish it, and an older `fbk` would pick up an image it was never tested
  with.
- **A fixed tag in the code** - has to be bumped by hand for every release.
- **The workspace version** - `v` plus `CARGO_PKG_VERSION`, which matches the
  tag the release is built from.

## Decision

`firebrick-spec` sets `DEFAULT_IMAGE` to
`ghcr.io/wmeints/firebrick-base:v<CARGO_PKG_VERSION>`. Each release of `fbk` and
`fbkd` runs the image published by the same release.

## Consequences

- Releasing doesn't need a separate change to the default image, as long as the
  tag matches the workspace version in `Cargo.toml`. The release workflow fails
  when they differ.
- A build of a version that has no release yet can't pull the default image.
  Until the first release, and after bumping the version, set `image` in
  `.firebrick.yml`.
- The `firebrick-base` package must be public, see
  [ADR 0001](0001-release-through-github-releases-and-ghcr.md).
- The registry owner `wmeints` is fixed in the code. Forks that publish their
  own image have to change `DEFAULT_IMAGE`.
- Existing sandboxes keep the image they were created with.
- Sandboxes run as the `agent` user, see
  [ADR 0006](0006-run-sandboxes-as-the-agent-user.md).
