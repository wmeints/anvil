# Anvil

This project helps you run your coding agent inside a sandboxed microVM so you 
can keep the agent under control and your secrets safe. 

--------------------------------------------------------------------------------

:construction: **Current state of the project**

You can run the project on your machine, and it will run Ubuntu 26.04 in a VM.
Beyond that, there's not much you can do yet!

For the next few weeks, I expect this tool will only work on Linux. Mac will
follow after, once I've worked out the key interactions on Linux.

--------------------------------------------------------------------------------

## Why does this project exist?

This project exists, because while Docker Sandboxes are awesome, they're also 
not truly free to use. You need to login on Docker Hub to use the product. When
the hub is down, you can't work. 

Also, I foresee a future where Docker Sandboxes will no longer be free. And 
that sucks for everyone who's working on open-source and wants to do so without
having to spend a ton of money.

## System requirements

- [Mise](https://mise.jdx.dev)
- Linux with KVM (`/dev/kvm` must be readable and writable by your user)
- [containerd](https://containerd.io) 2.3 or newer on the `PATH`. Anvil starts
  its own private instance as your user, so the system containerd service
  doesn't need to run.
- `erofs-utils` (provides `mkfs.erofs`)
- The `erofs` kernel module loaded
- Docker with buildx, to build the nerdbox components

Anvil runs without root. The only steps that need root are this one-time host
setup:

```bash
sudo modprobe erofs
echo erofs | sudo tee /etc/modules-load.d/erofs.conf  # load it on every boot
```

## Getting started

Before you run anything, make sure you have all dependencies available on your
machine with [Mise](https://mise.jdx.dev).

```bash
mise install
```

[Nerdbox](https://github.com/containerd/nerdbox) is included as a git
submodule in `third_party/nerdbox`. `task build` compiles anvil and builds the
nerdbox shim, libkrun, guest kernel and guest rootfs with Docker. The first
build compiles a Linux kernel and takes a while; later builds only rebuild
nerdbox when its sources change.

```bash
git submodule update --init
task build
./dist/bin/anvil run
```

The build output uses an install layout: `dist/bin/anvil` and the nerdbox
components in `dist/lib/anvil`. Anvil looks for them in `../lib/anvil` next to
its executable (override with `--nerdbox-dir` or `ANVIL_NERDBOX_DIR`). To
install into `~/.local` (or another `PREFIX`):

```bash
task install               # ~/.local/bin/anvil + ~/.local/lib/anvil
PREFIX=/opt/anvil task install
```

`anvil run` boots an `ubuntu:26.04` microVM and opens `/bin/bash` inside it.
Type `exit` to hibernate the sandbox and return to the host. The VM stops, but
the sandbox disk is kept, so the next `anvil run` resumes with your files
intact.

Anvil keeps its data in `~/.local/share/anvil`. If something goes wrong, check
`~/.local/share/anvil/containerd.log`.

## Documentation

TODO: Document how this product came to be.

## License

Anvil is licensed under the [MIT License](LICENSE). The bundled
[Nerdbox](https://github.com/containerd/nerdbox) submodule in `third_party/`
keeps its own license.
