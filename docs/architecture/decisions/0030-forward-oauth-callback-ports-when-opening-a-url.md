# 30. Forward OAuth callback ports when opening a URL

## Status

Accepted

## Context

OAuth logins started in a sandbox, such as `gh auth login --web`, Claude Code's
login or cloud CLIs, run a callback server on the guest's `localhost:<port>` and
send the browser to a URL with `redirect_uri=http://localhost:<port>/...`.
`fbkd` opens that URL in the browser on the host
([ADR 0026](0026-relay-urls-to-the-host-through-an-exec-stream.md)), so the
callback hits the host's `localhost:<port>`, where nothing listens, and the
login never completes. The port is often random and only known when the URL is
opened, so it can't be listed in `.firebrick.yml` up front.

The host port has to reach the guest's loopback, on a sandbox that already runs:

- microsandbox's published ports are fixed when the sandbox is created and
  target the guest's interface address, not its loopback, where callback servers
  listen.
- The raw agent protocol that microsandbox speaks with agentd has no stable API
  for TCP connections; using it would tie `fbkd` to microsandbox internals.
- microsandbox's SSH server opens `direct-tcpip` channels from inside the guest,
  and `fbkd` already forwards the ports of `.firebrick.yml` through it
  ([ADR 0019](0019-forward-ports-through-the-ssh-servers-direct-tcpip.md)).

## Decision

Before `fbkd` opens a URL from a sandbox, it forwards the loopback ports the URL
names into the sandbox, with the `direct-tcpip` forwards of the `forward`
module:

- It forwards the port of the URL's own host when that is `localhost`,
  `127.0.0.1` or `[::1]` with an explicit, non-zero port, and the same for the
  URL in a percent-decoded `http` `redirect_uri` query parameter. The host port
  maps to the same port in the sandbox.
- The listener is bound before the browser opens, so the callback can't arrive
  first.
- Each relay owns its callback forwards and their SSH session, separate from the
  configured forwards, so reconciling `.firebrick.yml` doesn't close them. A
  port that the configured forwards already map to the same guest port uses that
  forward instead. They close when the relay ends or is stopped because the
  sandbox stopped, when `fbkd` exits, and after 10 minutes without open or new
  connections or new URLs for the port.
- A URL whose own host is loopback reaches the sandbox, so it skips the local
  host and egress checks, but it only opens when its forward is open: otherwise
  the browser would reach a service of the host. A busy `redirect_uri` port is
  logged and the URL still opens, because the login may still finish another
  way, such as by pasting the code.

## Consequences

- OAuth logins and dev-server URLs opened from a sandbox work without
  configuring ports.
- A sandbox can make the host listen on a loopback port of its choice for up to
  10 minutes, and any process of the host user can connect to it, the same as
  with configured forwards. The rate limit of 5 URLs per 10 seconds bounds how
  fast it can claim ports.
- When the callback port is busy on the host, the browser sends the callback to
  the host's own service on that port, and `fbkd` logs a warning that says so.
- While a callback forward holds a host port, adding that port to the sandbox's
  configured ports fails with the port in use until the callback forward closes.
- Callback URIs in other parameters, such as `redirect_url`, and guest servers
  that listen only on `::1` aren't forwarded.
