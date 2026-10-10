---
name: sandbox-reference
description: "Look up how the microsandbox crate behaves before relying on it: its API for creating, starting, modifying and exec'ing into sandboxes, secrets, network policy, SSH, runtime setup and on-disk state. Use when changing code in crates/daemon that calls microsandbox, when designing a feature that depends on what microsandbox can or can't do, or when debugging a microsandbox error."
---

# Sandbox reference

`fbkd` drives sandboxes through the `microsandbox` crate. Its docs site doesn't
cover everything the code relies on, so answer questions from the crate source
of the exact version in `Cargo.lock`, and start from the facts below instead of
rediscovering them.

## Find the source

Read the version from the lockfile, never from `crates/daemon/Cargo.toml` (it
holds the minimum version) or from a version you remember:

```bash
V=$(grep -A1 '^name = "microsandbox"$' Cargo.lock | sed -n 's/^version = "\(.*\)"/\1/p')
R=$(ls -d ~/.cargo/registry/src/*/ | head -1)
M=${R}microsandbox-$V/lib            # the SDK
N=${R}microsandbox-network-$V/lib    # network policy, secrets proxy, TLS
```

Run `cargo fetch` if the directories don't exist. Search with `grep -rn` and
read with the Read tool. To answer a broad question, give an `Explore` agent the
paths above.

| Topic                           | Where                                                           |
| ------------------------------- | --------------------------------------------------------------- |
| `Sandbox` (get, list, remove)   | `$M/sandbox/mod.rs`                                             |
| Create-time settings            | `$M/sandbox/builder.rs` (`SandboxBuilder`)                      |
| Start, stop, status of a handle | `$M/sandbox/handle.rs` (`SandboxHandle`)                        |
| Change an existing sandbox      | `$M/sandbox/modify.rs`                                          |
| Exec and PTY sessions           | `$M/sandbox/exec.rs`, `exec_stream_with` in `$M/sandbox/mod.rs` |
| SSH server                      | `$M/sandbox/ssh.rs`                                             |
| Runtime install and version     | `$M/setup/`                                                     |
| Errors                          | `$M/error.rs` (`MicrosandboxError`)                             |
| Home dir, db and config paths   | `${R}microsandbox-utils-$V/lib/lib.rs`, `$M/config/`            |
| Network rules                   | `$N/model/policy/` (`builder.rs`, `types.rs`)                   |
| Secret substitution in requests | `$N/engine/secrets/handler.rs`                                  |

## How firebrick uses it

All calls live in `crates/daemon/src`. Reuse these before adding new ones:

- `sandboxes.rs` - `Sandbox::builder(..).image().cpus().memory().label()
  .volume()` and `.init(..)` to create, `Sandbox::get`, `Sandbox::list_with`
  (paged, filtered by label), `connect_or_start_detached`, `stop`, `remove`,
  `status_snapshot`, and `modify().label(..)`.
- `secrets.rs` - `builder.secret(..)` at create time, and `modify().secret(..)`
  / `modify().remove_secret(..)` on existing sandboxes.
- `session.rs` - `exec_stream_with(command, |e| e.args(..).stdin_pipe()
  .tty(true))` for `fbk run`.
- `tunnel.rs` / `ssh.rs` - `microsandbox::sandbox::ssh::SshServer` behind `fbk
  ssh-proxy`.
- `runtime.rs` - `setup::ensure_runtime`, `setup::install_runtime` and
  `setup::resolve_runtime_version` with the runtime embedded through the
  `embed-binaries` feature.

## Known behavior

Verified against microsandbox 0.7.7. Recheck an item in the source before you
build on it when `Cargo.lock` has a newer version.

**Lifecycle**

- `start_detached()` on a sandbox that is already running fails with
  `MicrosandboxError::SandboxStillRunning`. `connect_or_start_detached()`
  connects to a running sandbox, waits for a starting one, starts a stopped one,
  absorbs that race, and rejects draining and paused sandboxes.
