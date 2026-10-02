# 4. Run sandboxes as the image user, privileged inside the VM

## Status

Accepted

## Context

Anvil started every sandbox from containerd's default spec. It replaced the
image command with its own shell init, ran it as root, and ignored the image's
entrypoint, user and volumes. The `anvil-base` image gives agents a non-root
`agent` user with passwordless `sudo` and a Docker engine. None of that worked
under anvil:

- The default spec drops the capabilities `dockerd` needs, masks parts of
  `/proc` and `/sys`, applies a seccomp profile and sets `NoNewPrivileges`,
  which breaks `sudo`.
- The default spec has no cgroup mount, so `dockerd` exits during startup.
- Nothing honors the image's `VOLUME`, so `/var/lib/docker` sits on the
  virtiofs rootfs. overlayfs can't use virtiofs as its upper directory, so
  Docker can't create containers.
- Anvil's containerd is rootless. Resolving a user name or its groups reads
  `/etc/passwd` and `/etc/group` from the rootfs, which mounts the rootfs on
  the host and fails with `open /dev/loop-control: permission denied`.
- The guest sets no hostname, and the image's `/etc/hosts` is empty, because
  Docker normally writes both at runtime. `dockerd` resolves `localhost` over
  DNS, and each lookup waits for a 5-second timeout, so it took 20 seconds to
  start.

## Decision

The microVM is the security boundary
([solution strategy](../04-solution-strategy.md)). Inside it, the sandbox
container runs privileged:

- `oci.WithAllKnownCapabilities` instead of `oci.WithAllCurrentCapabilities`,
  because the latter copies the capabilities of the rootless daemon on the
  host. `oci.WithPrivileged` isn't used for the same reason.
- `oci.WithNewPrivileges`, so `sudo` works.
- No masked or read-only paths, writable sysfs, a writable cgroup mount at
  `/sys/fs/cgroup`, and no seccomp profile.
- No host devices. `dockerd` runs containers without a device cgroup rule, so
  the allow-all device rule the issue considered isn't needed.

The sandbox process comes from the image:

- **User.** The image must declare a numeric, non-root `USER`, as `UID` or
  `UID:GID`. Without a GID, the GID equals the UID. Anvil rejects an empty,
  named, root or out-of-range user with `ErrInvalidImageUser` before it creates
  a container or snapshot. There is no fallback to root. The process gets no
  supplementary groups, because anvil can't read `/etc/group`.
- **Process.** The image `Entrypoint` followed by `sleep infinity`. The image
  `Cmd` is ignored. `sleep infinity` keeps the sandbox running, so entrypoints
  must `exec "$@"`.
- **Volumes.** Each image `VOLUME` gets a tmpfs.
- **Hostname.** The sandbox name.

The `anvil-base` entrypoint covers what anvil can't do without reading the
rootfs: it starts `dockerd --group agent` so `agent` can use the Docker socket
without the `docker` group, and adds `localhost` and the hostname to
`/etc/hosts` when they're missing.

## Consequences

- An agent in an `anvil-base` sandbox works as `agent` and can use `sudo` and
  Docker.
- Plain images such as `ubuntu:26.04` are rejected. Custom images must declare
  a numeric non-root `USER` and should build `FROM ghcr.io/wmeints/anvil-base`.
- The agent can gain every capability inside the VM through `sudo`, so it
  effectively controls the guest kernel. The container no longer isolates
  anything; the guest is treated as untrusted, and the VMM, with its virtio
  devices, virtiofs shares and vsock channel to the shim, is the only boundary
  to the host. That is the boundary the solution strategy relies on.
- Volumes live in the memory of the VM. Docker's images and containers count
  against the VM's memory and are lost when the sandbox stops. Unlike Docker,
  anvil doesn't copy the image's files at a volume path into the volume, so a
  volume starts empty. A persistent
  disk per sandbox, which nerdbox supports through `ext4` mounts, is follow-up
  work.
- An image whose entrypoint doesn't `exec "$@"` exits early, and the sandbox
  stops with it.
- When a process in the sandbox kills the `sleep infinity`, tini exits and the
  sandbox stops ([decision 3](./0003-tini-in-the-base-image.md)).
- A session can start before the entrypoint has finished, so in the first
  seconds after a sandbox boots Docker may not answer yet and `sudo` may warn
  that it can't resolve the hostname.
