# 10. Edit VS Code settings with jsonc-parser

## Status

Accepted

## Context

VS Code's Remote-SSH extension asks for the platform of every host it doesn't
know yet, and opens an empty window that the user has to point at
`/workspaces/<leaf>` by hand. Remote-SSH only skips the question when the host
is in `remote.SSH.remotePlatform` of the user settings. There is no per-host
file or command line flag for it, so connecting in one step means the daemon has
to edit the user's `settings.json` for VS Code and its forks.

That file belongs to the user. It is JSONC: it has comments and trailing commas,
and is often a symlink into a dotfiles repository. Parsing it into a JSON value
and writing it back would drop the comments and reformat the file.

## Decision

The daemon keeps `remote.SSH.remotePlatform` in sync with the sandbox host names
in the user settings of VS Code, VS Code Insiders, Cursor and VSCodium, whenever
it syncs the SSH config. It edits the file with the concrete syntax tree of the
[`jsonc-parser`](https://docs.rs/jsonc-parser) crate (`cst` feature), which
changes only the touched keys and keeps comments, trailing commas and
formatting. It only adds and removes `*.fbk` keys, writes only when the content
changes, and leaves a file it can't parse unchanged.

## Consequences

- Connecting from Remote-SSH takes one step, and `fbk start` can print a `code
  --folder-uri` command that opens the workspace.
- The daemon writes to a file the user owns. A change to the settings format, or
  a bug in the edit, affects the user's editor; the edit is limited to one key
  and never writes a file it couldn't parse.
- One more dependency, which has no further dependencies of its own with the
  `cst` feature.
- The editors are found by their `User` directories under
  `$XDG_CONFIG_HOME`/`~/Library/Application Support`. Other forks, portable
  installs and Windows aren't covered.