- `Sandbox::list_with` rejects an empty cursor. Only call `.cursor(..)` with the
  cursor of the previous page.
- `.init(path)` hands PID 1 to an init binary in the image after agentd's setup.
  `.init("auto")` probes common init paths and refuses to boot an image that has
  none, so firebrick passes an explicit path (ADR 0007).

**Changing existing sandboxes**

- `modify()` changes cpus, memory, disk size, env, labels, workdir and secrets.
  Use `.next_start()` or `.restart()` to choose when changes apply.
- The network policy has no `modify()` setting: it's fixed when the sandbox is
  created. Changing rules means recreating the sandbox.

**Secrets**

- The guest only sees a placeholder such as `$MSB_GH_TOKEN`. The host proxy
  swaps in the real value on requests to the secret's allowed hosts, including
  inside decoded `Authorization: Basic` credentials, so git over HTTPS works.
- `SecretSource::Store` (a host keyring) isn't implemented and returns an error.
  Use `.value(..)` or `SecretSource::Env` (ADR 0004).

**Network**

- Rules are an ordered list and the first match decides. Destinations are
  `.ip(..)`, `.cidr(..)`, `.domain(..)` and `.domain_suffix(..)`, which also
  matches subdomains. There is no `*.example.com` wildcard syntax.
- On hosts without IPv6 egress, the guest still gets IPv6 and its IPv6
  connections are reset. `firebrick-base` disables guest IPv6 (ADR 0007).

**SSH**

- The SSH server needs no `sshd` in the guest and supports exec, PTY, SFTP and
  `direct-tcpip` port forwarding. It doesn't support agent forwarding (see
  `docs/architecture/11-risks-and-technical-debt.md`).

**Terminals**

- A TTY exec without a `TERM` env gets the host process's `TERM` (or `xterm`
  when it's unset or `dumb`), so `fbkd` passes on the terminal it was started
  from. `session.rs` sets `TERM` explicitly.
- The SSH server sets `TERM` in the guest from the client's PTY request
  (`ssh.rs`), so the host side can't choose it for SSH sessions.

**Host state**

- State lives in `$MSB_HOME`, or `~/.microsandbox` when it's unset: `bin/msb`,
  `db/msb.db`, `config.json` and `sandboxes/<name>/logs/`.
- Unix sockets live under `MSB_HOME`, and their paths can't be longer than 108
  bytes, so an isolated `MSB_HOME` needs a short path under `/tmp`. The session
  scratchpad is too long.
- A newer `msb` migrates `db/msb.db` in place, and an older crate then fails
  every call with "database schema is newer than this msb binary". In firebrick
  that surfaces as `Internal: failed to list sandboxes`. `.cargo/config.toml`
  sets `MSB_HOME=/tmp/firebrick-msb` for cargo commands so the tests don't use
  the user's database; never move or change `~/.microsandbox` to make them pass.
- To run firebrick or `msb` against real VMs, use the `smoke-test` skill. It
  isolates `MSB_HOME` and the daemon from the user's, and loads locally built
  images with `msb load`, so no registry is needed.

**Debugging**

- `fbkd` maps many errors to a generic gRPC `Internal` status. Read the daemon
  log (`$XDG_STATE_HOME/firebrick/`, by default `~/.local/state/firebrick/`) or
  call the crate directly to see the real `MicrosandboxError`.
- `msb` reproduces behavior without firebrick, for example `msb run <image> --
  <cmd>`, `msb create --name <n> --log-level debug <image>` and `msb exec <n> --
  <cmd>`. Run it as `$MSB` inside a `smoke-test` environment, and remove what
  you create with `msb stop` and `msb rm`.

## Keep this skill current

When you confirm a new behavior or limitation that a later change could depend
on, add it under **Known behavior** in the same change. When a dependency update
changes one of the items, update or remove it.
