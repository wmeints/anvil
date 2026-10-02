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

## Stopping a sandbox

`anvil stop <sandbox> [--timeout <duration>]` hibernates a sandbox to free the
CPU and memory of its VM. libkrun can't suspend a VM, so only the disk of the
sandbox is kept. Its processes and memory are gone.

1. The CLI rejects a `--timeout` of zero or less. Otherwise it sends a
   `StopSandbox` request with the sandbox name and the timeout (default `10s`)
   to the daemon.
2. The daemon sends `SIGTERM` to all processes in the sandbox task and waits up
   to the timeout for the init process of the sandbox to exit. While it waits,
   it repeats `SIGTERM` for the init process, because a VM that just booted
   may not have set up its signal handler yet.
3. The daemon deletes the task, which kills the processes that are left and
   stops the VM. The container and its writable snapshot are kept.

The init process of a sandbox is [tini](https://github.com/krallin/tini),
which anvil bundles, so it works with any image. Before every boot, anvil
writes tini to `/run/user/<uid>/anvil/init` on the host, because a host reboot
wipes that directory, and mounts the directory read-only at `/.anvil` in the
sandbox. PID 1 runs `/.anvil/tini -- /bin/sh -c 'while :; do sleep infinity &
wait; done'`:

- tini forwards `SIGTERM` to the shell, which isn't PID 1 and exits on it, so
  a sandbox stops right away unless the timeout runs out first.
- tini reaps orphaned processes, so zombies from sessions don't pile up.
- The loop restarts the `sleep` when a process in the sandbox kills it, so
  that doesn't stop the sandbox. Killing tini or the shell does stop it, for
  example with `kill 1`, `pkill sh` or `kill -9 -1`, because the shell isn't
  PID 1 and gets no protection from signals it doesn't handle.

Sandboxes keep the init from the time they were created. Init processes in a
PID namespace only get the signals they handle, so sandboxes created before
`anvil stop` existed run `sleep infinity` as init and always stop after the
timeout. Sandboxes created before tini was bundled run a shell that traps
`SIGTERM` as init.

Sessions attached to the sandbox end when it stops. Their CLI restores the
terminal and exits with the non-zero exit code of the session process.
Stopping a sandbox whose VM already stopped succeeds without output. The next
`anvil start` or `anvil run` boots the VM again, as described in
[Starting a sandbox](#starting-a-sandbox).

## Starting a sandbox

`anvil start <sandbox>` boots the VM of a stopped sandbox without opening a
session, for example to warm it up ahead of time. It's the counterpart of
`anvil stop`. libkrun can't suspend a VM, so the VM cold-boots on the kept
disk: files written before the stop are there, processes aren't.

1. The CLI sends a `StartSandbox` request with the sandbox name to the daemon.
2. The daemon loads the container of the sandbox and gets its task running:
   - a sandbox without a task gets a new task,
   - a stopped task is deleted and replaced by a new one,
   - a created task is started and a paused task is resumed.
3. The daemon answers once the task reports it started. The CLI exits with
   code 0 and prints nothing.

Starting a sandbox that's already running leaves its task as it is and
succeeds without output. An unknown sandbox fails with `sandbox not found`.
When containerd fails to boot the VM, the error starts with
`could not start sandbox "<name>"` and includes the cause.

`anvil run` takes the same steps before it opens a session, so a stopped
sandbox also boots on the next `anvil run`.
