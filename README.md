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

Building Nerdbox requires [Docker](https://docs.docker.com/engine/install/)
with buildx, [Task](https://taskfile.dev), `erofs-utils`, `e2fsprogs` and
[libkrun](https://github.com/containers/libkrun) 1.18 or newer. Clone and
build it:

```bash
git clone https://github.com/containerd/nerdbox.git
cd nerdbox
make
```

The build places its artifacts in `_output/`. Install the shim on your `PATH`
and the kernel and VM root filesystem in `/usr/local/lib`, where the shim
looks for them:

```bash
sudo install -m 755 _output/containerd-shim-nerdbox-v1 /usr/local/bin/
sudo install -m 644 _output/nerdbox-kernel-$(uname -m) /usr/local/lib/
sudo install -m 644 _output/nerdbox-rootfs.erofs /usr/local/lib/
```

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

`anvil run` boots an `ubuntu:26.04` microVM and opens `/bin/bash` inside it.
Type `exit` to hibernate the sandbox and return to the host. The VM stops, but
the sandbox disk is kept, so the next `anvil run` resumes with your files
intact.

`anvil` talks to the `anvild` daemon, so start `dist/bin/anvild` first. The
daemon connects to the rootless containerd socket in
`/run/user/<uid>/containerd/containerd.sock`, and containerd stores the images
and sandbox disks. If something goes wrong, check the output of `anvild` and
the containerd logs:

```bash
journalctl --user -u containerd
```

## Documentation

- [Architecture documentation](docs/architecture/README.md)
- [Engineering documentation](docs/engineering/README.md)

## License

Anvil is licensed under the [MIT License](LICENSE). The bundled
[Nerdbox](https://github.com/containerd/nerdbox) submodule in `third_party/`
keeps its own license.
