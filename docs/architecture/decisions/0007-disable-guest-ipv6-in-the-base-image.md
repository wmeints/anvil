# 7. Disable guest IPv6 in the base image

## Status

Accepted

## Context

microsandbox gives every guest an IPv6 address, an IPv6 default route and AAAA
DNS answers, even when the host can't reach the internet over IPv6. Its network
proxy completes the guest's TCP handshake before it connects to the real server.
When that upstream IPv6 connect fails, it resets the open connection, so clients
report "connection reset by peer" instead of falling back to IPv4. Every
download from a host with an IPv6 address fails on such hosts, while the same
download works outside the sandbox.

microsandbox 0.7.7 has no setting to turn off guest IPv6
([issue 1226](https://github.com/superradcompany/microsandbox/issues/1226)). The
guest can disable IPv6 with a sysctl, but that needs root at boot, and sandboxes
run as the unprivileged `agent` user
([ADR 0006](0006-run-sandboxes-as-the-agent-user.md)). microsandbox can hand PID
1 to an init binary in the image after its own setup, but it refuses to boot an
image that lacks the init it was told to use.

## Decision

The `anvil-base` image disables guest IPv6 in
`/etc/sysctl.d/99-disable-ipv6.conf` and ships a `/sbin/init` script that
applies those settings and then runs `tini` as PID 1, so orphaned processes are
reaped. A new `init` setting in `.anvil.yml`, `true` by default, controls
whether `anvild` hands PID 1 to the image's `/sbin/init`. It applies to every
image, so a custom image either ships an init or opts out with `init: false`
explicitly, instead of silently missing the workaround.

When the init fails to boot because the image doesn't have one, microsandbox
reports it only as error text. `anvild` matches that text, removes the
half-created sandbox and returns `FAILED_PRECONDITION` with a hint to set `init:
false`. Any other create failure stays a generic internal error.

## Consequences

- Sandboxes from the default image use IPv4 only, also on hosts where IPv6
  works.
- Custom images that don't build on `anvil-base` fail to start until they add an
  init or set `init: false`. With `init: false` they keep guest IPv6, and the
  reset on hosts without IPv6 egress.
- If microsandbox rewords its init error, users get the generic create error
  again instead of the hint, and the failed sandbox stays until `anvil rm`. The
  `vm-tests` cover the message.
- `anvild` and the default image must match: an `anvild` with this change can't
  boot an earlier `anvil-base` image, which has no `/sbin/init`.
  [ADR 0005](0005-default-to-the-anvil-base-image-of-the-same-release.md)
  already pins the default image to the release of `anvild`.
- Sandboxes created before this change keep guest IPv6 until they're recreated
  with `anvil rm` and `anvil start`.
- Remove the workaround once microsandbox can turn off guest IPv6 or falls back
  correctly.
