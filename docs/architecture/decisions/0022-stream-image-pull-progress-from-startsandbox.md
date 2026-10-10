# 22. Stream image pull progress from StartSandbox

## Status

Accepted

## Context

When `fbk start` or `fbk run` creates a sandbox whose image isn't cached, fbkd
pulls it first. For the `firebrick-base` image that can take minutes, and fbk
only showed `Creating sandbox <name>...`, so a slow download looked the same as
a hung daemon.

microsandbox reports the pull through
`SandboxBuilder::create_detached_with_pull_progress`, which returns a channel of
`PullProgress` events next to the task that creates the sandbox. The events are
best-effort: microsandbox drops them when the channel is full. A pull from the
cache sends only `Resolving`, `Resolved` and `Complete`, without layer
downloads.

`StartSandbox` was a unary RPC, so fbkd had no way to send anything before the
sandbox was created. The alternatives were a separate `PullImage` RPC, which
duplicates the create logic and leaves a window between pull and create, or a
progress RPC the CLI polls, which needs state in fbkd shared between requests.

Drawing a progress bar that updates in place needs terminal handling: cursor
movement, line clearing, terminal width and rate limiting.

## Decision

`StartSandbox` becomes a server-streaming RPC. Its stream carries
`ImagePullProgress` messages while the image downloads and ends with
`SandboxStarted`, which holds the forwards the unary response used to return.
Errors end the stream with a gRPC status, like the unary RPC returned them;
[ADR 0023](0023-match-image-pull-errors-on-their-types.md) gives a failed pull
its own codes and messages.

fbkd runs the start in a task of its own and adds up the bytes per layer in the
`pull` module, sending at most one message every 100 ms, one for each layer that
finishes, so the bar doesn't stall while the image is unpacked, and a final one
with `complete` set. It only sends progress once a layer downloads, so a cached
image sends none. Messages that can't be sent, because the client is slow or
gone, are dropped without failing the create.

fbk draws the bar with `indicatif` 0.18 when stderr is a terminal, and prints a
line when the download starts and one when it ends otherwise.

## Consequences

- The developer sees how far the pull has got, with downloaded and total MiB and
  a percentage, and can tell a slow download from a hung daemon.
- fbk and fbkd must be upgraded together: an older fbk can't read the stream of
  a newer fbkd, and the other way around. Both ship in the same release.
- A start keeps going when the client disconnects, so the sandbox is still
  created and its SSH config synced. fbkd doesn't wait for running starts when
  it shuts down, so stopping fbkd can still cut a create off.
- The CLI depends on `indicatif` and, through it, `console`. The unit tests use
  its `in_memory` feature to check what the bar draws.
- Progress for other phases, such as booting the VM or `mise install`, can be
  added to the same stream later as new `oneof` variants.
