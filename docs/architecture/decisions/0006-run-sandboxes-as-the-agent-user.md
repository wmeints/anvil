# 6. Run sandboxes as the agent user

## Status

Accepted

## Context

Agents in a sandbox ran as `root`: the default image was `ubuntu:26.04`, and the
generated SSH config logged in as `root`. An agent that runs as `root` can
change anything in the guest, which makes mistakes harder to recover from and
gives a compromised tool more to work with.

microsandbox's SSH server logs in as the user that the SSH client asks for, and
`anvil run` runs commands as the image's `USER`. The SSH config needs one user
name that works for every sandbox. Files in the mounted workspace show up with
the host user's UID by default, which only matches the guest user when both
happen to be `1000`.

## Decision

Every sandbox image provides a user named `agent` with UID `1000` and GID
`1000`, and runs as it. The `anvil-base` image does so, the generated SSH config
logs in as `agent`, and the README describes the requirement for custom images.
`anvild` mounts the workspace with owner `1000:1000`, so host files show up as
owned by `agent` whatever the host user's UID is.

## Consequences

- Images without an `agent` user, such as plain `ubuntu:26.04`, still run with
  `anvil run`, but SSH can't log in to them.
- `agent` gets passwordless `sudo` in `anvil-base`, so agents can still install
  system packages. Leave `sudo` out of a custom image to take that away.
- `agent` can write to the workspace on hosts where the user has another UID,
  such as `501` on macOS. Files that keep their own owner in the guest, such as
  files created by `root` inside the sandbox, aren't remapped.
- Sandboxes created before this change, from `ubuntu:26.04`, have no `agent`
  user, so SSH to them fails after upgrading. Recreate them with `anvil rm` and
  `anvil start`, or connect with `ssh root@<name>.anvil`.
