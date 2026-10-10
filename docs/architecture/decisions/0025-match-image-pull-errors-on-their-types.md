# 25. Match image pull errors on their types

## Status

Accepted

## Context

When fbkd couldn't pull the image of a new sandbox, the client only saw
`INTERNAL` with `failed to create sandbox`. A typo in the image name, a tag that
doesn't exist, a private image and a network outage all looked the same, and the
developer had to read fbkd's log to find out which one it was.

microsandbox returns the cause as
`MicrosandboxError::Image(ImageError::Registry(OciDistributionError))`.
`ImageError` comes from `microsandbox-image` and `OciDistributionError` from
`oci-client`. `microsandbox` re-exports neither. The only other way to tell the
causes apart is to match on the error message, which fbkd already has to do for
a missing init because microsandbox reports that as text only.

## Decision

The daemon depends on `microsandbox-image` and `oci-client` directly, without
default features, and matches the pull error on its variants:

- `UnauthorizedError` → `NOT_FOUND`: the image doesn't exist, or its registry
  needs a login. Docker Hub and GHCR answer an unknown repository with 401.
- `RegistryError` with `MANIFEST_UNKNOWN`, `NAME_UNKNOWN` or `NOT_FOUND`, or
  `ImageManifestNotFoundError` → `NOT_FOUND`: the image doesn't exist.
- `RequestError` → `UNAVAILABLE`: the registry can't be reached.
- Any other registry error → `INTERNAL`, naming the image.

Every message names the image reference.

## Consequences

- The developer sees which image failed and why, without reading the log.
- Both crates are already in `Cargo.lock` through `microsandbox`, so no new
  third-party code is built. `microsandbox` pins `microsandbox-image` with `=`,
  so its version must stay equal to `microsandbox`'s, like
  `microsandbox-network`. `oci-client` must follow the version
  `microsandbox-image` uses, or the types won't match and the build fails.
- A microsandbox upgrade that wraps the errors differently makes these errors
  fall back to `INTERNAL`. The unit tests build the errors themselves, so only
  the `vm-tests` integration test that starts a sandbox from an unknown tag
  catches that.
