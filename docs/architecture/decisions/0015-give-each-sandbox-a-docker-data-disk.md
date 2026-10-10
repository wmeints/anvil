# 15. Give each sandbox a Docker data disk

## Status

Accepted

## Context

The `firebrick-base` image is going to run a Docker engine inside the sandbox
VM, so agents can build images and run containers. Docker keeps its images,
layers and containers in `/var/lib/docker` and stores them with the overlayfs
snapshotter.

Microsandbox mounts the sandbox's root filesystem as overlayfs (`overlay on /
type overlay (lowerdir=/.msb/rootfs/lower,upperdir=/.msb/rootfs/upperfs/upper,
...)`). Overlayfs can't use another overlayfs as its upper directory, so
Docker's storage doesn't work when `/var/lib/docker` sits on the root
filesystem.

## Decision

`fbkd` attaches a sandbox-owned ext4 disk at `/var/lib/docker` to every sandbox
it creates, whatever the image and whether or not `init` is on. It uses
microsandbox's owned volumes: `.volume("/var/lib/docker", |m| m.owned_with(|v|
v.disk().size(mib)))`.

The size comes from the optional `volumes.docker` field in `.firebrick.yml`, in
the same units as `resources.memory`, and defaults to `20 GiB`. Volumes get
their own `volumes` section instead of a field under `resources`, so sizes of
later volumes have a place to go. The gRPC `StartSandboxRequest` mirrors it with
an optional `SandboxVolumes` message; a missing message or an empty `docker`
size means the default, so older clients keep working.

## Consequences

- Docker inside the sandbox can use the overlayfs snapshotter, because its data
  lives on a plain ext4 filesystem.
- The disk belongs to the sandbox: it keeps its contents across `fbk stop` and
  `fbk start`, and `fbk rm` deletes it with the sandbox. Images pulled in a
  sandbox aren't shared with other sandboxes.
- The disk is a sparse file in the sandbox's microsandbox directory, so it only
  takes the host space that the guest has written, up to its size.
- Every sandbox gets the disk, also when its image doesn't use Docker. The path
  isn't configurable.
- Like the other resources, the size applies when the sandbox is created.
  Sandboxes created before this change have no disk until they're recreated.
