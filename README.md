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
cargo install-cli     # cargo install --locked --path crates/cli
cargo install-daemon  # cargo install --locked --path crates/daemon
```

## Usage

Run the commands from your project directory. Anvil derives the sandbox from
that directory. Pass a name from `anvil ls` to `start`, `stop` or `rm` to
manage another sandbox from any directory.

| Command                     | Description                                          |
| --------------------------- | ---------------------------------------------------- |
| `anvil start [name]`        | Start the sandbox for the current directory, or the existing sandbox with the name from `anvil ls`. |
| `anvil run <cmd> [args...]` | Start the sandbox when needed and run a command in it with a terminal attached. |
| `anvil stop [name]`         | Stop the sandbox, or the one with the name from `anvil ls`. Files on its disk are kept. |
| `anvil ls [--format json]`  | List all sandboxes as a table, or as JSON with `--format json`. |
| `anvil rm [name]`           | Remove the sandbox, or the one with the name from `anvil ls`. |
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
image: ghcr.io/my-org/my-sandbox:1.0
resources:
  cpu: 2
  memory: 4 GiB
```

| Field              | Description                                                      | Default                                 |
| ------------------ | ---------------------------------------------------------------- | --------------------------------------- |
| `name`             | Name of the sandbox.                                             | Required                                |
| `image`            | OCI image the sandbox runs.                                      | `ghcr.io/wmeints/anvil-base:v<version>` |
| `resources.cpu`    | Number of vCPUs.                                                 | `2`                                     |
| `resources.memory` | Memory in `Mi`/`MiB` or `Gi`/`GiB`, such as `512 MiB` or `4Gi`. | `4 GiB`                                 |

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

To let `git` push and pull over HTTPS with `GH_TOKEN`, configure a credential
helper in the sandbox that hands git the placeholder as the password:

```sh
git config --global credential.https://github.com.helper \
  '!f() { test "$1" = get && echo username=x-access-token && echo "password=$GH_TOKEN"; }; f'
```

git sends the placeholder base64-encoded in a Basic `Authorization` header,
and the host decodes it, replaces the placeholder and encodes it again. If
`gh` is installed in the sandbox, `gh auth setup-git` configures an
equivalent helper.

Keep in mind that:

- Don't use SSH keys for git: the sandbox's SSH server doesn't support agent
  forwarding (`ssh -A`), and copying a private key into the sandbox puts the
  real key where the agent can read it. Use git over HTTPS instead.
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
`agent` user. Releases publish it as `ghcr.io/wmeints/anvil-base:<tag>`, and
sandboxes run the image that matches the installed anvil version unless the
`image` field in `.anvil.yml` names another one, such as an image built on top
of it.

### Bringing your own image

Anvil runs everything in a sandbox as the `agent` user. A custom image must:

- Have a user named `agent` with UID `1000` and GID `1000` and a home
  directory, such as `/home/agent`. Images based on Ubuntu ship an `ubuntu`
  user with UID 1000; remove it first.
- Set `USER agent`. `anvil run` runs commands as the image's user, while SSH
  always logs in as `agent`.
- Install `sudo` and allow `agent` to use it without a password, if agents
  should be able to install system packages.

The workspace is mounted at `/workspaces/<project>`, and its files show up
as owned by `agent`, whatever the UID of your user on the host is.

Sandboxes created by an older version of Anvil run `ubuntu:26.04`, which has no
`agent` user, so SSH can no longer log in to them. Recreate them with
`anvil rm` and `anvil start`, or connect with `ssh root@<name>.anvil`.

The simplest way to meet these requirements is to build on the base image:

```dockerfile
FROM ghcr.io/wmeints/anvil-base:v0.1.0

USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/*
USER agent
```

For another distribution, create the user yourself:

```dockerfile
FROM alpine:3.22

RUN apk add --no-cache bash git sudo \
    && addgroup -g 1000 agent \
    && adduser -D -u 1000 -G agent -s /bin/bash agent \
    && echo "agent ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/agent

USER agent
WORKDIR /home/agent
```

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

| Command                   | Description                                   |
| ------------------------- | --------------------------------------------- |
| `cargo build`             | Build the `anvil` and `anvild` binaries.      |
| `cargo unit-tests`        | Run the unit tests.                           |
| `cargo integration-tests` | Run the integration tests that boot real VMs. |
| `cargo lint`              | Run the linter.                               |
| `cargo fmt --all`         | Format the code.                              |

The default sandbox image is the `anvil-base` image of the same release, so it
doesn't exist for a version that hasn't been released yet. To run a
development build, push an image built from the [`Dockerfile`](Dockerfile) to a
local registry and set `image` in `.anvil.yml` to it:

```sh
docker run -d -p 127.0.0.1:5000:5000 --name registry registry:2
docker build -t localhost:5000/anvil-base:dev .
docker push localhost:5000/anvil-base:dev
```

The local registry speaks plain HTTP, so allow it in
`~/.microsandbox/config.json` before `anvild` starts:

```json
{ "registries": { "hosts": { "localhost:5000": { "insecure": true } } } }
```

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
