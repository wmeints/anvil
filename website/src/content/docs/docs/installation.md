---
title: Installation
description: Install the fbk and fbkd binaries from a release or from source.
---

Each [GitHub release](https://github.com/wmeints/firebrick/releases) has an
archive per platform with the `fbk` and `fbkd` binaries:

| Platform              | Target                      |
| --------------------- | --------------------------- |
| Linux x86_64          | `x86_64-unknown-linux-gnu`  |
| Linux ARM64           | `aarch64-unknown-linux-gnu` |
| macOS (Apple Silicon) | `aarch64-apple-darwin`      |

Linux needs KVM, and the Linux binaries need glibc 2.35 or newer. Windows isn't
supported.

The steps below install both binaries in `~/.local/bin`, which doesn't need root
permissions. Keep `fbk` and `fbkd` in the same directory, because the CLI starts
the daemon from its own directory.

## 1. Download and install the binaries

The commands in this step use bash or zsh syntax. If you use fish, run `bash`
first and run them in that shell.

Set the release to install and pick the target for your machine:

```sh
VERSION=v0.4.0
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  TARGET=x86_64-unknown-linux-gnu ;;
  Linux-aarch64) TARGET=aarch64-unknown-linux-gnu ;;
  Darwin-arm64)  TARGET=aarch64-apple-darwin ;;
  *) echo "Unsupported platform: $(uname -s)-$(uname -m). Stop here and build from source." ;;
esac
NAME="firebrick-$VERSION-$TARGET"
```

When this prints `Unsupported platform`, there's no release archive for your
machine. Skip the remaining steps and follow
[Building from source](#building-from-source) instead.

Download the archive and its checksum, and verify the archive:

```sh
curl -fLO "https://github.com/wmeints/firebrick/releases/download/$VERSION/$NAME.tar.gz"
curl -fLO "https://github.com/wmeints/firebrick/releases/download/$VERSION/$NAME.tar.gz.sha256"
shasum -a 256 -c "$NAME.tar.gz.sha256"   # or: sha256sum -c "$NAME.tar.gz.sha256"
```

Extract the archive and copy both binaries to `~/.local/bin`:

```sh
tar -xzf "$NAME.tar.gz"
mkdir -p ~/.local/bin
install -m 755 "$NAME/fbk" "$NAME/fbkd" ~/.local/bin/
```

## 2. Add `~/.local/bin` to your `PATH`

Check whether the directory is on your `PATH` already:

```sh
command -v fbk
```

When this prints nothing, add the directory to the startup file of your shell
and open a new terminal:

- **zsh**, the default shell on macOS:

  ```sh
  echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
  ```

- **bash**, the default shell on most Linux distributions:

  ```sh
  echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
  ```

- **fish**:

  ```sh
  fish_add_path ~/.local/bin
  ```

## 3. Verify the installation

```sh
fbk --version
fbk ls
```

`fbk --version` prints the installed version, for example `fbk 0.4.0`.

`fbk ls` starts `fbkd`, which installs the microsandbox runtime in
`~/.local/state/firebrick/msb`, and lists your sandboxes (none yet). An error
here means `fbkd` couldn't start, for example because it isn't next to `fbk`.

## macOS: remove the quarantine flag

The macOS binaries aren't signed. When you download the archive through a
browser instead of `curl`, macOS quarantines the binaries and refuses to run
them. Remove the quarantine flag:

```sh
xattr -d com.apple.quarantine ~/.local/bin/fbk ~/.local/bin/fbkd
```

## Upgrading and uninstalling

To upgrade, repeat step 1 with the new `VERSION`, then stop the running daemon
so the CLI starts the new one on the next command:

```sh
pkill -TERM -x fbkd
```

To uninstall, stop the daemon and remove the binaries:

```sh
pkill -TERM -x fbkd
rm ~/.local/bin/fbk ~/.local/bin/fbkd
```

## Migrating from Anvil

Firebrick was called Anvil before. Firebrick doesn't read any of the old names,
so move your setup over by hand:

1. Stop the old daemon and remove its binaries:

   ```sh
   pkill -TERM -x anvild
   rm ~/.local/bin/anvil ~/.local/bin/anvild
   ```

   If you installed them with `cargo install`, run `cargo uninstall anvil-cli
   anvil-daemon` instead of `rm`.

2. Move the data directory with your SSH keys and secrets:

   ```sh
   mv ~/.local/share/anvil ~/.local/share/firebrick
   ```

3. Rename `.anvil.yml` to `.firebrick.yml` in each project.
4. Remove the old `*.anvil` hosts: the `Include` line for
   `~/.local/share/anvil/ssh/config` in `~/.ssh/config`, the `*.anvil` keys in
   `remote.SSH.remotePlatform` of your VS Code settings, and the `*.anvil`
   entries in `ssh_connections` of your Zed settings.
5. Run `fbk start` once in each project. It gives the existing sandbox a
   `<leaf>.fbk` host name. Until then, `fbk ls` shows no host name for it, you
   can't connect to it over SSH, and `fbk secret set` and `fbk secret rm` skip
   it.

## Building from source

Install the development toolchain with [mise](https://mise.jdx.dev) from a clone
of the repository, then let `cargo install` put both binaries in `~/.cargo/bin`.
Make sure that directory is on your `PATH`, as in step 2:

```sh
git clone https://github.com/wmeints/firebrick.git
cd firebrick
mise install
cargo install-cli     # cargo install --locked --path crates/cli
cargo install-daemon  # cargo install --locked --path crates/daemon
```

A build from source that isn't a release defaults to a `firebrick-base` image
tag that may not exist yet. Set `image` in `.firebrick.yml` to a released tag or
your own image; see [Custom images](/firebrick/docs/custom-images/).
