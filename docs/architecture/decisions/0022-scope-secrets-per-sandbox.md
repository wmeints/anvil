# 22. Scope secrets per sandbox

## Status

Accepted

## Context

[ADR 0004](0004-store-secrets-in-a-private-file.md) made every secret global:
`fbkd` keeps one list in `secrets.yml` and adds every secret to every sandbox it
created. A name such as `MY_SECRET` can only have one value, so a user can't
give each project its own API token, while shared secrets such as
`CLAUDE_CODE_OAUTH_TOKEN` should stay the same for all sandboxes.

microsandbox keeps the secrets per sandbox already, so the question is only
which secrets `fbkd` adds to which sandbox, and how it stores that.

We considered two options:

- **A: an optional `sandbox` field per entry in `secrets.yml`** - an entry
  without it is global, an entry with it belongs to that sandbox. A sandbox gets
  the global secrets, and its own secrets replace the global ones with the same
  name.
- **B: a secrets section in `.firebrick.yml`** - the project declares its
  secrets. The values would end up in the workspace, which the agent can read
  and which may be committed, so the values would need a separate store anyway.

## Decision

We use option A.

- `fbk secret set` and `fbk secret rm` take `--scope <global|sandbox>`, which
  defaults to `global`. `sandbox` targets the sandbox of the working directory,
  resolved like `fbk stop` and `fbk rm` without a name. `SetSecretRequest` and
  `RemoveSecretRequest` carry it as `optional string sandbox`, where unset means
  global.
- A name is unique within a scope. The same name may exist globally and in any
  number of sandbox scopes. `secrets.yml` with the same name twice in one scope
  fails to load.
- A sandbox-scoped secret wins over the global secret with the same name in its
  sandbox:
  - Setting it adds it to that sandbox only, replacing the global value there.
  - Removing it removes it from that sandbox, and adds the global secret with
    the same name back when there is one.
  - Setting or removing a global secret skips the sandboxes that have a
    sandbox-scoped secret with that name.
- A sandbox-scoped secret can only be set for, or removed from, a sandbox that
  exists (`NOT_FOUND` otherwise). New sandboxes get the global secrets only.
- When `RemoveSandbox` removes a sandbox, `fbkd` removes that sandbox's scoped
  secrets from `secrets.yml`, so a later sandbox with the same name doesn't
  inherit them.
- `ListSecrets` returns the scope of each secret, sorted by name and then scope
  with the global secret first. `fbk secret ls` shows it in a `SCOPE` column,
  and as a `scope` field in JSON, with `global` or the sandbox name.

`secrets.yml` stores the scope as an optional `sandbox` field:

```yaml
- name: CLAUDE_CODE_OAUTH_TOKEN
  value: ...
  allowed_hosts: [api.anthropic.com]
- name: MY_SECRET
  sandbox: firebrick-d9f287
  value: ...
  allowed_hosts: [api.example.com]
```

## Consequences

- Existing `secrets.yml` files load unchanged, as global secrets.
- The values stay in `secrets.yml` (`0600`) and never leave the daemon, as in
  ADR 0004. Secret changes still apply on the next start of a sandbox.
- A scope names a sandbox, not a project directory. Renaming a project's sandbox
  in `.firebrick.yml` leaves its scoped secrets behind under the old name until
  they're removed with `fbk secret rm --scope sandbox` from a directory that
  resolves to that name, or until a sandbox with that name is removed.
- A sandbox removed without `fbkd`, for example with `msb`, keeps its scoped
  secrets in `secrets.yml`; a new sandbox with that name doesn't get them, but
  `fbk secret ls` still lists them.
