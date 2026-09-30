---
name: fix-bug
description: "Fix a bug in anvil through root cause analysis and a reproducing test. Use when the user reports a bug, a crash, a failing command, an unexpected error, or wrong behavior and wants it fixed."
---

# Fix bug

Fix the cause of a bug, not its symptom, and leave a test behind that fails
without the fix.

## Steps

### 1. Reproduce the bug

- Restate the observed and the expected behavior in one sentence each.
- Reproduce the bug with the smallest command or input you can find. Use the
  built binaries from `task build` for CLI bugs.
- If you can't reproduce it, stop and ask the user for the missing details. Do
  not fix a bug you haven't seen.

### 2. Find the root cause

- Trace the failure from the symptom back to the code that makes the wrong
  decision. Read the code; don't guess from names.
- Ask "why" until the answer is a line of code or a wrong assumption, not a
  side effect of one.
- Check the [architecture docs](../../../docs/architecture/README.md) for
  known pitfalls, such as rootless containerd and host mounts.
- Write down the root cause in one or two sentences. If several causes are
  plausible, rule them out one by one with evidence.

### 3. Write a failing test

- Add a test that reproduces the bug through the public interface of the
  package, next to the existing tests.
- Use a unit test when you can. Use an integration test
  (`<name>_integration_test.go` with `//go:build integration`) only when the
  bug needs containerd.
- Run it and confirm it fails for the reason you found in step 2.

### 4. Fix the root cause

- Make the smallest change that fixes the cause. Follow the implementation
  ladder and the [engineering guidelines](../../../docs/engineering/README.md)
  from `CLAUDE.md`.
- Look for the same mistake elsewhere in the code and fix it there as well.

### 5. Verify

- Run the new test and confirm it passes.
- Run `task format`, `task lint` and `task test`. Run
  `task test:integration` when the fix touches `internal/sandbox` or
  `internal/daemon`.
- Repeat the reproduction from step 1 and confirm the bug is gone.

### 6. Report

Tell the user the root cause, the fix, the test that covers it, and any
related spots you changed. Suggest a commit message of the form
`fix(<scope>): <description>`.
