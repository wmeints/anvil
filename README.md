# Anvil

This project helps you run your coding agent inside a sandboxed microVM so you 
can keep the agent under control and your secrets safe. 

--------------------------------------------------------------------------------

:construction: **Current state of the project**

You can run the project on your machine, and it will run Ubuntu 26.04 in a VM.
Beyond that, there's not much you can do yet! For the next few weeks, I expect 
this tool will only work on Linux. Mac will follow after, once I've worked out 
the key interactions on Linux.

--------------------------------------------------------------------------------

## Why does this project exist?

This project exists, because while Docker Sandboxes are awesome, they're also 
not truly free to use. You need to login on Docker Hub to use the product. When
the hub is down, you can't work. 

Also, I foresee a future where Docker Sandboxes will ask money for the tool. And 
that sucks for everyone who's working on open-source and wants to do so without
having to spend a ton of money.

## System requirements

- [Mise](https://mise.jdx.dev)
- Linux with KVM (`/dev/kvm` must be readable and writable by your user)
- [containerd](https://containerd.io) 2.3 or newer. Make sure [to configure 
  it to run rootless](https://containerd.io/docs/2.4/rootless/).

Anvil uses containerd's default snapshotter, so the rootless containerd needs
no extra configuration beyond the Nerdbox shim below.

## Setting up Nerdbox

Anvil starts every sandbox with the `io.containerd.nerdbox.v1` runtime, so
containerd must be able to find the [Nerdbox](https://github.com/containerd/nerdbox)
shim. Without it, `anvil run` fails to start the sandbox.

Download the latest release from the
[Nerdbox releases page](https://github.com/containerd/nerdbox/releases) and
verify its checksum. Releases are published for x86_64 only:

```bash
VERSION=0.2.5
BASE=https://github.com/containerd/nerdbox/releases/download/v${VERSION}
curl -LO ${BASE}/nerdbox-${VERSION}-linux-amd64.tar.gz
curl -LO ${BASE}/nerdbox-${VERSION}-linux-amd64.tar.gz.sha256sum
sha256sum -c nerdbox-${VERSION}-linux-amd64.tar.gz.sha256sum
tar xzf nerdbox-${VERSION}-linux-amd64.tar.gz
cd nerdbox-${VERSION}-linux-amd64
```

The archive contains the shim, the VM kernel and root filesystem, and a
bundled copy of libkrun. Install the shim on your `PATH` and the other files
in `/usr/local/lib`, where the shim looks for them. The shim only picks up
the bundled libkrun under the name `libkrun-x86_64.so`:

```bash
sudo install -m 755 containerd-shim-nerdbox-v1 /usr/local/bin/
sudo install -m 644 nerdbox-kernel-x86_64 /usr/local/lib/
sudo install -m 644 nerdbox-rootfs.erofs /usr/local/lib/
sudo install -m 755 libkrun-nerdbox.so /usr/local/lib/libkrun-x86_64.so
```

> [!NOTE]
> On arm64 there is no release to download, so build Nerdbox from source.
> The build requires [Docker](https://docs.docker.com/engine/install/) with
> buildx, [Task](https://taskfile.dev), `erofs-utils`, `e2fsprogs` and
> [libkrun](https://github.com/containers/libkrun) 1.18 or newer. The shim
> uses the system libkrun, so install the artifacts without a libkrun copy:
>
> ```bash
> git clone https://github.com/containerd/nerdbox.git
> cd nerdbox
> make
> sudo install -m 755 _output/containerd-shim-nerdbox-v1 /usr/local/bin/
> sudo install -m 644 _output/nerdbox-kernel-* /usr/local/lib/
> sudo install -m 644 _output/nerdbox-rootfs.erofs /usr/local/lib/
> ```

Restart your rootless containerd so it picks up the shim, and verify that
the shim is found:

```bash
systemctl --user restart containerd
command -v containerd-shim-nerdbox-v1
```

## Getting started

Before you run anything, make sure you have all dependencies available on your
machine with [Mise](https://mise.jdx.dev).

```bash
mise install
```

If you want to contribute, install the git hooks as well. They check the
formatting, lint and tests before each commit and run the integration tests
before each push.

```bash
lefthook install
```

Build the CLI and the daemon. The executables end up in `dist/bin`:

```bash
task build
```

`anvil run` boots an `anvil-base` microVM and opens `/bin/bash` inside it. You
work as the non-root `agent` user, with passwordless `sudo` and Docker
available. Type `exit` to return to the host. The VM keeps running after the
session ends. Hibernate the sandbox with `anvil stop` to free its CPU and
memory:

```bash
dist/bin/anvil stop <name>
```

The VM stops, but the sandbox disk is kept, so your files survive. Running
processes don't survive a stop. Processes get 10 seconds to exit before they're
killed; change that with `--timeout`.

Boot a stopped sandbox again with `anvil start`, for example to warm it up
before you open a session:

```bash
dist/bin/anvil start <name>
```

The VM cold-boots with your files intact. `anvil run` boots a stopped sandbox
too, so you only need `anvil start` when you don't want to open a session yet.

`anvil` talks to the `anvild` daemon, so start `dist/bin/anvild` first. The
daemon connects to the rootless containerd socket in
`/run/user/<uid>/containerd/containerd.sock`, and containerd stores the images
and sandbox disks. If something goes wrong, check the output of `anvild` and
the containerd logs:

```bash
journalctl --user -u containerd
```

### Custom images

`anvil create` uses `ghcr.io/wmeints/anvil-base:latest` unless you pass an
image with `-i`. A custom image must declare a numeric non-root `USER`, such
as `1000:1000`, because anvil can't resolve user names. Build it
`FROM ghcr.io/wmeints/anvil-base` to keep the `agent` user, `sudo`, Docker and
the entrypoint. Anvil runs the image's `ENTRYPOINT` with `sleep infinity` as
its arguments and ignores `CMD`, so an entrypoint must end with `exec "$@"`.
Image volumes, such as Docker's `/var/lib/docker`, start empty, live in memory
and are lost when the sandbox stops.

## Documentation

- [Architecture documentation](docs/architecture/README.md)
- [Engineering documentation](docs/engineering/README.md)

## License

Anvil is licensed under the [MIT License](LICENSE).
[Nerdbox](https://github.com/containerd/nerdbox) is an external dependency
under its own license.
