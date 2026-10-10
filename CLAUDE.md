# Instructions for working on Firebrick

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
- `cargo test -p firebrick-daemon --features vm-tests` - runs the integration
  tests that boot real microsandbox VMs.
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
2. `cargo test -p firebrick-daemon --features vm-tests` passes when a crate
   other than `crates/cli`, `crates/proto`, a `Cargo.toml`, `Cargo.lock` or
   `.cargo/config.toml` changed.
3. The [Architecture Docs](docs/architecture/01-introduction-and-goals.md)
   describe the new behavior, and a decision record exists in
   `docs/architecture/decisions/` for new dependencies or architectural choices.

## Workflows

- Use the `create-issue` skill to file work on GitHub as an issue an agent can
  implement without further questions.
- Use the `fix-bug` skill for bugs: reproduce, find the root cause, write a
  failing test, then fix.
- Use the `smoke-test` skill to run `fbk` and `fbkd` against real VMs, for
  example to reproduce a bug or try a change by hand. It isolates the daemon,
  microsandbox state and SSH config from the user's own.
- Use the `implement-feature` skill for new behavior: agree on a spec first,
  then test, implement and document.
- Use the `sandbox-reference` skill before relying on how the `microsandbox`
  crate behaves. It shows where to find the source of the locked version and
  lists the behavior and limits we already know.
- Use the `submit-pr` skill to open a PR. It runs the checks and asks the
  `reviewer` agent and the `test-reviewer` agent to review the branch, so don't
  run the review agents before each commit.

## Automated checks

These checks enforce the rules above, so don't try to bypass them:

- `mise install` provides the Rust toolchain, `buf`, `protoc`, `dprint`,
  `actionlint` and `lefthook`.
- `.cargo/config.toml` sets `MSB_HOME=/tmp/firebrick-msb` for every cargo
  command, so builds and the `vm-tests` use their own microsandbox runtime,
  database and images instead of `~/.microsandbox`. A `msb` that migrated the
  user's database can't break the tests. Concurrent test runs, for example from
  two worktrees, can share the home: each run names its sandboxes
  `fbk-it-<pid>-<test>` and first removes the leftovers of killed runs. Don't
  point the tests at `~/.microsandbox`; if the isolated home is broken, remove
  `/tmp/firebrick-msb` and run the tests again.
- Claude Code hooks in `.claude/settings.json` run `cargo fmt` on Rust files
  after each edit (`.claude/hooks/format-rust.sh`) and `dprint fmt` on Markdown
  files after each edit (`.claude/hooks/format-markdown.sh`). Before a turn ends
  with uncommitted Rust, proto or Cargo changes, `.claude/hooks/stop-checks.sh`
  runs the format check, clippy and the unit tests, plus the `vm-tests` when the
  files from step 2 of the definition of done changed. A failure blocks the turn
  once; when it still fails on the retry, the hook reports it to the user and
  lets the turn end instead of looping.
- Lefthook runs the format checks for Rust and Markdown, `actionlint` on the
  GitHub workflows, clippy and the unit tests before each commit.
- GitHub Actions (`.github/workflows/ci.yaml`) runs the format checks,
  `actionlint`, build, clippy, unit tests and the `vm-tests` integration tests
  on every pull request and every push to `main`.
- `.github/workflows/image.yaml` builds the `firebrick-base` image for both
  platforms, without pushing it, on pull requests and pushes to `main` that
  change the `Dockerfile`.
