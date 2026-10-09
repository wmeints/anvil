---
name: smoke-test
description: "Run the anvil CLI and anvild daemon end to end in an isolated environment, against real microsandbox VMs, without touching the user's daemon, sandboxes or config. Use to try a change by hand, reproduce a bug, check `anvil start`/`run`/`ssh` behavior, or boot a locally built sandbox image."
---

# Smoke test

Try `anvil` and `anvild` for real in a throwaway environment. The user usually
has their own `anvild` running, sandboxes in `~/.microsandbox`, and an anvil
`Include` in `~/.ssh/config`. A smoke test must never touch any of them.

`smoke-env.sh` next to this file creates the environment and cleans it up. It
puts everything under a short `/tmp/anvil-smoke.XXXX` directory, because unix
socket paths are limited to 108 bytes and the scratchpad path is too long:

| Variable          | Isolates                                                  |
| ----------------- | --------------------------------------------------------- |
| `XDG_RUNTIME_DIR` | the daemon socket, so the CLI starts its own `anvild`     |
| `MSB_HOME`        | microsandbox's runtime, database, images and sandboxes    |
| `XDG_DATA_HOME`   | SSH keys, the generated SSH config and `secrets.yml`      |
| `XDG_STATE_HOME`  | the daemon log                                            |
| `XDG_CONFIG_HOME` | the VS Code-family and Zed settings anvild syncs on Linux |
| `HOME`            | the `Include` line anvild adds to `~/.ssh/config`         |

## Steps

### 1. Build

```bash
cargo build
```

The CLI starts the `anvild` that sits next to its own binary, so
`target/debug/anvil` always runs the daemon you just built.

### 2. Create the environment

```bash
.claude/skills/smoke-test/smoke-env.sh up
```

It prints the path of an env file, such as `/tmp/anvil-smoke.Ab12/env`. Shell
state doesn't carry over between commands, so start **every** command that runs
`anvil`, `msb` or `ssh` with `source <env file>`. The env file sets:

- `$ANVIL` - the CLI from `target/debug`.
- `$MSB` - microsandbox's `msb` in the isolated home. It exists after the first
  `anvil` command, because anvild installs the runtime when it starts.
- `$SMOKE_WORK` - the directory for workspaces.
- `$SMOKE_SSH_CONFIG` - the generated SSH config.

### 3. Create a workspace

Make a directory per scenario under `$SMOKE_WORK`. Its name is the leaf of the
workspace path, so `$SMOKE_WORK/demo` gets the host name `demo.anvil`. Add an
`.anvil.yml` when the scenario needs a name, image or setting:

```bash
source /tmp/anvil-smoke.Ab12/env
mkdir -p "$SMOKE_WORK/demo" && cd "$SMOKE_WORK/demo"
printf 'name: smoke-demo\nimage: ubuntu:26.04\ninit: false\n' > .anvil.yml
```

Pick the image deliberately:

- **No `image`**: the default `ghcr.io/wmeints/anvil-base:v<version>`. This only
  works when that version is published, so not after a version bump.
- **`ubuntu:26.04`**: boots fast, but needs `init: false` because it has no
  `/sbin/init`. It also has no `agent` user, so SSH fails with `guest user not
  found: agent`.
- **A locally built image**: see "Test a local image" below.

A fresh `MSB_HOME` has no image cache, so the first `anvil start` pulls the
image. That takes up to a minute.

### 4. Run the scenario

Run the smallest set of commands that shows the behavior. Use `timeout` so a
hang doesn't block the session, and give `anvil run` a non-terminal stdin:

```bash
source /tmp/anvil-smoke.Ab12/env && cd "$SMOKE_WORK/demo"
timeout 300 "$ANVIL" start; echo "exit=$?"
"$ANVIL" ls
timeout 60 "$ANVIL" run -- sh -c 'id; pwd' </dev/null; echo "exit=$?"
timeout 60 ssh -F "$SMOKE_SSH_CONFIG" -o BatchMode=yes demo.anvil 'whoami'
```

Pass `-F "$SMOKE_SSH_CONFIG"` to `ssh`. OpenSSH reads `~/.ssh` from the password
database, not from `$HOME`, so it never sees the isolated config by itself.

When something fails, read the logs:

- the daemon: `$XDG_STATE_HOME/anvil/anvild.log.*`
- a sandbox: `$MSB_HOME/sandboxes/<name>/logs/`
- microsandbox directly: `"$MSB" ls`, `"$MSB" logs <name>`

**After changing the daemon code**, rebuild and stop this environment's daemon,
so the next `anvil` command starts the new build. Sandboxes keep running while
the daemon restarts.

```bash
cargo build && source /tmp/anvil-smoke.Ab12/env && kill $(lsof -t "$XDG_RUNTIME_DIR/anvild.sock")
```

### 5. Clean up

Always clean up, also when the scenario failed:

```bash
.claude/skills/smoke-test/smoke-env.sh down /tmp/anvil-smoke.Ab12/env
```

This stops and removes every sandbox in the environment, stops its daemon and
deletes the directory. It refuses any path that isn't a smoke-test environment.

### 6. Report

Tell the user which commands you ran, with their real output and exit codes, and
which image the sandbox used.

## Test a local image

To test changes to the `Dockerfile` before the image is published, build it with
Docker and load it into the isolated microsandbox home. No registry is needed.
Run this from the repository root:

```bash
source /tmp/anvil-smoke.Ab12/env
docker build -q -t anvil-base:smoke .
docker save anvil-base:smoke -o "$SMOKE_ROOT/image.tar"
"$MSB" load -q -i "$SMOKE_ROOT/image.tar" -t anvil-base:smoke
```

`$MSB` only exists after anvild has started once, so run `"$ANVIL" ls` first in
a new environment. Then use `image: anvil-base:smoke` in the workspace's
`.anvil.yml`. After cleaning up, remove the Docker tag with `docker rmi
anvil-base:smoke`.

## Guardrails

- **Never touch the user's environment.** Don't stop or kill an `anvild` you
  didn't start, don't run `anvil` or `msb` without sourcing the env file, and
  don't change `~/.microsandbox`, `~/.local/share/anvil` or `~/.ssh/config`.
- **Never kill daemons by name.** `pkill anvild` also stops the user's daemon.
  Stop only the process that listens on the environment's socket.
- **Don't leave sandboxes behind.** Run step 5 before you finish the turn.
- **Report real output.** If a step couldn't run, say so instead of assuming it
  would have passed.
