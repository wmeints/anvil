# 3. Run tini as the init of the base image

## Status

Accepted

## Context

The process that runs as PID 1 of a sandbox inherits every orphaned process,
and in a PID namespace it ignores the signals it doesn't handle. In the
`anvil-base` image, `anvil-entrypoint` `exec`s the container command, so that
command became PID 1. A command such as `sleep infinity` or `bash` ignores
`SIGTERM` and doesn't reap orphans, so a stop waits for its timeout and
zombies from agent sessions pile up.

We considered these options:

- **Shell init in anvil.** anvil runs a shell that traps `SIGTERM` as init.
  That works with any image that has `/bin/sh`, but it reaps orphans only as
  a side effect of the shell's `wait`.
- **Bundled tini.** anvil embeds static tini binaries, writes one to the host
  before every boot and mounts it read-only in every sandbox, like
  `docker run --init`. That works with any image, but anvil ships and updates
  a third-party executable and shares a host directory with every VM.
- **tini in the image.** `anvil-base` installs tini and runs it as PID 1 from
  its `ENTRYPOINT`.

## Decision

`anvil-base` installs Ubuntu's `tini` package and sets
`ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/anvil-entrypoint"]`.

Once anvil runs the image's entrypoint
([#3](https://github.com/wmeints/anvil/issues/3)), sandboxes only accept
images built for anvil, so tini in the image covers them. Bundling tini would
mainly serve plain images such as `ubuntu:26.04`, which anvil won't accept
anymore.

tini runs without `-g`. Forwarding `SIGTERM` to the process group wouldn't
reach `dockerd`, because tini runs as `agent` and can't signal the root
`dockerd`, and it would signal every agent process in that group.

## Consequences

- `anvil-base` and the images built `FROM` it stop right away on `SIGTERM`
  and reap orphaned processes, also under `docker run`.
- `dockerd` still gets no clean shutdown: it's killed when tini exits, so
  `anvil-entrypoint` must keep clearing the stale runtime state on start.
- Ubuntu patches and updates tini with the rest of the image. anvil ships no
  third-party binaries and mounts nothing from the host for its init.
- An image that sets its own `ENTRYPOINT` without tini runs without an init
  that reaps orphans, and `anvil stop` waits for the timeout. The docs for
  building custom images must say so.
- Until anvil runs the image's entrypoint, sandboxes keep anvil's shell init
  and don't use tini. Since
  [decision 4](./0004-run-sandboxes-as-the-image-user-privileged-in-the-vm.md),
  anvil runs the entrypoint, so tini is the init of `anvil-base` sandboxes.
- When the container command, such as `sleep infinity`, exits, tini exits and
  the sandbox stops. A process in the sandbox that kills that command stops
  the sandbox.
