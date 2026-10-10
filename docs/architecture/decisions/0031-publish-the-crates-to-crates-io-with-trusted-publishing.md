# 31. Publish the crates to crates.io with Trusted Publishing

## Status

Accepted

## Context

The release workflow of
[ADR 0001](0001-release-through-github-releases-and-ghcr.md) ships prebuilt
archives on GitHub releases, which users download and unpack by hand. Users with
a Rust toolchain would rather run `cargo install`, which needs the crates on
crates.io. Version 0.4.0 was published there by hand; doing that for every
release is easy to forget and needs an account with publish rights.

Publishing from a workflow needs a crates.io credential. A long-lived API token
stored as a repository secret can be leaked and must be rotated, and it breaks
ADR 0001's rule that a release needs no secrets beyond the `GITHUB_TOKEN`.

## Decision

`release.yaml` gets a `publish` job that publishes the five workspace crates to
crates.io with the tag's version:

- It runs after the `package` and `image` jobs, and the `release` job that
  creates the GitHub release needs it, so the GitHub release only appears once
  crates.io has the version.
- It authenticates with crates.io
  [Trusted Publishing](https://crates.io/docs/trusted-publishing) through
  `rust-lang/crates-io-auth-action`, which exchanges the job's GitHub OIDC token
  for a short-lived crates.io token. The job needs `id-token: write`, and
  `tag-release.yaml` grants it to the job that calls `release.yaml`.
- `cargo publish` publishes the crates in dependency order. Cargo refuses to
  publish a version that exists, so the job asks the crates.io API per crate and
  leaves out the crates that have the version already. Rerunning the job after a
  partial publish publishes only the rest.

## Consequences

- A release still needs no stored secret: the crates.io token lives only for the
  job.
- Users can install both binaries with `cargo install firebrick-cli
  firebrick-daemon`, which needs a Rust toolchain and `protoc`. Both land in
  `~/.cargo/bin`, side by side as the CLI requires.
- crates.io matches each crate's configured workflow against the OIDC token's
  `workflow_ref`, which names the top-level workflow: a release from a merged
  version bump authenticates as `tag-release.yaml`, a tag pushed by hand as
  `release.yaml`. Each crate's Trusted Publishing configuration must list the
  workflow a release runs through.
- A published version can't be replaced, only yanked. A failure after `publish`,
  in the `release` job, leaves the version on crates.io without a GitHub
  release; rerunning the failed jobs completes it.
- Pre-release tags publish their pre-release version to crates.io as well.
