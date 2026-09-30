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
- [containerd](https://containerd.io) 2.3 or newer. Make sure [to configure 
  it to run rootless](https://containerd.io/docs/2.4/rootless/).
- `erofs-utils` (provides `mkfs.erofs`)
- The `erofs` kernel module loaded

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

If you want to contribute, install the git hooks as well. They check the
formatting, lint and tests before each commit and run the integration tests
before each push.

```bash
lefthook install
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

- [Architecture documentation](docs/architecture/README.md)
- [Engineering documentation](docs/engineering/README.md)

## License

Anvil is licensed under the [MIT License](LICENSE). The bundled
[Nerdbox](https://github.com/containerd/nerdbox) submodule in `third_party/`
keeps its own license.
