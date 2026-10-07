# 3. Render CLI tables with ratatui

## Status

Accepted

## Context

`anvil ls` printed each sandbox as tab-separated name, status and host name.
Tabs don't line up when names differ in length and the output had no header,
so it was hard to read. Scripts that want the list need a stable,
machine-readable format instead.

## Decision

The CLI renders the sandbox list with the `Table` widget of `ratatui`, with
its default features disabled. It doesn't take over the terminal: it renders
the table into an in-memory `Buffer` sized to the content and prints the
buffer as plain text lines. That works the same for a terminal and a pipe.
`Table` draws no lines between columns or below the header, so the CLI
leaves room for them and draws them onto the buffer, joined to the border.

`anvil ls --format json` prints the list as a JSON array of objects with
`name`, `status` (lowercase) and `hostname`, serialized with `serde_json`.

We didn't write the alignment by hand: borders, header and column widths are
what `ratatui` already provides, and the CLI may grow more tables or a
richer terminal UI.

## Consequences

- The table output has borders and a header, so it's meant for people.
  Scripts use `--format json`.
- With its default features off, `ratatui` adds no terminal backend and
  doesn't pull in a second `crossterm` version.
- The CLI depends on `ratatui`, `serde` and `serde_json`.
