# Crosscutting concepts

## Error reporting in the daemon

The daemon doesn't send the errors of microsandbox, the secret store or the SSH
server to the client. It replaces them with a short, generic message, such as
`failed to remove sandbox`, and a matching gRPC status code. Wherever it does
that, it first logs the original error with `tracing::error!(error = ?err,
"<message>")`, naming the sandbox, secret or host name involved. The error is
logged with its `Debug` representation, because the `Display` representation of
these errors leaves out their source, which is what's needed to debug an
internal error. The log is written to the console and to `fbkd.log` in the
daemon's log directory.

A failure the user can cause and fix, such as connecting to a stopped sandbox,
is logged as a warning instead, so it doesn't hide real internal errors.

Because the `Debug` representation includes every source, an error type must not
carry secret values anywhere in its chain. serde_yaml quotes the values it can't
parse, so a parse error of the secrets file only keeps the line and column of
the error, not the serde_yaml error itself.

The `vm-tests` in `crates/daemon/tests/grpc.rs` run the daemon in-process and
install a `tracing` subscriber that writes through libtest's output capture, at
level `info` unless `RUST_LOG` says otherwise. A failing test prints the
daemon's log lines, including the original error behind a generic status, next
to its panic message; a passing test prints nothing extra. The tests use the
current-thread runtime, so the daemon logs on the test's own thread and its
lines end up under the right test.

## Secret scopes

Each secret in `secrets.yml` is global or belongs to one sandbox, through an
optional `sandbox` field
([ADR 0022](decisions/0022-scope-secrets-per-sandbox.md)). An entry without it
is global, so files from before scopes load unchanged. A name is unique within a
scope, but the same name may exist globally and for any number of sandboxes.

| Change                         | Sandboxes it applies to                                                                                                                                                                    |
| ------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Set or remove a global secret  | Every sandbox firebrick created, except the ones with a sandbox-scoped secret with that name.                                                                                              |
| Set a sandbox-scoped secret    | Its sandbox only, replacing the global secret with that name there.                                                                                                                        |
| Remove a sandbox-scoped secret | Its sandbox only, which gets the global secret with that name back when there is one.                                                                                                      |
| Create or recreate a sandbox   | The global secrets, with the sandbox's own sandbox-scoped secrets in their place. A brand-new sandbox has none, because a sandbox-scoped secret can only be set for a sandbox that exists. |
| Remove a sandbox               | `fbkd` removes its sandbox-scoped secrets from `secrets.yml`, so a later sandbox with the same name doesn't inherit them.                                                                  |

So in a sandbox, a sandbox-scoped secret wins over the global secret with the
same name. Like every secret change, these apply the next time a sandbox starts.

## Securing the daemon socket

Any process that can use the daemon socket can start sandboxes, read workspace
files through them and manage secrets, so the daemon restricts the socket to the
user it runs as, in two layers:

- After binding the socket, `server` sets its mode to `0600`, whatever the
  umask. This matters most when the socket falls back to the shared temp dir
  because `XDG_RUNTIME_DIR` isn't set. When setting the mode fails, the daemon
  removes the socket and exits instead of serving on an unprotected socket.
- For every accepted connection, `server` reads the peer's credentials with
  `SO_PEERCRED` and hands the connection to tonic only when the peer's UID is
  the daemon's effective UID or `0`. The daemon's UID is read from the owner of
  the socket it just bound, so it needs no extra dependency. Any other
  connection, and a connection whose credentials can't be read, is closed before
  it serves a request, and the daemon logs a warning with the peer's UID and PID
  or the I/O error. It keeps accepting other connections.

Root is allowed on purpose: root can bypass the check anyway, for example by
reading the daemon's memory, so rejecting it adds no security and only gets in
the way of an administrator.
