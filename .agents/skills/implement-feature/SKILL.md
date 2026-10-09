---
name: implement-feature
description: "Implement a new feature or change in behavior, starting from an agreed spec. Use when the user asks to add, build, support, or change a capability of the project."
---

# Implement feature

Agree on what to build before building it, then build the minimum that meets
the spec, test-first.

## Steps

### 1. Understand the problem

- Read the architecture docs in `docs/architecture/` to understand how the
  project is structured, then read the code the feature touches. If the project
  has no `docs/architecture/` directory, rely on `CLAUDE.md` instead.
- Walk the implementation ladder from `CLAUDE.md`. If the feature doesn't need
  to be built, or already exists, say so and stop.

### 2. Write a spec and confirm it

Present a short spec to the user and wait for approval before writing code:

- **Goal**: the problem it solves and for whom.
- **Behavior**: the commands, flags, API messages, or output that change, with
  an example of each.
- **Errors**: what can go wrong and what the user sees in that case.
- **Design**: the modules that change and their public interface. Prefer
  extending a deep module over adding a new shallow one.
- **Out of scope**: what this change deliberately doesn't do.
- **Tests**: the unit and integration tests that prove the behavior.

Ask about anything the spec can't answer from the request or the code.

### 3. Write the tests

- Write tests against the public interface for each behavior and error in the
  spec.
- Mark integration tests as described in `CLAUDE.md`, so they can be run
  separately from the unit tests.
- Run them and confirm they fail.

### 4. Implement

- Write the minimum code that makes the tests pass.
- Follow the engineering guidelines in `docs/engineering/`.
- Follow the configured linter rules.
- Add new dependencies only when the spec names them.

### 5. Update the docs

- Update the architecture docs that describe the changed behavior, such as the
  building block view or runtime view.
- Add a decision record to `docs/architecture/decisions/` for new dependencies
  or architectural choices.
- Update `README.md` when the usage of the project changes.

### 6. Verify

- Run the format, lint, and test commands from `CLAUDE.md`, and the `vm-tests`
  integration tests when `crates/daemon` changed. Fix any failures.
- Report what you built, how it maps to the spec, and anything you left out.
