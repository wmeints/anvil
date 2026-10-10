# 14. Name sandboxes after a hash of the working directory

## Status

Accepted

## Context

Without a `.firebrick.yml`, the CLI named the sandbox after the full working
directory path, with every run of non-alphanumeric characters replaced by an
underscore, such as `home_user_my_project_v2`. Deep paths gave long names that
are hard to read in `fbk ls` and to type in `fbk stop <name>`. The root path `/`
had no name at all.

## Decision

Without a spec file, the CLI names a new sandbox `firebrick-` followed by the
first 6 hex digits of the SHA-256 hash of the full working directory path, such
as `firebrick-d9f287`. The hash covers the path as `env::current_dir()` returns
it, without resolving symlinks. The `sha2` crate computes the hash.

Before using the hashed name, the CLI asks the daemon whether a sandbox with the
old path-derived name exists, and keeps using that sandbox when it does.

## Consequences

- Sandbox names have a fixed, short length, and every directory, including `/`,
  gets one.
- Sandboxes created before this change keep their name and their files.
- Each command without a spec file or explicit name makes an extra `GetSandbox`
  call to look for the old name.
- Two paths can share a hashed name, with a chance of about 1 in 16 million per
  pair. `fbk start` and `fbk run` then refuse to use the sandbox from the second
  path, because it mounts the first one as its workspace, instead of sharing it.
  A spec file with its own `name` gives the second path its own sandbox.
- Opening the same directory through a symlink gives another sandbox, as it did
  before.
- `firebrick-cli` depends on `sha2` directly. It was already in the dependency
  tree through other crates.
