# 3. Bundle tini as the sandbox init

## Status

Accepted

## Context

Every sandbox needs an init process that keeps it running until `anvil stop`.
An init process in a PID namespace ignores the signals it doesn't handle, and
it inherits every orphaned process in the sandbox. The first init was a shell
that trapped `SIGTERM` and ran `sleep infinity` in a loop. It stopped on
`SIGTERM`, and it only reaped orphaned processes as a side effect of the
shell's `wait` builtin. Whether that happens depends on the shell in the
image, and an init that doesn't reap lets zombies from agent sessions pile up
in long-lived sandboxes.

We considered these options:

- **Shell trap.** Keep the shell init. It works with any image that has
  `/bin/sh`, but reaping depends on that shell.
- **tini in the image.** Install tini in `anvil-base`. Plain images, such as
  `ubuntu:26.04`, wouldn't have it.
- **Bundled tini.** Ship tini with anvil and mount it in every sandbox, like
  `docker run --init` does.

## Decision

anvil bundles the static tini v0.19.0 binaries from the
[GitHub release](https://github.com/krallin/tini/releases/tag/v0.19.0) in
`internal/sandbox/tini/` and embeds the one for `GOARCH`. Nerdbox only runs on
`amd64` and `arm64`, so those are the only binaries. The downloads were
verified against the release's `.sha256sum` files:

| File                | SHA-256                                                            |
| ------------------- | ------------------------------------------------------------------ |
| `tini-static-amd64` | `c5b0666b4cb676901f90dfcb37106783c5fe2077b04590973b885950611b30ee` |
| `tini-static-arm64` | `eae1d3aa50c48fb23b8cbdf4e369d0910dfc538566bfd09df89a774aa84a48b9` |

tini is MIT licensed. Its `LICENSE` sits next to the binaries.

Before every boot, anvil writes tini to `/run/user/<uid>/anvil/init` and
mounts that directory read-only at `/.anvil`. The init runs
`/.anvil/tini -- /bin/sh -c 'while :; do sleep infinity & wait; done'`. The
loop stays, so a process that kills the `sleep` doesn't stop the sandbox.

## Consequences

- Every image stops right away on `SIGTERM` and has its orphaned processes
  reaped, without changes to the image.
- The anvil binary grows by the size of one tini binary, under 1 MB.
- The init still needs `/bin/sh` and `sleep` in the image.
- The shell under tini isn't PID 1, so a process that kills it, for example
  with `kill -9 -1`, stops the sandbox. With the shell as PID 1, only `kill 1`
  did.
- Updating tini means downloading the new release binaries, verifying their
  checksums and updating this record.
- Sandboxes created before this change keep their old init until they're
  recreated.
