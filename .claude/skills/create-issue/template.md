# Issue Body Templates

Use the template that matches the issue type. Keep the section names. Replace
every `<...>` placeholder with real content, and remove a section marked
*optional* when it doesn't apply.

## Bug

~~~markdown
## Summary

<One sentence: what is wrong, and where.>

## Reproduction

Version: `<git rev-parse --short HEAD>`

```sh
<the smallest command sequence that shows the bug>
```

**Observed:** <the actual output or behavior, quoted exactly.>

**Expected:** <the correct output or behavior.>

## Context

- **Affected code:** <paths and symbols, such as `internal/sandbox/name.go`
  and `sandbox.ValidateName`.>
- **Suspected cause:** <hypothesis and the evidence for it, or "unknown".>
- **Constraints:** <architecture rules that apply, with a link to the doc.>

## Acceptance criteria

- [ ] A test in `<path>_test.go` reproduces the bug and fails without the fix.
- [ ] <the observable behavior after the fix.>
- [ ] <each related error case.>

## Out of scope

- <adjacent work the implementer must leave alone.>

## Verification

```sh
task format && task lint && task test
<task test:integration, when internal/sandbox or internal/daemon changes>
<the reproduction command, now showing the expected behavior>
```

## Notes *(optional)*

<Decisions left to the implementer, links, related issues.>
~~~

## Feature

~~~markdown
## Goal

<The problem this solves and for whom, in two or three sentences.>

## Behavior

<The commands, flags, API messages or output that change, with an example
of each.>

```sh
<example invocation and output>
```

## Errors

- <what can go wrong> → <what the user sees.>

## Context

- **Affected code:** <packages, files and symbols to change.>
- **Reuse:** <existing code or dependencies to build on.>
- **Constraints:** <architecture rules that apply, with a link to the doc.>
- **Depends on:** <#issue, or remove this line.>

## Acceptance criteria

- [ ] <each observable behavior from the Behavior section.>
- [ ] <each error case from the Errors section.>
- [ ] Unit tests cover the public interface in `<path>_test.go`.
- [ ] <integration test in `<path>_integration_test.go`, when containerd is
  involved.>
- [ ] The architecture docs describe the new behavior.
- [ ] <A decision record in `docs/architecture/decisions/`, when this adds a
  dependency or makes an architectural choice.>
- [ ] <`README.md` shows the new usage, when the CLI changes.>

## Out of scope

- <adjacent work the implementer must leave alone.>

## Verification

```sh
task format && task lint && task test
<task test:integration, when internal/sandbox or internal/daemon changes>
<a command that demonstrates the feature>
```

## Notes *(optional)*

<Decisions left to the implementer, alternatives considered, links.>
~~~
