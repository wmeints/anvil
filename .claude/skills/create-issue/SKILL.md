---
name: create-issue
description: "Register a new GitHub issue for anvil that a coding agent can pick up and implement without further questions. Use when the user wants to file, log, register, or open an issue, a bug report, a feature request, or a task on GitHub."
---

# Create issue

Turn a request into a GitHub issue that an agent can implement in one pull
request. The issue is the agent's whole spec: everything it needs to know must
be in the body, verified against the code, and free of open questions.

## Tooling

Use the `gh` CLI. Check that it is authenticated first:

```bash
gh auth status
```

If it reports unauthenticated, stop and ask the user to run `gh auth login`.

## Steps

### 1. Classify the issue

Decide whether the request is a **bug** (existing behavior is wrong) or a
**feature** (new or changed behavior). The type decides the template section
in `template.md` and the label: `bug` or `enhancement`. Documentation-only
work gets the `documentation` label and the feature template.

### 2. Check for duplicates

Search open and closed issues for the same problem:

```bash
gh issue list --state all --search "<keywords>" --limit 20
```

If a matching issue exists, show it to the user and ask whether to comment on
it instead of filing a new one.

### 3. Ground the issue in the code

An agent trusts the issue, so every pointer in it must be correct.

- Find the packages, files and symbols the change touches in `cmd/` and
  `internal/`. Name them with a path, such as
  `internal/sandbox/sandbox.go` and `sandbox.NewSandbox`. Don't use line
  numbers; they go stale.
- Read the relevant [architecture docs](../../../docs/architecture/README.md)
  and note constraints the implementer must respect, such as rootless
  containerd and host mounts.
- Find existing tests the implementer should extend and code they should
  reuse, following the implementation ladder in `CLAUDE.md`.
- For a bug, reproduce it with the smallest command you can find, as in
  the `fix-bug` skill. Record the exact command, output and version
  (`git rev-parse --short HEAD`). If you can't reproduce it, say so in the
  issue instead of guessing a cause.
- Record a suspected root cause only with the evidence for it, and mark it as
  a hypothesis.

### 4. Size the issue

One issue is one pull request. Split the work when it:

- touches more than one of the CLI, daemon, sandbox and TUI in ways that can
  ship separately,
- needs a new dependency or architectural decision before the rest can start,
- has acceptance criteria that don't depend on each other.

Propose the split to the user. File the parts as separate issues in
dependency order, and register each dependency as a GitHub relationship in
step 7.

### 5. Resolve open questions

Ask the user about anything the issue can't answer from the request or the
code: expected behavior, error messages, flag names, scope boundaries. An
agent can't ask back, so an unresolved question becomes a guess in the
implementation. Don't file the issue while questions remain; list decisions
the user deliberately leaves to the implementer under **Notes** instead.

### 6. Write the issue

Fill in the template from `template.md` for the issue type. Rules:

- **Title**: imperative and specific, prefixed like a commit:
  `fix(sandbox): reject names that start with a digit` or
  `feat(cli): add a --timeout flag to anvil run`.
- **Acceptance criteria**: each one observable and testable, written as a
  checkbox. Include the error cases. An implementer is done when every box can
  be ticked.
- **Out of scope**: name the tempting adjacent work the implementer must leave
  alone.
- **Verification**: the exact commands that prove the change works, including
  `task test:integration` when `internal/sandbox` or `internal/daemon`
  changes.
- Replace every `<...>` placeholder and remove optional sections that don't
  apply. Never leave a placeholder behind.

### 7. Confirm and create

Show the user the title, labels, body and the issues it is blocked by, and
wait for approval: creating an issue is public. Then write the body to a temp file and create the issue:

```bash
BODY=$(mktemp)
cat > "$BODY" <<'EOF'
<the body from step 6>
EOF

gh issue create \
  --title "<title>" \
  --body-file "$BODY" \
  --label "<bug|enhancement|documentation>"

rm -f "$BODY"
```

Add `good first issue` when the change is small and isolated to one package.
If a label doesn't exist, create the issue without it and tell the user.

Register every issue on the **Depends on** line as a "blocked by"
relationship, so GitHub tracks the order and not just the body text. The new
issue's number is at the end of the URL that `gh issue create` prints:

```bash
BLOCKER_ID=$(gh api repos/{owner}/{repo}/issues/<blocker number> -q .id)
gh api -X POST repos/{owner}/{repo}/issues/<new number>/dependencies/blocked_by \
  -F issue_id="$BLOCKER_ID"
```

If registering a relationship fails, keep the issue, tell the user which
relationship is missing, and continue.

### 8. Report

Give the user the issue URL and the issues it is blocked by, and for a split,
the URLs in dependency order.

## Guardrails

- **No unverified pointers.** Every path and symbol in the issue exists at the
  current `HEAD`.
- **No open questions.** Resolve them with the user before filing.
- **One pull request per issue.** Split larger work.
- **Testable criteria only.** Replace "works well" or "is fast" with an
  observable behavior or a measurable number.
- **Never fabricate.** Reproduction output, errors and versions come from a
  real run. Leave a section out rather than invent its content.
