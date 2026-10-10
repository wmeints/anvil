# 13. Share the gRPC code through a proto crate

## Status

Accepted

## Context

The CLI and daemon each compiled `proto/daemon.v1.proto` from the workspace root
in their own `build.rs`. `cargo publish` only packages the files inside a
crate's folder, so the packaged `firebrick-cli` couldn't find the proto file and
failed to build. Publishing to crates.io also requires every dependency to have
a version, not only a path.

## Decision

The proto file moves to `crates/proto/proto/daemon.v1.proto`, inside a new
`firebrick-proto` crate. Its `build.rs` generates the client and the server code
with `tonic-prost-build`, and both executables depend on it and re-export it as
their `api` module. The internal crates are declared in
`[workspace.dependencies]` with both a path and a version.

## Consequences

- `firebrick-proto`, `firebrick-spec`, `firebrick-utils` and `firebrick-cli` can
  be published, in that order, for example with one `cargo publish -p ...`
  command for all of them.
- The proto file is compiled once instead of twice per build.
- The CLI also compiles the generated server code, which it doesn't use. The
  server code needs no extra dependencies, because tonic's default features
  already include the server transport.
- Building `firebrick-proto` from crates.io, for example with `cargo install
  firebrick-cli`, needs `protoc` 3.15 or newer, which supports proto3 optional
  fields. Older distributions ship an older `protoc`, such as 3.12 on Ubuntu
  22.04.
- The internal crates' versions in `[workspace.dependencies]` have to be bumped
  together with `[workspace.package]`.
