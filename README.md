# Firebrick

Firebrick (`fbk`) is an agentic sandbox: it runs coding agents safely inside a
microVM on your own machine. Each project gets its own lightweight VM, built
from an OCI image, with only the project directory shared from the host. You
don't need a cloud account, a commercial license or root permissions.

Firebrick supports two ways of working:

- **Terminal agents** such as [Claude Code](https://claude.ai/code),
  [OpenCode](https://opencode.ai) and [Oh-my-pi](https://omp.sh): run them in
  the sandbox with `fbk run`.
- **IDE-integrated agents** such as GitHub Copilot: connect your IDE to the
  sandbox over SSH.

> [!NOTE]
> Firebrick is early in development. Egress control is planned but not available
> yet.

## How it works

Firebrick consists of two executables:

- `fbk` - the CLI you use to manage sandboxes and run commands in them.
- `fbkd` - a daemon that manages the sandboxes through
  [microsandbox](https://docs.microsandbox.dev). The CLI starts it automatically
  when it isn't running.

The CLI and daemon talk gRPC over a unix socket (`$XDG_RUNTIME_DIR/fbkd.sock`,
or `fbkd.sock` in the temp directory when `XDG_RUNTIME_DIR` isn't set). The
daemon mounts the working directory read/write in the sandbox at
`/workspaces/<leaf>`, where `<leaf>` is the name of the directory.

## Requirements

- Linux with KVM, or macOS on Apple Silicon.
- On Linux, glibc 2.35 or newer for the release binaries.

Windows isn't supported. `fbkd` embeds the microsandbox runtime and installs it
in `~/.microsandbox` on first start.

## Installation

Each [GitHub release](https://github.com/wmeints/anvil/releases) has an archive
per platform with the `fbk` and `fbkd` binaries:

| Platform              | Target                      |
| --------------------- | --------------------------- |
| Linux x86_64          | `x86_64-unknown-linux-gnu`  |
| Linux ARM64           | `aarch64-unknown-linux-gnu` |
| macOS (Apple Silicon) | `aarch64-apple-darwin`      |

The steps below install both binaries in `~/.local/bin`, which doesn't need root
permissions. Keep `fbk` and `fbkd` in the same directory, because the CLI starts
the daemon from its own directory.

### 1. Download and install the binaries

The commands in this step use bash or zsh syntax. If you use fish, run `bash`
first and run them in that shell.

Set the release to install and pick the target for your machine:

```sh
VERSION=v0.3.0
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
curl -fLO "https://github.com/wmeints/anvil/releases/download/$VERSION/$NAME.tar.gz"
curl -fLO "https://github.com/wmeints/anvil/releases/download/$VERSION/$NAME.tar.gz.sha256"
shasum -a 256 -c "$NAME.tar.gz.sha256"   # or: sha256sum -c "$NAME.tar.gz.sha256"
```

Extract the archive and copy both binaries to `~/.local/bin`:

```sh
tar -xzf "$NAME.tar.gz"
mkdir -p ~/.local/bin
install -m 755 "$NAME/fbk" "$NAME/fbkd" ~/.local/bin/
```

### 2. Add `~/.local/bin` to your `PATH`

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

### 3. Verify the installation

```sh
fbk --version
fbk ls
```

`fbk --version` prints the installed version, for example `fbk 0.3.0`.

`fbk ls` starts `fbkd`, which installs the microsandbox runtime in
`~/.microsandbox`, and lists your sandboxes (none yet). An error here means
`fbkd` couldn't start, for example because it isn't next to `fbk`.

### macOS: remove the quarantine flag

The macOS binaries aren't signed. When you download the archive through a
browser instead of `curl`, macOS quarantines the binaries and refuses to run
them. Remove the quarantine flag:

```sh
xattr -d com.apple.quarantine ~/.local/bin/fbk ~/.local/bin/fbkd
```

### Upgrading and uninstalling

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

### Migrating from Anvil

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

### Building from source

With the [development toolchain](#development) installed, `cargo install` puts
both binaries in `~/.cargo/bin`. Make sure that directory is on your `PATH`, as
in step 2:

```sh
cargo install-cli     # cargo install --locked --path crates/cli
cargo install-daemon  # cargo install --locked --path crates/daemon
```

## Usage

Run the commands from your project directory. Firebrick derives the sandbox from
that directory. Pass a name from `fbk ls` to `start`, `stop` or `rm` to manage
another sandbox from any directory.

| Command                           | Description                                                                                                                        |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- |
| `fbk start [name]`                | Start the sandbox for the current directory, or the existing sandbox with the name from `fbk ls`.                                  |
| `fbk run <cmd> [args...]`         | Start the sandbox when needed and run a command in it with a terminal attached.                                                    |
| `fbk stop [name]`                 | Stop the sandbox, or the one with the name from `fbk ls`. Files on its disk are kept. Killed when it doesn't shut down within 30s. |
| `fbk ls [--format json]`          | List all sandboxes as a table, or as JSON with `--format json`.                                                                    |
| `fbk rm [--force] [name]`         | Remove the sandbox, or the one with the name from `fbk ls`. Refuses a running sandbox; `--force` stops it first.                   |
| `fbk validate`                    | Check the `.firebrick.yml` file in the current directory.                                                                          |
| `fbk init [--force]`              | Write a `.firebrick.yml` with the defaults to the current directory. `--force` overwrites an existing one.                         |
| `fbk secret set <name> [<value>]` | Set a secret for all sandboxes. See [Secrets](#secrets).                                                                           |
| `fbk secret ls [--format json]`   | List the secrets and their allowed hosts, without their values.                                                                    |
| `fbk secret rm <name>`            | Remove a secret from all sandboxes.                                                                                                |

For example, to open a shell in the sandbox:

```sh
fbk run bash
```

The sandbox keeps running after the command exits.

When the project pins its tools with [mise](https://mise.jdx.dev), Firebrick
installs them when `fbk start` or `fbk run` creates or starts the sandbox, but
not when an SSH connection starts it. It trusts the `mise.toml`, `.mise.toml`,
`mise/config.toml`, `.config/mise.toml` and `.tool-versions` files at the root
of the project and runs `mise install`, so `fbk start` returns with the tools
ready. When `mise install` fails, `fbk start` prints mise's error and the
sandbox keeps running, so you can connect and fix the config. Images without
mise skip this step. Set `mise: false` in `.firebrick.yml` to turn it off.

### Configuring a sandbox

Add a `.firebrick.yml` file to the project directory to configure the sandbox.
`fbk init` creates one with the defaults from the table below, named after the
project directory. In a directory called `My.App`, it writes:

```yaml
name: my-app
resources:
  cpu: 2
  memory: 4 GiB
image: ghcr.io/wmeints/firebrick-base:v<version>
init: true
mise: true
volumes:
  docker: 20 GiB
```

`<version>` is the installed firebrick version. Edit the file to change any of
the fields, such as `image` to run your own image.

| Field              | Description                                                     | Default                                     |
| ------------------ | --------------------------------------------------------------- | ------------------------------------------- |
| `name`             | Name of the sandbox.                                            | Required                                    |
| `image`            | OCI image the sandbox runs.                                     | `ghcr.io/wmeints/firebrick-base:v<version>` |
| `init`             | Run the image's `/sbin/init` as PID 1. See below.               | `true`                                      |
| `mise`             | Trust and install the project's mise tools when it starts.      | `true`                                      |
| `resources.cpu`    | Number of vCPUs.                                                | `2`                                         |
| `resources.memory` | Memory in `Mi`/`MiB` or `Gi`/`GiB`, such as `512 MiB` or `4Gi`. | `4 GiB`                                     |
| `volumes.docker`   | Size of the Docker data disk, in the same units as `memory`.    | `20 GiB`                                    |

Without `.firebrick.yml`, Firebrick uses the defaults and names the sandbox
`firebrick-` followed by the first 6 characters of the SHA-256 hash of the full
path of the working directory, such as `firebrick-d9f287`. A sandbox created by
an older version keeps its name, such as `home_user_my_project`.

Two directories can, rarely, hash to the same name. `fbk start` and `fbk run`
then refuse to use the first directory's sandbox from the second one, so an
agent can't reach the other project's files:

```text
Error: sandbox firebrick-d9f287 belongs to /home/user/my-project; add a .firebrick.yml with its own name to give this directory a separate sandbox
```

Add a `.firebrick.yml` with its own `name` to the second directory to give it a
separate sandbox.

Every sandbox gets a private ext4 disk mounted at `/var/lib/docker`, so Docker
can store images and containers inside the sandbox; Docker's storage doesn't
work on the sandbox's overlayfs root filesystem. The disk keeps its contents
when the sandbox stops, and `fbk rm` deletes it with the sandbox.

The image, init, mise setting, resources and volumes apply when the sandbox is
created. To change them for an existing sandbox, run `fbk rm` and start it
again. Sandboxes created by an older version have no Docker data disk until you
recreate them.

### Secrets

Give agents tokens without letting the real values into the sandbox:

```sh
gh auth token | fbk secret set GH_TOKEN --from-stdin
fbk secret set ANTHROPIC_API_KEY --from-stdin < ~/anthropic-key.txt
fbk secret set MY_TOKEN --from-stdin --allow-host api.example.com
```

In the sandbox, the environment variable holds a placeholder such as
`$MSB_GH_TOKEN`. When a request to one of the secret's allowed hosts carries the
placeholder in an HTTP header, the host replaces it with the real value.
Requests that carry it to other hosts are blocked. Use `--from-stdin` rather
than the value as an argument, so the value stays out of your shell history.

These names have default allowed hosts. Other names need `--allow-host`, which
you can repeat:

| Name                                           | Allowed hosts                                         |
| ---------------------------------------------- | ----------------------------------------------------- |
| `GH_TOKEN`, `GITHUB_TOKEN`                     | `github.com`, `api.github.com`, `uploads.github.com`  |
| `COPILOT_GITHUB_TOKEN`                         | `github.com`, `api.github.com`, `*.githubcopilot.com` |
| `ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN` | `api.anthropic.com`                                   |

Secrets apply to all sandboxes. A running sandbox gets a new or changed secret
after `fbk stop` and `fbk start`.

`fbk secret ls` shows the names and allowed hosts of the secrets, never their
values. `fbk secret rm <name>` removes a secret, but a running sandbox keeps
using it until it restarts. If a token leaked, revoke it where you created it as
well.

To let `git` push and pull over HTTPS with `GH_TOKEN`, configure a credential
helper in the sandbox that hands git the placeholder as the password:

```sh
git config --global credential.https://github.com.helper \
  '!f() { test "$1" = get && echo username=x-access-token && echo "password=$GH_TOKEN"; }; f'
```

git sends the placeholder base64-encoded in a Basic `Authorization` header, and
the host decodes it, replaces the placeholder and encodes it again. If `gh` is
installed in the sandbox, `gh auth setup-git` configures an equivalent helper.

Keep in mind that:

- Don't use SSH keys for git: the sandbox's SSH server doesn't support agent
  forwarding (`ssh -A`), and copying a private key into the sandbox puts the
  real key where the agent can read it. Use git over HTTPS instead.
- The values are stored unencrypted in files only your user can read. If your
  machine may be compromised, rotate the tokens where you created them and set
  the new values.

### Connecting over SSH

`fbkd` generates SSH keys and an SSH config, and includes that config in
`~/.ssh/config`. Each sandbox gets a host name after its project directory, such
as `my-project.fbk`. When two projects share a directory name, the second gets
`my-project-2.fbk`:

```sh
ssh my-project.fbk
```

Point your IDE's remote SSH support at the same host name to work in the sandbox
from your editor. For VS Code, VS Code Insiders, Cursor and VSCodium, `fbkd`
also registers each host as a Linux host in the Remote-SSH settings, and `fbk
start` prints a command that opens the workspace in the sandbox:

```sh
$ fbk start
Connect with: ssh my-project.fbk
Open in VS Code: code --folder-uri vscode-remote://ssh-remote+my-project.fbk/workspaces/my-project
```

Run the printed `code` command, or pick the host in Remote-SSH and open
`/workspaces/my-project`.

For Zed, `fbkd` adds each sandbox with its workspace to the `ssh_connections` in
Zed's settings, and `fbk start` prints a command that opens it:

```sh
Open in Zed: zed ssh://my-project.fbk/workspaces/my-project
```

Run the printed `zed` command, or pick the sandbox in Zed's Remote Projects
dialog.

### Base image

The [`Dockerfile`](Dockerfile) describes a base image for sandboxes with `git`,
`curl`, `sudo`, [mise](https://mise.jdx.dev), the Docker engine and an
unprivileged `agent` user. Releases publish it as
`ghcr.io/wmeints/firebrick-base:<tag>`, and sandboxes run the image that matches
the installed firebrick version unless the `image` field in `.firebrick.yml`
names another one, such as an image built on top of it.

The image's init starts `dockerd` when the sandbox boots, and `agent` is in the
`docker` group, so agents can run `docker`, `docker compose` and `docker buildx`
without `sudo`. Docker runs directly on the sandbox VM and keeps its images and
containers on the sandbox's own disk at `/var/lib/docker`, so they survive `fbk
stop` and `fbk start`. When `dockerd` fails to start, the reason is in
`/var/log/dockerd.log`. With `init: false`, nothing starts `dockerd` and
`docker` reports `Cannot connect to the Docker daemon`; start it in the
background with `sudo sh -c 'dockerd >/var/log/dockerd.log 2>&1 &'`.

### Bringing your own image

Firebrick runs everything in a sandbox as the `agent` user. A custom image must:

- Have a user named `agent` with UID `1000` and GID `1000` and a home directory,
  such as `/home/agent`. Images based on Ubuntu ship an `ubuntu` user with UID
  1000; remove it first.
- Set `USER agent`. `fbk run` runs commands as the image's user, while SSH
  always logs in as `agent`.
- Install `sudo` and allow `agent` to use it without a password, if agents
  should be able to install system packages.
- Provide an executable `/sbin/init`, or set `init: false` in `.firebrick.yml`.
  With `init` on, which is the default, Firebrick runs `/sbin/init` as PID 1,
  and `fbk start` fails with a hint when the image has none. The base image's
  init disables guest IPv6, starts `dockerd` in the background, and then runs
  [tini](https://github.com/krallin/tini) to reap zombie processes. It works
  around a microsandbox bug that resets IPv6 connections on hosts without IPv6
  internet access
  ([microsandbox#1226](https://github.com/superradcompany/microsandbox/issues/1226)).
  Images built on the base image inherit it; with `init: false`, such hosts
  can't download from servers that have an IPv6 address, and `dockerd` doesn't
  run until you start it with `sudo sh -c 'dockerd >/var/log/dockerd.log 2>&1
  &'`.
- Add `agent` to the `docker` group and start `dockerd` from `/sbin/init`, if
  agents should be able to run `docker` without `sudo`. Firebrick attaches a
  disk for Docker's data at `/var/lib/docker` to every sandbox.

The workspace is mounted at `/workspaces/<project>`, and its files show up as
owned by `agent`, whatever the UID of your user on the host is.

Sandboxes created by an older version of Firebrick run `ubuntu:26.04`, which has
no `agent` user, so SSH can no longer log in to them. Recreate them with `fbk
rm` and `fbk start`, or connect with `ssh root@<name>.fbk`.

The simplest way to meet these requirements is to build on the base image:

```dockerfile
FROM ghcr.io/wmeints/firebrick-base:v0.3.0

USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3 \
    && rm -rf /var/lib/apt/lists/*
USER agent
```

For another distribution, create the user yourself, and set `init: false` in
`.firebrick.yml` or add an init like the base image's:

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

Install the toolchain (Rust, `buf`, `dprint`, `actionlint` and `lefthook`) with
[mise](https://mise.jdx.dev). This also installs the git hooks:

```sh
mise install
```

Building also needs `protoc` 3.15 or newer, which supports proto3 optional
fields. The `protobuf-compiler` package on older distributions, such as Ubuntu
22.04, is too old; install a current release from the
[protobuf releases](https://github.com/protocolbuffers/protobuf/releases)
instead.

| Command                   | Description                                   |
| ------------------------- | --------------------------------------------- |
| `cargo build`             | Build the `fbk` and `fbkd` binaries.          |
| `cargo unit-tests`        | Run the unit tests.                           |
| `cargo integration-tests` | Run the integration tests that boot real VMs. |
| `cargo lint`              | Run the linter.                               |
| `cargo fmt --all`         | Format the code.                              |

Cargo commands in this repository use `/tmp/firebrick-msb` as the microsandbox
home (`MSB_HOME`, set in `.cargo/config.toml`), so the integration tests don't
share a runtime or database with your own `~/.microsandbox`.

The default sandbox image is the `firebrick-base` image of the same release, so
it doesn't exist for a version that hasn't been released yet. To run a
development build, push an image built from the [`Dockerfile`](Dockerfile) to a
local registry and set `image` in `.firebrick.yml` to it:

```sh
docker run -d -p 127.0.0.1:5000:5000 --name registry registry:2
docker build -t localhost:5000/firebrick-base:dev .
docker push localhost:5000/firebrick-base:dev
```

The local registry speaks plain HTTP, so allow it in
`~/.microsandbox/config.json` before `fbkd` starts:

```json
{ "registries": { "hosts": { "localhost:5000": { "insecure": true } } } }
```

The workspace contains five crates:

| Crate              | Folder          | Purpose                                                                  |
| ------------------ | --------------- | ------------------------------------------------------------------------ |
| `firebrick-cli`    | `crates/cli`    | The `fbk` CLI.                                                           |
| `firebrick-daemon` | `crates/daemon` | The `fbkd` daemon.                                                       |
| `firebrick-proto`  | `crates/proto`  | Generated gRPC code for the daemon API.                                  |
| `firebrick-spec`   | `crates/spec`   | Parses and validates `.firebrick.yml`.                                   |
| `firebrick-utils`  | `crates/utils`  | Shared paths for the socket, logs and SSH files, and the name sanitizer. |

The gRPC contract lives in
[`crates/proto/proto/daemon.v1.proto`](crates/proto/proto/daemon.v1.proto).

## Documentation

- [Architecture](docs/architecture/01-introduction-and-goals.md) - the arc42
  architecture documentation.
- [Decisions](docs/architecture/decisions/) - architecture decision records.
- [CLAUDE.md](CLAUDE.md) - coding guidelines and the definition of done.

## License

Firebrick is licensed under the [MIT License](LICENSE).
