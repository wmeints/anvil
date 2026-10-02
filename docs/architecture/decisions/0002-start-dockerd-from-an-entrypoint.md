# 2. Start dockerd from an entrypoint script

## Status

Accepted

## Context

`dockerd` must run before the agent can use Docker in the `anvil-base` image.
We can start it with an init system, such as systemd, or with a small
entrypoint script.

## Decision

The `anvil-entrypoint` script starts `dockerd` as root in the background with
`sudo`, waits at most 30 seconds until `docker info` succeeds, and then runs
the container command with `exec`.

## Consequences

- The image stays small and runs any command, like a regular container image.
- Nothing restarts `dockerd` when it crashes. Its log is in
  `/var/log/dockerd.log`.
- The container command runs as PID 1. It doesn't reap orphaned processes
  and, like bash, may ignore `SIGTERM`, so `dockerd` gets no clean shutdown
  on a stop. The entrypoint clears the stale runtime state on the next start.
  Since [decision 3](./0003-tini-in-the-base-image.md), tini runs as PID 1,
  so the command gets `SIGTERM` and orphans are reaped, but `dockerd` is
  still killed without a clean shutdown.
- `/var/lib/docker` is a volume, so Docker's overlay2 storage doesn't sit on
  overlay. Whether anvil and nerdbox honor the `VOLUME` still has to be
  checked when anvil starts using the image. Since
  [decision 4](./0004-run-sandboxes-as-the-image-user-privileged-in-the-vm.md),
  anvil backs each volume with a tmpfs.
- A container without the privileges `dockerd` needs fails after 30 seconds
  with a clear error instead of hanging.
