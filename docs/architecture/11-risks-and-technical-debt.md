# Risks and technical debt

- **No Windows support:** the constraints require the application to work on
  Windows, but the CLI and daemon talk over a unix socket and rely on unix
  signals, so they don't compile for Windows and no Windows release exists
  ([ADR 0001](decisions/0001-release-through-github-releases-and-ghcr.md)).
  Porting needs a cross-platform transport, such as named pipes, and
  microsandbox's Windows support is still in preview.
- **Manual runtime installation:** release users must install the
  microsandbox runtime in the exact version `anvild` was built with. A
  mismatch makes every sandbox start fail. Embedding the runtime with
  microsandbox's `embed-binaries` feature would remove this step.
- **Ageing Linux build runners:** Linux releases build on the `ubuntu-22.04`
  runners to support glibc 2.35. GitHub retires runner images before their
  Ubuntu release reaches end of support (April 2027), after which the
  release jobs fail. Building in an older-glibc container, or with
  `cargo-zigbuild` against a pinned glibc version, would remove this
  dependency.
