# 7. Disable guest IPv6 in the base image

## Status

Accepted

## Context

microsandbox gives every guest an IPv6 address, an IPv6 default route and
AAAA DNS answers, even when the host can't reach the internet over IPv6. Its
network proxy completes the guest's TCP handshake before it connects to the
real server. When that upstream IPv6 connect fails, it resets the open
connection, so clients report "connection reset by peer" instead of falling
back to IPv4. Every download from a host with an IPv6 address fails on such
hosts, while the same download works outside the sandbox.

microsandbox 0.7.7 has no setting to turn off guest IPv6
([issue 1226](https://github.com/superradcompany/microsandbox/issues/1226)).
The guest can disable IPv6 with a sysctl, but that needs root at boot, and
sandboxes run as the unprivileged `agent` user
([ADR 0006](0006-run-sandboxes-as-the-agent-user.md)). microsandbox can hand
PID 1 to an init binary in the image after its own setup, but it refuses to
boot an image that lacks the init it was told to use.

## Decision

The `anvil-base` image disables guest IPv6 in
`/etc/sysctl.d/99-disable-ipv6.conf` and ships a `/sbin/init` script that
applies those settings and then runs `tini` as PID 1, so orphaned processes
are reaped. `anvild` hands PID 1 to `/sbin/init` only for sandboxes that run
the default image; other images boot as before.

## Consequences

- Sandboxes from the default image use IPv4 only, also on hosts where IPv6
  works.
- Custom images keep guest IPv6, and the reset on hosts without IPv6
  egress. They can copy the sysctl file and `/sbin/init` from `anvil-base`,
  but `anvild` doesn't run their init.
- `anvild` and the default image must match: an `anvild` with this change
  can't boot an earlier `anvil-base` image, which has no `/sbin/init`.
  [ADR 0005](0005-default-to-the-anvil-base-image-of-the-same-release.md)
  already pins the default image to the release of `anvild`.
- Sandboxes created before this change keep guest IPv6 until they're
  recreated with `anvil rm` and `anvil start`.
- Remove the workaround once microsandbox can turn off guest IPv6 or
  falls back correctly.
