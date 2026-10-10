# Crosscutting concepts

## Error reporting in the daemon

The daemon doesn't send the errors of microsandbox, the secret store or the SSH
server to the client. It replaces them with a short, generic message, such as
`failed to remove sandbox`, and a matching gRPC status code. Wherever it does
that, it first logs the original error with `tracing::error!(error = ?err,
"<message>")`, naming the sandbox, secret or host name involved. The error is
logged with its `Debug` representation, because the `Display` representation of
these errors leaves out their source, which is what's needed to debug an
internal error. The log is written to the console and to `anvild.log` in the
daemon's log directory.
