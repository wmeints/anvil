# Anvil

Anvil runs coding agents safely inside a microVM-based sandbox on your own
machine. Each project gets its own lightweight VM, built from an OCI image,
with only the project directory shared from the host. You don't need a cloud
account, a commercial license or root permissions.

Anvil supports two ways of working:

- **Terminal agents** such as [Claude Code](https://claude.ai/code),
  [OpenCode](https://opencode.ai) and [Oh-my-pi](https://omp.sh): run them in
  the sandbox with `anvil run`.
- **IDE-integrated agents** such as GitHub Copilot: connect your IDE to the
  sandbox over SSH.

> [!NOTE]
> Anvil is early in development. Egress control is planned but not
> available yet.

## How it works

Anvil consists of two executables:

- `anvil` - the CLI you use to manage sandboxes and run commands in them.
- `anvild` - a daemon that manages the sandboxes through
  [microsandbox](https://docs.microsandbox.dev). The CLI starts it
  automatically when it isn't running.

The CLI and daemon talk gRPC over a unix socket
(`$XDG_RUNTIME_DIR/anvild.sock`, or `anvild.sock` in the temp directory when
`XDG_RUNTIME_DIR` isn't set). The daemon mounts the working directory
read/write in the sandbox at `/workspaces/<leaf>`, where `<leaf>` is the name
of the directory.

## Requirements

- Linux with KVM, or macOS on Apple Silicon.
- On Linux, glibc 2.35 or newer for the release binaries.

Windows isn't supported. `anvild` embeds the microsandbox runtime and installs
it in `~/.microsandbox` on first start.

## Installation

Each [GitHub release](https://github.com/wmeints/anvil/releases) has an
archive per platform with the `anvil` and `anvild` binaries:

| Platform              | Target                       |
| --------------------- | ---------------------------- |
| Linux x86_64          | `x86_64-unknown-linux-gnu`   |
| Linux ARM64           | `aarch64-unknown-linux-gnu`  |
| macOS (Apple Silicon) | `aarch64-apple-darwin`       |

The steps below install both binaries in `~/.local/bin`, which doesn't need
root permissions. Keep `anvil` and `anvild` in the same directory, because
the CLI starts the daemon from its own directory.

### 1. Download and install the binaries

The commands in this step use bash or zsh syntax. If you use fish, run
`bash` first and run them in that shell.

Set the release to install and pick the target for your machine:

```sh
VERSION=v0.1.0
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  TARGET=x86_64-unknown-linux-gnu ;;
  Linux-aarch64) TARGET=aarch64-unknown-linux-gnu ;;
  Darwin-arm64)  TARGET=aarch64-apple-darwin ;;
  *) echo "Unsupported platform: $(uname -s)-$(uname -m). Stop here and build from source." ;;
esac
NAME="anvil-$VERSION-$TARGET"
```

When this prints `Unsupported platform`, there's no release archive for your
machine. Skip the remaining steps and follow
[Building from source](#building-from-source) instead.

Download the archive and its checksum, and verify the archive:

```sh
curl -fLO "https://github.com/wmeints/anvil/releases/download/$VERSION/$NAME.tar.gz"
curl -fLO "https://github.com/wmeints/anvil/releases/download/$VERSION/$NAME.tar.gz.sha256"
shasum -a 256 -c "$NAME.tar.gz.sha256"   # or: sha256sum -c "$NAME.tar.gz.sha256"
```

Extract the archive and copy both binaries to `~/.local/bin`:

```sh
tar -xzf "$NAME.tar.gz"
mkdir -p ~/.local/bin
install -m 755 "$NAME/anvil" "$NAME/anvild" ~/.local/bin/
```

### 2. Add `~/.local/bin` to your `PATH`

Check whether the directory is on your `PATH` already:

```sh
command -v anvil
```

When this prints nothing, add the directory to the startup file of your
shell and open a new terminal:

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

### 3. Verify the installation

```sh
anvil ls
```

This starts `anvild`, which installs the microsandbox runtime in
`~/.microsandbox`, and lists your sandboxes (none yet). An error here means
`anvild` couldn't start, for example because it isn't next to `anvil`.

### macOS: remove the quarantine flag

The macOS binaries aren't signed. When you download the archive through a
browser instead of `curl`, macOS quarantines the binaries and refuses to run
them. Remove the quarantine flag:

```sh
xattr -d com.apple.quarantine ~/.local/bin/anvil ~/.local/bin/anvild
```

### Upgrading and uninstalling

To upgrade, repeat step 1 with the new `VERSION`, then stop the running
daemon so the CLI starts the new one on the next command:

```sh
pkill -TERM -x anvild
```

To uninstall, stop the daemon and remove the binaries:

```sh
pkill -TERM -x anvild
rm ~/.local/bin/anvil ~/.local/bin/anvild
```

### Building from source

With the [development toolchain](#development) installed, `cargo install`
puts both binaries in `~/.cargo/bin`. Make sure that directory is on your
`PATH`, as in step 2:

```sh
cargo install --locked --path crates/cli
cargo install --locked --path crates/daemon
```

## Usage

Run the commands from your project directory. Anvil derives the sandbox from
that directory.

| Command                     | Description                                          |
| --------------------------- | ---------------------------------------------------- |
| `anvil start`               | Start the sandbox for the current directory.         |
| `anvil run <cmd> [args...]` | Start the sandbox when needed and run a command in it with a terminal attached. |
| `anvil stop`                | Stop the sandbox. Files on its disk are kept.        |
| `anvil ls [--format json]`  | List all sandboxes as a table, or as JSON with `--format json`. |
| `anvil rm`                  | Remove the sandbox.                                  |
| `anvil validate`            | Check the `.anvil.yml` file in the current directory. |
| `anvil secret set <name> [<value>]` | Set a secret for all sandboxes. See [Secrets](#secrets). |
| `anvil secret ls [--format json]` | List the secrets and their allowed hosts, without their values. |
| `anvil secret rm <name>` | Remove a secret from all sandboxes. |

For example, to open a shell in the sandbox:

```sh
anvil run bash
```

The sandbox keeps running after the command exits.

### Configuring a sandbox

Add an `.anvil.yml` file to the project directory to configure the sandbox:

```yaml
name: my-project
image: ubuntu:26.04
resources:
  cpu: 2
  memory: 4 GiB
```

| Field              | Description                                                      | Default        |
| ------------------ | ---------------------------------------------------------------- | -------------- |
| `name`             | Name of the sandbox.                                             | Required       |
| `image`            | OCI image the sandbox runs.                                      | `ubuntu:26.04` |
| `resources.cpu`    | Number of vCPUs.                                                 | `2`            |
| `resources.memory` | Memory in `Mi`/`MiB` or `Gi`/`GiB`, such as `512 MiB` or `4Gi`. | `4 GiB`        |

Without `.anvil.yml`, Anvil names the sandbox after the full path of the
working directory and uses the defaults.

The image and resources apply when the sandbox is created. To change them for
an existing sandbox, run `anvil rm` and start it again.

### Secrets

Give agents tokens without letting the real values into the sandbox:

```sh
gh auth token | anvil secret set GH_TOKEN --from-stdin
anvil secret set ANTHROPIC_API_KEY --from-stdin < ~/anthropic-key.txt
anvil secret set MY_TOKEN --from-stdin --allow-host api.example.com
```

In the sandbox, the environment variable holds a placeholder such as
`$MSB_GH_TOKEN`. When a request to one of the secret's allowed hosts carries
the placeholder in an HTTP header, the host replaces it with the real value.
Requests that carry it to other hosts are blocked. Use `--from-stdin` rather
than the value as an argument, so the value stays out of your shell history.

These names have default allowed hosts. Other names need `--allow-host`,
which you can repeat:

| Name | Allowed hosts |
| --- | --- |
| `GH_TOKEN`, `GITHUB_TOKEN` | `github.com`, `api.github.com`, `uploads.github.com` |
| `COPILOT_GITHUB_TOKEN` | `github.com`, `api.github.com`, `*.githubcopilot.com` |
| `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN` | `api.anthropic.com` |

Secrets apply to all sandboxes. A running sandbox gets a new or changed
secret after `anvil stop` and `anvil start`.

`anvil secret ls` shows the names and allowed hosts of the secrets, never
their values. `anvil secret rm <name>` removes a secret, but a running
sandbox keeps using it until it restarts. If a token leaked, revoke it where
you created it as well.

Keep in mind that:

- `git` over HTTPS can't use secrets, because it sends the token in a way
  the placeholder can't be replaced. Use `gh` or SSH keys for git.
- The values are stored unencrypted in files only your user can read. If
  your machine may be compromised, rotate the tokens where you created them
  and set the new values.

### Connecting over SSH

`anvild` generates SSH keys and an SSH config, and includes that config in
`~/.ssh/config`. Each sandbox gets a host name after its project directory,
such as `my-project.anvil`. When two projects share a directory name, the
second gets `my-project-2.anvil`:

```sh
ssh my-project.anvil
```

Point your IDE's remote SSH support at the same host name to work in the
sandbox from your editor.

### Base image

The [`Dockerfile`](Dockerfile) describes a base image for sandboxes with
`git`, `curl`, `sudo`, [mise](https://mise.jdx.dev) and an unprivileged
`agent` user. Releases publish it as `ghcr.io/wmeints/anvil-base:<tag>`. Use
it, or an image built on top of it, through the `image` field in
`.anvil.yml`.

## Development

Install the toolchain (Rust, `buf` and `lefthook`) with
[mise](https://mise.jdx.dev). This also installs the git hooks:

```sh
mise install
```

Building also needs `protoc` 3.15 or newer, which supports proto3 optional
fields. The `protobuf-compiler` package on older distributions, such as
Ubuntu 22.04, is too old; install a current release from the
[protobuf releases](https://github.com/protocolbuffers/protobuf/releases)
instead.

| Command                                                 | Description                                   |
| ------------------------------------------------------- | --------------------------------------------- |
| `cargo build`                                           | Build the `anvil` and `anvild` binaries.      |
| `cargo test --workspace`                                | Run the unit tests.                           |
| `cargo test -p anvil-daemon --features vm-tests`        | Run the integration tests that boot real VMs. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Run the linter.                               |
| `cargo fmt --all`                                       | Format the code.                              |

The workspace contains four crates:

| Crate          | Folder           | Purpose                                         |
| -------------- | ---------------- | ----------------------------------------------- |
| `anvil-cli`    | `crates/cli`     | The `anvil` CLI.                                |
| `anvil-daemon` | `crates/daemon`  | The `anvild` daemon.                            |
| `anvil-spec`   | `crates/spec`    | Parses and validates `.anvil.yml`.              |
| `anvil-utils`  | `crates/utils`   | Shared paths for the socket, logs and SSH files. |

The gRPC contract lives in [`proto/daemon.v1.proto`](proto/daemon.v1.proto).

## Documentation

- [Architecture](docs/architecture/01-introduction-and-goals.md) - the arc42
  architecture documentation.
- [Decisions](docs/architecture/decisions/) - architecture decision records.
- [CLAUDE.md](CLAUDE.md) - coding guidelines and the definition of done.

## License

Anvil is licensed under the [MIT License](LICENSE).
