# Building block view

## Level 1 - Building blocks

```mermaid
C4Container
    Person(user, "Developer")
    Container(anvil, "anvil", "Go, Kong", "CLI")
    Container(anvild, "anvild", "Go, gRPC", "User daemon")
    System_Ext(containerd, "containerd", "Rootless runtime with Nerdbox")

    Rel(user, anvil, "Runs commands")
    Rel(anvil, anvild, "gRPC", "Unix socket")
    Rel(anvild, containerd, "Manages sandboxes")
```

- `anvil` - CLI that sends commands to the daemon and attaches the terminal to
  sandbox sessions.
- `anvild` - Daemon that manages the lifecycle of the sandboxes and sessions.

The CLI and daemon speak gRPC (`api/v1alpha1`) over a unix socket in
`$XDG_RUNTIME_DIR/anvil`. The socket directory is private to the user (`0700`).

## Level 2 - CLI

```mermaid
C4Component
    Container_Boundary(cli, "anvil") {
        Component(cmd, "cmd/anvil", "Kong", "Commands")
        Component(control, "internal/control", "gRPC client", "Daemon client")
        Component(api, "api/v1alpha1", "Protobuf", "Control API")
        Component(paths, "internal/paths", "Go", "File locations")
    }
    Container_Ext(anvild, "anvild")

    Rel(cmd, control, "Uses")
    Rel(control, api, "Uses")
    Rel(control, paths, "Finds socket")
    Rel(control, anvild, "gRPC")
```

- `cmd/anvil` - Parses the `create`, `ls`, `rm` and `run` commands and puts the
  terminal in raw mode for sessions.
- `internal/control` - Client for the daemon that wraps the gRPC calls and
  streams session I/O and terminal resizes.
- `api/v1alpha1` - Protobuf definition and generated code of the control API.
- `internal/paths` - Well-known paths, such as the daemon socket.

## Level 2 - Daemon

```mermaid
C4Component
    Container_Boundary(daemon, "anvild") {
        Component(cmd, "cmd/anvild", "Go", "Entrypoint")
        Component(server, "internal/daemon", "gRPC server", "Control API server")
        Component(sandbox, "internal/sandbox", "containerd client", "Sandbox runtime")
        Component(api, "api/v1alpha1", "Protobuf", "Control API")
        Component(paths, "internal/paths", "Go", "File locations")
    }
    System_Ext(containerd, "containerd")

    Rel(cmd, server, "Runs")
    Rel(server, api, "Implements")
    Rel(server, sandbox, "Uses")
    Rel(server, paths, "Finds sockets")
    Rel(sandbox, paths, "Finds FIFOs")
    Rel(sandbox, containerd, "Manages containers")
```

- `cmd/anvild` - Starts the daemon.
- `internal/daemon` - Listens on the socket, connects to containerd and maps
  the control API onto sandbox operations.
- `internal/sandbox` - Creates, starts, stops and removes sandbox VMs and runs
  terminal sessions in them.
- `api/v1alpha1` - Protobuf definition and generated code of the control API.
- `internal/paths` - Well-known paths, such as the daemon socket, the
  containerd socket and the FIFO directory.
