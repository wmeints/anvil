# 20. Look up user home directories with nix

## Status

Accepted

## Context

The `host` of an entry in the `mounts` section of `.firebrick.yml` can start
with `~`. Users expect shell semantics: `~` and `~/path` mean `$HOME`, and
`~user/path` means the home directory of `user`. The standard library has no way
to look up another user's home directory.

Parsing `/etc/passwd` by hand misses users that come from NSS sources such as
LDAP or SSSD on Linux, and the regular users on macOS, which live in Directory
Services instead of `/etc/passwd`. `getpwnam_r(3)` covers both, but calling it
through `libc` needs `unsafe` code and buffer management.

## Decision

The CLI depends on `nix` with only the `user` feature and looks up `~user` with
`nix::unistd::User::from_name`, which wraps `getpwnam_r`. `nix` 0.31 is already
in `Cargo.lock` through microsandbox, so this adds no new third-party code to
the build. `~` and `~/path` still expand from `$HOME`.

## Consequences

- `~user` resolves the same way as in a shell, on Linux and macOS.
- An unknown user fails with `can't expand <host>: user <user> doesn't exist`
  before the sandbox is created.
- The CLI's `nix` version should follow the one microsandbox uses, so the build
  keeps a single copy.
