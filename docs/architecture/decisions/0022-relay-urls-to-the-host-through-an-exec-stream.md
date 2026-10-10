# 22. Relay URLs to the host through an exec stream

## Status

Accepted

## Context

Coding agents and CLIs in a sandbox call `xdg-open` or `$BROWSER` to show a web
page or to start a login flow, such as `gh auth login --web` or Claude Code's
login. The guest has no browser, so these calls fail. The URL has to reach
`fbkd`, which can open it in the user's browser on the host, for every way of
entering a sandbox: `fbk run`, `ssh <leaf>.fbk` and IDEs over SSH.

microsandbox offers no guest-to-host channel that works for existing sandboxes:

- The network policy is fixed when the sandbox is created. Allowing
  `host.microsandbox.internal` so the guest could call an HTTP endpoint of
  `fbkd` would also expose every other service that listens on the host's
  loopback, and would mean recreating every existing sandbox.
- vsock routes are fixed at create time too.

What `fbkd` can always do is run a process in a running sandbox and read its
output with `exec_stream_with`, the same way `fbk run` sessions work.

## Decision

- `fbkd` starts one relay per running sandbox as an exec stream: an inline `sh
  -c` script that creates the FIFO `/tmp/.firebrick/open.fifo` and echoes every
  line written to it. Because the script is inline, it also works in images that
  don't ship the stand-in, and the relay doesn't need root.
- The relay starts on `StartSandbox`, `Attach` and `SshTunnel`, the calls that
  show the sandbox is in use. A registry in `SandboxManager` keyed by sandbox
  name keeps it to one relay per sandbox; an entry goes away when its exec
  stream ends or the sandbox stops.
- The `firebrick-base` image installs `firebrick-open` as `xdg-open` and
  `BROWSER`. It writes a URL to the FIFO and gives up after 5 seconds when no
  relay reads it, printing the URL so the user can open it.
- The stand-in and `fbkd` both accept only one `http://` or `https://` URL
  without whitespace or control characters; `fbkd` also limits it to 8 KiB. It
  opens the URL with `xdg-open` or `open` through `std::process::Command`,
  without a shell. The check is a few lines of string handling, so no URL
  parsing crate is added.

## Consequences

- `xdg-open https://...` works in the sandbox without any change to the network
  policy or to existing sandboxes, once they run the new image.
- Any process in the sandbox can make the host open an http or https URL in the
  browser, without asking. The browser is no more exposed than when the user
  clicks a link, but a misbehaving agent can open tabs. Asking first and
  allow-lists are left for later.
- The relay is one extra `sh` process per running sandbox and holds an agent
  connection while the sandbox runs.
- A sandbox that is running while `fbkd` restarts has no relay until the next
  `StartSandbox`, `Attach` or `SshTunnel`. URLs written in between time out in
  the stand-in, which prints them for the user.
- Callbacks to `localhost:<port>` after a login in the host browser don't reach
  the sandbox yet; forwarding those ports builds on this relay.
