# Instructions for working on Anvil

## Purpose of this project

This project builds a sandbox for coding agents. The goal is to keep the agent 
safe inside a microVM-based sandbox. 

## Technology stack

- Use [Nerdbox](https://github.com/containerd/nerdbox) as the basis for the sandbox. 
- Use [Kong](https://github.com/alecthomas/kong) for the CLI implementation.
- Use [Bubbletea](https://github.com/charmbracelet/bubbletea) for the TUI

## Important commands

- `task build` - compiles the sources into the final executable
- `task test` - runs the unit-tests in the project
- `task lint` - verifies the code quality in the source files
- `task format` - formats the source files so the linter passes

## Coding guidelines

Prefer deep modules with narrow interfaces for structuring the code. Each 
module should have tests focusing the public interface.

Before implementing anything make sure you understand the problem. Perform a 
root cause analysis for bugs and ensure you have a thorough spec for new 
features.

Follow the implementation ladder to prevent over-engineering:

1. Does it have to be built. No? Don't do it.
2. Does it already exist in the codebase? Reuse it.
3. Does the standard library do it? Use it.
4. Does a project dependency provide it? Use the dependency.
5. Can this be done with one line? Write the one-liner.
6. Only then, implement the minimum amount of logic required.

## Architecture

Refer to the [Architecture Docs](docs/architecture/README.md) for the key 
architectural decisions and the general design of the project.

## Current state of the project 

We just started the implementation of the first version of the tool. Currently,
you can run a sandbox based on `ubuntu:26.04` on Linux with a local 
`containerd` runtime.


