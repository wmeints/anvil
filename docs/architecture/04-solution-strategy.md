# Solution strategy

## Solutions for quality goals

- **Security:** Agents can safely run inside the sandbox without touching 
  data on the host. 

  We solve this by running the agent inside a microVM. The VM has no
  connections to the host other than the volume mounts we expose to the VM. We 
  use secret proxying to ensure we don't expose any real secrets to the 
  sandbox.

- **Security:** Agents cannot access outside website without express permission 
  from the user.

  We'll inject a proxy with TLS interception into the VM so we can look at 
  outgoing traffic. We'll allow users to provide policies to control what
  traffic is allowed and what isn't. We will need to provide a balanced list 
  of allowed domains out of the box to ensure the user isn't frustrated with 
  unnecessary network blockages.

- **Performance**: Sandboxes start within seconds so work can continue quickly.

  We'll use a MicroVM-based solution for the sandboxes as they start very
  quickly and support snapshotting + suspending guests. We can keep sandboxes
  alive and allow users to connect from multiple terminal windows to the same
  VM so the down time is zero if the sandbox is running.

- **Compatibility:** Sandboxes use OCI images to ensure developers can easily 
  build sandboxes on the `anvil-base` image or their own custom images. Each
  image provides an unprivileged `agent` user with UID and GID `1000`.

  We'll use container images as the basis because it's so well-known in the
  community. We'll extend this with a specific configuration format to allow
  the user to specify additional port mappings, volume mappings, and resources
  such as CPU and memory.

- **Reliability:** Sandboxes can be easily restarted and recreated should they 
  fail.

  We'll use a dedicated daemon for managing the sandboxes. Before running a
  command, the CLI checks the sandbox and creates or starts it when needed.
  A broken sandbox can be removed and recreated from its spec.

## Technology choices

- [Microsandbox][MICROSANDBOX]: Sandboxes run as microVMs managed through
  microsandbox. It boots OCI images as lightweight VMs, which gives us the
  flexibility of container images with the isolation of a VM. Its built-in SSH
  server backs the `<name>.anvil` host names.

- [Clap][CLAP]: The `anvil` CLI and the `anvild` command line are built with
  clap's derive API.

- [Tokio][TOKIO]: Both executables use tokio as their async runtime.

- [Tonic][TONIC]: The CLI talks to the daemon over gRPC using tonic. The API is
  defined in `proto/daemon.v1.proto` and compiled at build time with
  `tonic-prost-build`.

- [Serde YAML][SERDE_YAML]: The `.anvil.yml` sandbox spec is parsed with
  serde_yaml.

- [Tracing][TRACING]: Both executables emit logs and diagnostics through
  tracing, so they can be exported as OpenTelemetry data.

## Solution structure

The application has two executables:

- `anvild` - The daemon process managing the lifecycle of the sandboxes. It
  exposes the `SandboxManagementService` gRPC API on a unix socket and
  provisions the SSH keys and config used to reach the sandboxes.

- `anvil` - The CLI for working with sandboxes. It is a client of the daemon,
  starts the daemon when it isn't running, and houses the commands to start,
  stop, list, remove and validate sandboxes, run commands in them and tunnel
  SSH connections to them.

Both executables live in a single Cargo workspace. Each crate has its own
folder under `crates/`:

| Crate          | Folder           | Purpose                                                                  |
| -------------- | ---------------- | ------------------------------------------------------------------------ |
| `anvil-cli`    | `crates/cli`     | The `anvil` executable: commands, daemon client, terminal sessions, SSH proxy. |
| `anvil-daemon` | `crates/daemon`  | The `anvild` executable: gRPC server, sandbox lifecycle, SSH provisioning. |
| `anvil-spec`   | `crates/spec`    | Parses and validates the `.anvil.yml` sandbox spec.                      |
| `anvil-utils`  | `crates/utils`   | Shared helpers, such as the paths of the daemon socket, logs and SSH files. |

The gRPC contract shared by the CLI and the daemon lives in
`proto/daemon.v1.proto`. Each executable generates its own client and server
code from it in its `build.rs`.

[MICROSANDBOX]: https://docs.microsandbox.dev/getting-started/introduction
[CLAP]: https://docs.rs/clap/latest/clap/
[TOKIO]: https://tokio.rs/
[TONIC]: https://github.com/grpc/grpc-rust
[SERDE_YAML]: https://docs.rs/serde_yaml/latest/serde_yaml/
[TRACING]: https://github.com/tokio-rs/tracing
