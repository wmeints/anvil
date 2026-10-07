# Runtime view

Every command except `validate` starts by connecting to the daemon over
`$XDG_RUNTIME_DIR/anvild.sock`. When nobody listens on the socket, the CLI
removes a stale socket file, spawns `anvild` and polls the socket for at most
5 seconds. The diagrams below show this step once, in
[Running a session](#running-a-session), and leave it out of the others.

Before `anvild` listens, it exits when the socket already exists, and then
makes sure the microsandbox runtime matches the runtime embedded in its
binary. It extracts the embedded runtime when the runtime is missing or has
another version. When `MSB_PATH` or `paths.msb` selects a runtime with
another version, `anvild` exits instead, and the CLI reports a timeout.

The CLI resolves the sandbox name from `.anvil.yml` in the working directory,
or derives it from the full working directory path when there's no spec file.

## Running a session

`anvil run <command> [args...]` makes sure the sandbox runs, then attaches the
local terminal to a command in the sandbox through the bidirectional `Attach`
stream.

```mermaid
sequenceDiagram
    actor Dev as Developer
    participant CLI as anvil
    participant D as anvild
    participant MS as microsandbox
    participant VM as Sandbox VM

    Dev->>CLI: anvil run COMMAND [ARGS...]
    opt Daemon isn't listening
        CLI->>CLI: Remove stale socket
        CLI->>D: Spawn anvild
        CLI->>CLI: Poll socket (max 5s)
    end
    CLI->>CLI: Resolve spec

    loop Until the sandbox runs (max 120s)
        CLI->>D: GetSandbox(name)
        alt NOT_FOUND
            CLI->>D: StartSandbox(name, image, resources, workspace)
            Note over D,MS: Creates the sandbox, see Starting a sandbox
        else Stopped or Crashed
            CLI->>D: StartSandbox(name, workspace)
            Note over D,MS: Starts the sandbox, see Starting a sandbox
        else Starting
            CLI->>CLI: Wait 250ms and poll again
        else Stopping or Paused
            CLI-->>Dev: Error: try again once it has stopped
        else Running
            Note over CLI: Continue
        end
    end

    CLI->>D: Attach: AttachStart(name, command, args, size)
    D->>MS: Sandbox::get(name).connect()
    D->>MS: exec_stream_with(command, args, tty)
    MS->>VM: Start process with a TTY
    D->>MS: Resize TTY to the window size
    CLI->>CLI: Enable raw mode

    par Input
        loop For each read from stdin
            Dev->>CLI: Keystrokes
            CLI->>D: AttachInput(data)
            D->>VM: Write to stdin
        end
    and Window resizes
        loop For each SIGWINCH
            CLI->>D: AttachResize(width, height)
            D->>VM: Resize TTY
        end
    and Output
        loop Until the process exits
            VM-->>D: stdout / stderr
            D-->>CLI: AttachResponse(output)
            CLI-->>Dev: Write to stdout
        end
    end

    VM-->>D: Exited(code)
    D-->>CLI: AttachResponse(exit_code)
    CLI->>CLI: Restore terminal mode
    CLI-->>Dev: Exit with the process exit code
```

When the CLI disconnects before the process exits, the daemon kills the
process. The sandbox keeps running after the session ends.

## Stopping a sandbox

`anvil stop` stops the sandbox for the working directory. The sandbox and its
disk stay, so it can be started again later.

```mermaid
sequenceDiagram
    actor Dev as Developer
    participant CLI as anvil
    participant D as anvild
    participant MS as microsandbox

    Dev->>CLI: anvil stop
    CLI->>CLI: Resolve spec
    CLI->>D: StopSandbox(name)
    D->>MS: Sandbox::get(name)
    alt Sandbox doesn't exist
        MS-->>D: SandboxNotFound
        D-->>CLI: NOT_FOUND
        CLI-->>Dev: Error
    else Sandbox exists
        MS-->>D: Sandbox handle
        D->>MS: stop()
        MS-->>D: Stopped
        D-->>CLI: StopSandboxResponse
        CLI-->>Dev: Exit 0
    end
```

## Starting a sandbox

`anvil start` creates the sandbox when it doesn't exist yet, or starts the
existing one. The image and resources from the spec only apply when the
sandbox is created.

```mermaid
sequenceDiagram
    actor Dev as Developer
    participant CLI as anvil
    participant D as anvild
    participant MS as microsandbox
    participant SSH as SSH config

    Dev->>CLI: anvil start
    CLI->>CLI: Resolve spec, fill in default image and resources
    CLI->>D: StartSandbox(name, image, resources, workspace)
    D->>MS: Sandbox::get(name)

    alt Sandbox exists
        opt Sandbox has no anvil.hostname label
            D->>MS: List sandboxes for taken host names
            D->>D: Pick unique project.anvil host name
            D->>MS: Set anvil.hostname label
        end
        D->>MS: start_detached()
    else Sandbox doesn't exist
        D->>D: Validate workspace and resources
        alt Invalid
            D-->>CLI: INVALID_ARGUMENT
            CLI-->>Dev: Error
        end
        D->>MS: List sandboxes for taken host names
        D->>D: Pick unique project.anvil host name
        D->>MS: Create detached sandbox (image, cpus, memory, label,<br/>workspace mounted at /workspaces/project)
    end

    D->>MS: List sandboxes
    D->>SSH: Write Host entries for all host names
    D-->>CLI: StartSandboxResponse
    CLI->>D: GetSandbox(name)
    D-->>CLI: GetSandboxResponse(hostname)
    CLI-->>Dev: Connect with: ssh project.anvil
```

`anvil run` and SSH connections start a sandbox the same way when it isn't
running, so `anvil start` is optional.

## Setting a secret

`anvil secret set <name> <value>` stores a secret that sandboxes use without
seeing its value. With `--from-stdin`, the CLI reads the value from stdin
instead.

```mermaid
sequenceDiagram
    actor Dev as Developer
    participant CLI as anvil
    participant D as anvild
    participant F as secrets.yml
    participant MS as microsandbox

    Dev->>CLI: anvil secret set GH_TOKEN --from-stdin
    CLI->>CLI: Read value from stdin
    CLI->>D: SetSecret(name, value, allowed_hosts)
    D->>D: Validate name, value and hosts,<br/>use default hosts when none are given
    alt Invalid
        D-->>CLI: INVALID_ARGUMENT
        CLI-->>Dev: Error
    end
    D->>F: Replace secret (mode 0600)
    D->>MS: List sandboxes
    loop For each sandbox with an anvil.hostname label
        D->>MS: modify().secret(name, value, hosts).next_start()
    end
    D-->>CLI: SetSecretResponse(failed_sandboxes)
    CLI-->>Dev: Secret set, warnings for failed sandboxes
```

When `anvild` creates a sandbox, it adds all secrets from `secrets.yml`.
microsandbox enables TLS interception for the sandbox and sets each secret's
environment variable to a placeholder such as `$MSB_GH_TOKEN`. Its TLS proxy
replaces the placeholder with the real value in HTTP headers of requests to
the allowed hosts, and blocks requests that carry the placeholder to other
hosts. A running sandbox gets a new or changed secret the next time it
starts.

## Connecting via SSH

`ssh <leaf>.anvil`, `scp` and IDEs reach a sandbox through the SSH config the
daemon generates. It sets `anvil ssh-proxy <host>` as `ProxyCommand`, so the
SSH protocol runs over the `SshTunnel` gRPC stream instead of a network port.
The daemon serves each connection with microsandbox's SSH server over an
in-memory pipe.

```mermaid
sequenceDiagram
    actor Dev as Developer
    participant SSHC as OpenSSH client
    participant CLI as anvil ssh-proxy
    participant D as anvild
    participant MS as microsandbox
    participant VM as Sandbox VM

    Dev->>SSHC: ssh project.anvil
    SSHC->>SSHC: Read ~/.ssh/config and the included anvil config
    SSHC->>CLI: Spawn ProxyCommand: anvil ssh-proxy project.anvil
    CLI->>D: SshTunnel: hostname
    D->>MS: List sandboxes with label anvil.hostname=project.anvil
    alt No sandbox has that host name
        D-->>CLI: NOT_FOUND
        CLI-->>SSHC: Exit with error
    end
    D->>MS: connect_or_start_detached()
    MS->>VM: Boot when not running
    D->>MS: Prepare SSH server (host key, authorized client key)
    D->>D: Serve SSH over an in-memory duplex pipe

    par Client to sandbox
        loop Until stdin closes
            SSHC->>CLI: SSH bytes on stdin
            CLI->>D: SshTunnelRequest(data)
            D->>VM: Write to SSH server
        end
    and Sandbox to client
        loop Until the SSH server closes
            VM-->>D: SSH bytes from SSH server
            D-->>CLI: SshTunnelResponse(data)
            CLI-->>SSHC: SSH bytes on stdout
        end
    end

    SSHC->>CLI: Close stdin
    CLI->>D: End request stream
    D->>D: Shut down pipe, wait for SSH server
    D-->>CLI: End response stream
    CLI-->>SSHC: Exit 0
```

The client authenticates with the client key the daemon created, and checks
the sandbox against the host key pinned for `*.anvil` in the `known_hosts`
file. The sandbox keeps running after the connection closes.
