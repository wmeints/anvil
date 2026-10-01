# Instructions for working on Anvil

## Purpose of this project

This project builds a sandbox for coding agents. The goal is to keep the agent 
safe inside a microVM-based sandbox. 

## Technology stack

- Use [Nerdbox](https://github.com/containerd/nerdbox) as the basis for the sandbox. 
- Use [Kong](https://github.com/alecthomas/kong) for the CLI implementation.
- Use [Bubbletea](https://github.com/charmbracelet/bubbletea) for the TUI

## Important commands

- `task build` - compiles the sources into the final executable
- `task test` - runs the unit-tests in the project with the race detector
- `task test:integration` - runs the integration tests (requires containerd)
- `task lint` - verifies the code quality in the source files
- `task format` - formats the source files so the linter passes

## Coding guidelines

Prefer deep modules with narrow interfaces for structuring the code. Each 
module should have tests focusing the public interface.

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

Follow the [Engineering guidelines](docs/engineering/README.md), such as the
rules for creating and wrapping errors.

The linter is strict, so write code that fits from the start:

- Cognitive complexity per function is at most 5 (`gocognit`) and nested `if`
  blocks are limited (`nestif`). Split logic into small functions early.
- Lines are at most 88 characters (`lll`).
- Every exported identifier and every package needs a doc comment (`revive`).
- `gosec` and `errorlint` are enabled: bound integer conversions, compare
  errors with `errors.Is`/`errors.As`, and wrap them with `%w`.
- Don't suppress findings with `nolint` directives; fix the code. A hook
  blocks new directives, so if a finding is a false positive, ask the user to
  add the directive.

## Testing

- Unit tests live next to the code in `<name>_test.go` and test the public
  interface of the package.
- Tests that need containerd live in `<name>_integration_test.go` and start
  with the `//go:build integration` constraint.
- Fix a bug by first writing a test that reproduces it.

## Definition of done

A change is done when:

1. `task format`, `task lint` and `task test` pass.
2. `task test:integration` passes when `internal/sandbox` or `internal/daemon`
   changed.
3. The [Architecture Docs](docs/architecture/README.md) describe the new
   behavior, and a decision record exists in `docs/architecture/decisions/`
   for new dependencies or architectural choices.

## Workflows

- Use the `create-issue` skill to file work on GitHub as an issue an agent
  can implement without further questions.
- Use the `fix-bug` skill for bugs: reproduce, find the root cause, write a
  failing test, then fix.
- Use the `implement-feature` skill for new behavior: agree on a spec first,
  then test, implement and document.
- Ask the `reviewer` agent to review a change before committing it. The
  `submit-pr` skill runs the checks and the reviewer before opening a PR.

## Automated checks

These checks enforce the rules above, so don't try to bypass them:

- Claude Code hooks in `.claude/settings.json` format Go files after each
  edit, block edits to `third_party/`, `dist/` and `go.sum`, block skipping git
  hooks and force-pushing, and run `task lint` and `task test` before a turn
  ends while Go files have uncommitted changes. The hooks need `jq`, which
  `mise install` provides. The guard is a speed bump against mistakes, not a
  security boundary.
- Lefthook runs the format check, lint and unit tests before each commit and
  the integration tests before each push.
- GitHub Actions runs the format check, lint, unit tests and build on every
  pull request.

## Architecture

Refer to the [Architecture Docs](docs/architecture/README.md) for the key 
architectural decisions and the general design of the project.

## Current state of the project

You can run a sandbox based on `ubuntu:26.04` on Linux with a local
`containerd` runtime.
