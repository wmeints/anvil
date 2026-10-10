# 11. Own the firebrick entries in Zed's ssh_connections

## Status

Accepted

## Context

Zed's remote development connects with the system `ssh`, so the `<leaf>.fbk`
hosts in the generated SSH config already work. To open a sandbox, though, the
user has to add the host in Zed's Remote Projects dialog and type
`/workspaces/<leaf>` by hand. Zed reads its remote projects only from the
`ssh_connections` array in the user's `settings.json`; there is no per-host file
for them.

[ADR 0010](0010-edit-vs-code-settings-with-jsonc-parser.md) lets the daemon edit
the VS Code-family settings, limited to one key per host. Zed's entries are
objects with a host, a nickname and a list of projects, so keeping them in sync
means editing whole array entries.

## Decision

The daemon keeps one `ssh_connections` entry per sandbox in Zed's user
`settings.json`, whenever it syncs the SSH config. It owns every entry whose
`host` ends in `.fbk`: it adds missing ones, rewrites the ones whose value
differs from `host`, `nickname` and the workspace project, and removes stale and
duplicate ones. It uses the same `jsonc-parser` CST editing as ADR 0010, shared
in the `settings_file` module, so other entries, keys, comments and trailing
commas stay as they are. It skips Zed when the `zed` config directory doesn't
exist, and leaves a file unchanged when it can't parse it or `ssh_connections`
isn't an array of objects.

## Consequences

- A sandbox shows up in Zed's Remote Projects with its workspace, and `fbk
  start` can print a `zed ssh://<host>/workspaces/<leaf>` command that opens it.
- The daemon writes whole entries in a file the user owns, not just one key.
  Changes the user makes to an `*.fbk` entry, such as extra project paths, are
  overwritten on the next sync.
- No new dependency.
- Zed is found under `$XDG_CONFIG_HOME` (default `~/.config`) on Linux and under
  `~/.config` on macOS. Flatpak installs, Zed Preview and Windows aren't
  covered.
