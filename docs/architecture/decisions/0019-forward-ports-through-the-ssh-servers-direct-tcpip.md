# 19. Forward ports through the SSH server's direct-tcpip

## Status

Accepted

## Context

Applications in a sandbox, such as a dev server on port 3000, must be reachable
from the browser on the host. Users list the ports in `.firebrick.yml`, and a
changed list must apply to a running sandbox without restarting it.

microsandbox has published ports (`SandboxBuilder::port`), but they don't fit:

- They are fixed when the sandbox is created and `modify()` has no setting for
  them, so a changed list would mean recreating the sandbox.
- They target the guest's interface address, not its loopback. Dev servers often
  listen on `127.0.0.1` only, and would be unreachable.

microsandbox's SSH server, which `fbkd` already uses for `fbk ssh-proxy`,
supports `direct-tcpip` channels. agentd opens the TCP connection from inside
the guest, so the guest's `127.0.0.1` is reachable, and the server runs in the
daemon over an in-memory pipe, so no port in the guest or on the host is
involved.

## Decision

`fbkd` forwards ports itself, in a `forward` module:

- For each mapping it listens on the host's loopback, `127.0.0.1:<host>` and
  `[::1]:<host>` when the host has IPv6 loopback, never on other addresses.
- Each accepted connection gets a `direct-tcpip` channel to `127.0.0.1:<guest>`
  of the sandbox's SSH server, and bytes are copied both ways.
- One SSH session per sandbox carries all channels. `fbkd` serves microsandbox's
  SSH server over a `tokio::io::duplex` pipe and connects a `russh` client to
  it, with a client and host key generated for that session: the pipe never
  leaves the daemon, so the keys don't need to be stored or pinned. The session
  has no inactivity timeout, so configured forwards stay open while the sandbox
  runs.
- The mappings are stored in the sandbox's `firebrick.ports` label, so `fbk
  start <name>`, SSH connections and a restarted `fbkd` can reopen the forwards
  without the spec. The label is updated with a `next_start` modification, which
  doesn't restart a running sandbox.
- `fbkd` reconciles the open forwards with the list on every `StartSandbox`: it
  closes removed ones, opens new ones and leaves unchanged ones and their
  connections alone.

## Consequences

- Changing `ports` and running `fbk start` applies to a running sandbox without
  a restart, and servers that listen only on the guest's `127.0.0.1` are
  reachable.
- The forwards live in the daemon's memory. They close when `fbkd` exits and
  reopen from the labels when it starts again. A sandbox that stops by itself,
  not through `fbk stop`, keeps its host listeners until the next `StartSandbox`
  or `fbk stop`; connections to them close right away.
- Every forwarded byte passes through the SSH session and the agent connection
  of the sandbox, which is slower than a published port. That is fine for dev
  servers and debugging, not for bulk transfers.
- Any process of the host user can connect to the forwarded loopback ports, the
  same as with a dev server running on the host itself.
- Guest servers that listen only on `::1` and UDP aren't reachable.
- The `direct-tcpip` client is reusable for other forwards, such as the OAuth
  callback ports of #56.
