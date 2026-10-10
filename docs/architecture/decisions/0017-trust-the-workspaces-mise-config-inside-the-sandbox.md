# 17. Trust the workspace's mise config inside the sandbox

## Status

Accepted

## Context

`fbkd` installs the workspace's mise tools when it creates or starts a sandbox
(see [Starting a sandbox](../06-runtime-view.md#starting-a-sandbox)). mise
refuses to load a config file that hasn't been trusted, so `fbkd` has to run
`mise trust` on the workspace's config files first.

Trusting a mise config lets it run code: env templates, hooks and plugins from
the project run without the developer confirming them. On a developer's machine
that is a decision mise leaves to the developer, because the code runs with the
developer's rights.

Firebrick splits the system into two sides. The inside of the sandbox is the
dirty side: the coding agent runs there with passwordless `sudo` and executes
whatever code the project and its dependencies bring along. The host outside the
sandbox is the clean side, and the microVM is the boundary between the two. The
boundary has two deliberate openings: the workspace is mounted read/write, so
changes to the project's files show up on the host, and the sandbox has network
access.

## Decision

`fbkd` trusts the mise config files at the root of the workspace without asking
the developer. It trusts each file by path, never with `mise trust --all`, and
runs `mise trust` and `mise install` inside the sandbox as the image's default
user.

mise runs only on the dirty side, where the agent can already run any code. Code
from a trusted config can reach exactly what the agent can reach: the guest, the
workspace's files and the network. It can't reach the rest of the host. Trusting
the config therefore gives that code nothing the agent doesn't already have, and
doesn't change which side is dirty and which is clean. Firebrick doesn't run
mise on the host.

## Consequences

- Projects that pin their tools with mise get them installed without manual
  steps, and the sandbox comes up ready to build.
- Code from the workspace's mise config runs automatically when a sandbox is
  created or started. Like the agent, it can change the project's files on the
  host through the workspace mount and send data out over the network. Review
  changes to the workspace before running them on the host, as you would for the
  agent's own changes.
- The microVM boundary has to hold for this to be safe. That was already
  required to contain the agent, so this adds no new requirement on the sandbox.
- Developers who don't want this set `mise: false` in `.firebrick.yml`.
- `fbkd` must never run `mise trust` or other workspace-controlled commands on
  the host, because that would move dirty-side code to the clean side.
