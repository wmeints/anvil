# Runtime view


## Running a session

`anvil run <sandbox> <command> [args...]` starts an interactive session in a
sandbox. When the VM of the sandbox isn't running, for example after a host
reboot, the daemon boots it first. The files in the sandbox are kept, but
processes from before the VM stopped are gone.

1. The CLI switches the local terminal to raw mode and opens a bidirectional
   `AttachSandbox` stream to the daemon. The first message is an `AttachStart`
   with the sandbox name, command and the terminal size.
2. The daemon execs the command as a new process in the sandbox task through
   containerd, with a TTY attached. The process inherits the environment,
   working directory and user of the sandbox. `TERM` is always set to
   `xterm-256color`, because the host's `TERM` may have no terminfo entry in
   the sandbox image.
3. Keystrokes flow from the CLI to the process as `SessionInput` messages and
   terminal resizes (`SIGWINCH`) as `WindowSize` messages. Terminal output flows
   back as `stdout` messages.
4. When the process exits, the daemon sends the exit code and closes the
   stream. The CLI restores the terminal and exits with the same code.

When the CLI disconnects early, the daemon kills the session process. The
sandbox itself keeps running.
