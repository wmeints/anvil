# Crosscutting concepts

## How are sandboxes executed

When running a sandbox, we're talking about an actual OCI container running in 
a VM. We use [containerd](https://containerd.io/) to run containers. So in a 
way, this is just another container management tool.

All this doesn't sound terribly isolated, and you're right. But before we talk 
about isolation there's one more concept to understand. The containerd runtime
runs containers using shims. Whenever you restart containerd, the containers 
themselves remain running. That's because they run behind a shim that has a 
one-on-one relationship with the actual container process. 

It's the shim that we need to work with to provide proper isolation. We use 
[nerdbox](https://github.com/containerd/nerdbox) as a shim so that the container
isn't a cgroup, but a virtual machine.

You won't see a lot of interaction in this application with nerdbox other than
us specifying that a new container must run with the nerdbox shim. The rest of 
the logic in this application only deals with containerd.

As long as you run containerd rootless, you're good to go!

## How anvil derives the sandbox process

Anvil can't use containerd's `oci.WithImageConfig`, because it resolves the
user in the image, which mounts the rootfs on the host and fails under
rootless containerd. `internal/sandbox` builds the spec from the image config
instead:

- **Process.** The image `Entrypoint` followed by `sleep infinity`. The image
  `Cmd` is ignored, so an image without an entrypoint runs `sleep infinity`.
  For `anvil-base`, the process is
  `/usr/bin/tini -- /usr/local/bin/anvil-entrypoint sleep infinity`.
  Entrypoints must `exec "$@"`, or the sandbox stops when they exit.
- **User.** The image must declare a numeric, non-root `USER`, such as `1000`
  or `1000:1000`. Without a GID, the GID equals the UID. An empty, named, root
  or out-of-range user fails with `ErrInvalidImageUser` before anvil creates a
  container, so a rejected image leaves nothing behind. Sessions run as the
  same user. The process has no supplementary groups.
- **Environment.** The image `Env` and `WorkingDir`, which defaults to `/`.
- **Hostname.** The sandbox name.
- **Volumes.** A tmpfs for each image `VOLUME`, because overlayfs can't use
  the virtiofs rootfs as its upper directory. A volume starts empty, without
  the image's files at that path. The contents live in the VM's memory and are
  lost when the sandbox stops.
- **Privileges.** The container is privileged inside the VM: all known
  capabilities, new privileges allowed for `sudo`, no masked or read-only
  paths, writable sysfs and cgroup mounts, and no seccomp profile. The VM is
  the security boundary.

See [decision 4](./decisions/0004-run-sandboxes-as-the-image-user-privileged-in-the-vm.md).
