# Risks and technical debt

- **No Windows support:** the constraints require the application to work on
  Windows, but the CLI and daemon talk over a unix socket and rely on unix
  signals, so they don't compile for Windows and no Windows release exists
  ([ADR 0001](decisions/0001-release-through-github-releases-and-ghcr.md)).
  Porting needs a cross-platform transport, such as named pipes, and
  microsandbox's Windows support is still in preview.
- **Secrets in plaintext at rest:** the secret values are stored in
  plaintext in `secrets.yml` and in microsandbox's database, readable by
  every process that runs as the user. When the host may be compromised,
  rotate the secrets at their issuers
  ([ADR 0004](decisions/0004-store-secrets-in-a-private-file.md)).
- **Secrets only work in HTTP headers:** Anvil keeps microsandbox's default
  substitution scope, which replaces secret placeholders in headers,
  including decoded Basic auth credentials, but not in URLs or request
  bodies. Tools that send a token elsewhere can't use
  secrets.
- **Unreleased builds have no default image:** the default image is the
  `anvil-base` image tagged with the workspace version, which only exists
  after that version is released. Development builds need an `image` in
  `.anvil.yml`
  ([ADR 0005](decisions/0005-default-to-the-anvil-base-image-of-the-same-release.md)).
- **Workspace ownership is only tested with host UID 1000:** the integration
  tests check that the workspace shows up as `1000:1000` and that UID 1000
  can write to it, but they run on hosts where the user has UID 1000.
  Mapping another host UID, such as `501` on macOS, relies on microsandbox's
  mount owner override
  ([ADR 0006](decisions/0006-run-sandboxes-as-the-agent-user.md)).
- **No SSH agent forwarding:** microsandbox's SSH server rejects agent
  forwarding, so git over SSH needs a private key inside the sandbox. Git
  uses HTTPS with a secret instead.
- **Ageing Linux build runners:** Linux releases build on the `ubuntu-22.04`
  runners to support glibc 2.35. GitHub retires runner images before their
  Ubuntu release reaches end of support (April 2027), after which the
  release jobs fail. Building in an older-glibc container, or with
  `cargo-zigbuild` against a pinned glibc version, would remove this
  dependency.
