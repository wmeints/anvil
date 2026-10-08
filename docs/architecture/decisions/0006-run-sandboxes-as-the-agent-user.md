# 6. Run sandboxes as the agent user

## Status

Accepted

## Context

Agents in a sandbox ran as `root`: the default image was `ubuntu:26.04`, and
the generated SSH config logged in as `root`. An agent that runs as `root`
can change anything in the guest, which makes mistakes harder to recover
from and gives a compromised tool more to work with.

microsandbox's SSH server logs in as the user that the SSH client asks for,
and `anvil run` runs commands as the image's `USER`. The SSH config needs one
user name that works for every sandbox, and a fixed UID makes files in the
mounted workspace line up with the host user.

## Decision

Every sandbox image provides a user named `agent` with UID `1000` and GID
`1000`, and runs as it. The `anvil-base` image does so, the generated SSH
config logs in as `agent`, and the README describes the requirement for
custom images.

## Consequences

- Images without an `agent` user, such as plain `ubuntu:26.04`, still run
  with `anvil run`, but SSH can't log in to them.
- `agent` gets passwordless `sudo` in `anvil-base`, so agents can still
  install system packages. Leave `sudo` out of a custom image to take that
  away.
- The workspace keeps the UID of the host user. With UID `1000` on the host,
  `agent` owns the workspace files; with another UID, it may not be able to
  write to them.
