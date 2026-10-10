# 21. Recreate sandboxes from a disk snapshot to change their network rules

## Status

Accepted

## Context

The deny page of a sandbox with network enforcement tells the user to run `fbk
network allow <host>`
([ADR 0018](0018-enforce-egress-with-microsandboxs-network-policy.md)).
microsandbox fixes a sandbox's network policy when it creates the sandbox:
`modify()` changes resources, labels, env and secrets, but not the policy. Until
now, a rule change needed `fbk rm` and a new `fbk start`, which throws away
everything outside the workspace, including installed packages and the Docker
data disk.

microsandbox 0.7.7 can take a disk snapshot of a stopped sandbox. The snapshot
holds the root disk's writable layer and the sandbox-owned disks, such as the
Docker disk. Two APIs create a sandbox from it:

- `RestoreBuilder` restores a snapshot, but has no setters for `init`, labels,
  env, the workdir or secrets. A sandbox with `init: true` would lose its
  `/sbin/init` handoff, and the SSH host name label and the secrets would be
  lost.
- `SandboxBuilder::override_snapshot` takes the root filesystem from a snapshot
  and keeps every other builder setting. It is public but `#[doc(hidden)]`; the
  `msb` CLI uses it.

## Decision

`fbk network` writes the change to `.firebrick.yml` first and then sends the
whole network section to `fbkd` with a new `UpdateNetwork` RPC, also when the
file didn't change, so a sandbox that missed an earlier update or a hand edit
catches up. Every sandbox `fbkd` creates gets a `firebrick.network` label that
records its rules, or `off` when they aren't enforced. When the label already
matches, `fbkd` leaves the sandbox alone and answers `updated: false`. It
refuses a paused sandbox with `FAILED_PRECONDITION`, because the recreated
sandbox can't be paused again. Otherwise, it applies the rules by:

1. Reading the sandbox's settings from its stored config: labels (including the
   stored ports), workspace mount, workdir, extra mounts, vCPUs, memory, Docker
   disk size and `init`.
2. Stopping it when it runs, and taking a disk snapshot (not a full one).
3. Removing it and creating it again with
   `Sandbox::builder(name).override_snapshot(<snapshot path>)`, the same builder
   chain `create_sandbox` uses, the stored secrets and the new rules.
4. Stopping it again when it was stopped, and deleting the snapshot. Its port
   forwards are closed while it's stopped and opened again when it runs.

When the snapshot or the remove fails, `fbkd` starts a sandbox that was running
again, so it keeps its old rules and state. When the recreate fails, `fbkd`
removes what was created, keeps the snapshot and names it in the error and the
log, so the user can recover the sandbox. When only stopping the recreated
sandbox fails, the error says the rules were updated.

microsandbox files the snapshot in a group named after the sandbox, where its
bare name doesn't select it, so `fbkd` refers to the snapshot by its path.

## Consequences

- Files outside the workspace, installed packages and Docker data survive a rule
  change. Running processes don't: the sandbox cold-boots, like after a stop and
  start. Full (memory) snapshots could keep them, but aren't used.
- `override_snapshot` is hidden API and can change in a microsandbox release
  without notice. The `vm-tests` (`update_network_*` in
  `crates/daemon/tests/grpc.rs`) cover it, so an update that breaks it fails CI.
- Between removing and recreating the sandbox, it briefly doesn't exist, so an
  SSH connection to its host name fails in that window. A per-sandbox lock in
  `fbkd` makes start, stop, remove and connect requests for the sandbox wait.
- The CLI rewrites `.firebrick.yml` through serde, which drops its comments and
  formatting.
- Sandboxes created before the `firebrick.network` label existed are recreated
  on their first `fbk network` command, even when their rules don't change.
- The network section is the only create-time setting that can change without
  `fbk rm`.
