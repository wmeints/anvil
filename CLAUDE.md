# Instructions for working on Anvil

## Purpose of this project

This project builds a sandbox for coding agents. The goal is to keep the agent
safe inside a microVM-based sandbox.

## Technology stack

- Use [Microsandbox](https://docs.microsandbox.dev/getting-started/introduction)
  as the basis for the sandbox.
- Use [Clap](https://docs.rs/clap/latest/clap/) for the CLI implementation.
- Use [Tokio](https://tokio.rs/) for the async runtime
- Use [Tonic](https://github.com/grpc/grpc-rust) for the gRPC implementation.
- Use [Serde_Yaml](https://docs.rs/serde_yaml/latest/serde_yaml/) for parsing
  the yaml configuration.
- Use [Tracing](https://github.com/tokio-rs/tracing) for OpenTelemtry tracing
  data.

## Important commands

- `cargo build` - compiles all crates into the required executables.
- `cargo test --workspace` - runs the unit tests.
- `cargo test -p anvil-daemon --features vm-tests` - runs the integration tests
  that boot real microsandbox VMs.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` - runs
  the linter, including the `vm-tests` integration tests.
- `cargo fmt --all` - formats the code; `cargo fmt --all --check` verifies it.
- `dprint fmt` - formats the Markdown files to 80 columns with aligned tables;
  `dprint check` verifies them.

`.cargo/config.toml` defines shortcuts for these: `cargo lint`, `cargo
unit-tests`, `cargo integration-tests`, and `cargo install-cli` / `cargo
install-daemon` to install the binaries into `~/.cargo/bin`.

## Coding guidelines

Prefer deep modules with narrow interfaces for structuring the code. Each module
should have tests focusing the public interface.

Before implementing anything make sure you understand the problem. Perform a
root cause analysis for bugs and ensure you have a thorough spec for new
features.

Follow the implementation ladder to prevent over-engineering:

1. Does it have to be built. No? Don't do it.
2. Does it already exist in the codebase? Reuse it.
3. Does the standard library do it? Use it.
4. Does a project dependency provide it? Use the dependency.
5. Can this be done with one line? Write the one-liner.
6. Only then, implement the minimum amount of logic required.

Clippy runs with `-D warnings`, so every warning fails the build. Write code
that fits from the start:

- Keep functions small and shallow. Split logic into small functions early
  instead of nesting `if`/`match` blocks deeply.
- Let `rustfmt` decide the layout; don't format code by hand.
- Document every public item and every crate with `///` and `//!` comments.
- Use `thiserror` for error types in library code and `anyhow` in binaries.
  Propagate errors with `?` and add context instead of discarding the source.
- Don't `unwrap()` or `expect()` outside tests unless the invariant is
  documented next to the call. Bound integer conversions with `try_from` instead
  of `as`.
- Don't suppress lints with `#[allow(...)]`; fix the code. If a finding is a
  false positive, ask the user before adding an `allow`.

## Testing

- Unit tests live next to the code in a `#[cfg(test)] mod tests` block at the
  bottom of the module and test its public interface.
- Tests that boot real microsandbox VMs live in the crate's `tests/` directory
  and are registered in `Cargo.toml` as a `[[test]]` with `required-features =
  ["vm-tests"]`, so `cargo test` skips them by default.
- Use `#[tokio::test]` for async tests, and give temporary files, sockets and
  sandboxes unique names (for example with `tempfile` or the process id) so
  tests can run in parallel.
- Fix a bug by first writing a test that reproduces it.

## Definition of done

A change is done when:

1. `cargo fmt --all --check`, `dprint check`, `cargo clippy --workspace
   --all-targets --all-features -- -D warnings` and `cargo test --workspace`
   pass.
2. `cargo test -p anvil-daemon --features vm-tests` passes when `crates/daemon`
   changed.
3. The [Architecture Docs](docs/architecture/README.md) describe the new
   behavior, and a decision record exists in `docs/architecture/decisions/` for
   new dependencies or architectural choices.

## Workflows

- Use the `create-issue` skill to file work on GitHub as an issue an agent can
  implement without further questions.
- Use the `fix-bug` skill for bugs: reproduce, find the root cause, write a
  failing test, then fix.
- Use the `smoke-test` skill to run `anvil` and `anvild` against real VMs, for
  example to reproduce a bug or try a change by hand. It isolates the daemon,
  microsandbox state and SSH config from the user's own.
- Use the `implement-feature` skill for new behavior: agree on a spec first,
  then test, implement and document.
- Use the `sandbox-reference` skill before relying on how the `microsandbox`
  crate behaves. It shows where to find the source of the locked version and
  lists the behavior and limits we already know.
- Use the `submit-pr` skill to open a PR. It runs the checks and asks the
  `reviewer` agent to review the branch, so don't run the reviewer before each
  commit.

## Automated checks

These checks enforce the rules above, so don't try to bypass them:

- `mise install` provides the Rust toolchain, `buf`, `dprint` and `lefthook`.
- Claude Code hooks in `.claude/settings.json` run `cargo fmt` on Rust files
  after each edit (`.claude/hooks/format-rust.sh`), run `dprint fmt` on Markdown
  files after each edit (`.claude/hooks/format-markdown.sh`), and run the format
  check, clippy, the unit tests and the `vm-tests` integration tests before a
  turn ends while Rust, proto or `Cargo.toml` files have uncommitted changes
  (`.claude/hooks/stop-checks.sh`).
- Lefthook runs the format checks for Rust and Markdown, clippy and the unit
  tests before each commit and the `vm-tests` integration tests before each
  push.
- GitHub Actions (`.github/workflows/ci.yaml`) runs the format checks, build,
  clippy, unit tests and the `vm-tests` integration tests on every pull request
  and every push to `main`.
