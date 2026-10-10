# Changelog

The notable changes of each firebrick release, newest first. The
`create-release` skill adds a section for each release in its version bump pull
request, and the release workflow copies the section into the GitHub release,
above the generated list of pull requests. Each section starts with a `##
vX.Y.Z - YYYY-MM-DD` heading.

## v0.4.0 - 2026-10-10

This release gives you control over what a sandbox can reach and see: network
allow/deny rules, extra mounts, port forwards and per-sandbox secrets, all
configurable from `.firebrick.yml` or the new `fbk` subcommands. Sandboxes also
come up ready to work, with Docker and the project's mise tools installed.

### Breaking changes

- `fbkd` keeps its microsandbox runtime, images, database and sandboxes in
  `$XDG_STATE_HOME/firebrick/msb` (`~/.local/state/firebrick/msb`) instead of
  `~/.microsandbox`, so a separately installed `msb` can no longer break it.
  Sandboxes created by earlier versions stay in `~/.microsandbox` and no longer
  show up in `fbk ls`; recreate them after upgrading. Images are pulled again on
  first start. A non-empty `MSB_HOME` still takes precedence. (#101)

### Features

- `fbk init` writes a default `.firebrick.yml` for the current directory. (#89)
- Restrict a sandbox's outgoing traffic with a `network` section in
  `.firebrick.yml`: with `enforce: true`, only hosts, domains, IPs and CIDR
  ranges in `allow` are reachable, and `deny` rules win. Blocked HTTP and HTTPS
  requests get a page that names the host. (#92)
- `fbk network allow`, `fbk network deny` and `fbk network policy
  enable|disable` change the rules of an existing sandbox and update
  `.firebrick.yml`. (#97)
- `fbk network disable` removes a sandbox's network device entirely; `fbk
  network enable` restores it. (#103)
- Mount extra host directories into the sandbox with a `mounts` section in
  `.firebrick.yml`, optionally read-only. Host paths may start with `~` or
  `~user`. (#95)
- Forward host ports to servers in the sandbox with a `ports` section in
  `.firebrick.yml`, or live with `fbk port forward <host>:<guest>` and `fbk port
  rm`. (#96, #99)
- Secrets can be scoped to one sandbox with `fbk secret set --scope sandbox`; a
  sandbox-scoped secret overrides the global one with the same name in that
  sandbox only. (#100)
- `xdg-open` and `$BROWSER` inside the sandbox open http and https URLs in the
  host browser, for example for device login flows. Local addresses are refused,
  and enforced network rules apply. (#104)
- `fbk` shows a progress bar while it pulls the sandbox image. (#102)
- The `firebrick-base` image includes Docker, and every sandbox gets a dedicated
  disk for `/var/lib/docker`, sized with `volumes.docker` in `.firebrick.yml`.
  (#83, #84)
- `fbkd` trusts the workspace's mise config and runs `mise install` when a
  sandbox starts, so the project's tools are ready. Set `mise: false` in
  `.firebrick.yml` to turn it off. (#85)

### Fixes

- `fbk start` and `fbk run` refuse to reuse a sandbox that mounts a different
  working directory, instead of silently giving the agent access to another
  project's files when two directories hash to the same name. (#87)
