# Solution strategy

## Solutions for quality goals

- **Security:** Agents can safely run inside the sandbox without touching 
  data on the host. 

  We solve this by running the agent inside a container-based VM. The VM has no
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
  build sandboxes using standard images like `ubuntu:26.04` or their own custom
  images.

  We'll use container images as the basis because it's so well-known in the
  community. We'll extend this with a specific configuration format to allow
  the user to specify additional port mappings, volume mappings, and resources
  such as CPU and memory.

- **Reliability:** Sandboxes can be easily restarted and recreated should they 
  fail.

  We'll use a dedicated daemon for managing the sandboxes. The daemon uses 
  desired state configuration with a reconsiliation loop to ensure sandboxes 
  are usable for the user.

## Technology choices

- [Nerdbox][NERDBOX]: We're using nerdbox to run containers as a virtual 
  machine. This tool is excellent for our use case because it provides a nice
  balance between the flexiblity of container images and the safety of a VM.

- [gVisor][GVISOR]: Networking for the product runs via gVisor to control where
  the networking connections go. We combine this tool with a custom-built proxy
  in the daemon to ensure we can control egress traffic.

- [virtio][VIRTIO]: Storage mounts run through virtio, as this combines well 
  with the nerdbox tooling.

- [bubbletea][BUBBLETEA] and [charm][CHARM]: Any dashboards or user 
  interactions in the terminal happen through these two libraries as they make 
  it a lot easier to build a decent terminal UI.

## Solution structure

The application will have two executables:

- `anvild` - The daemon process managing the lifecycle of the sandboxes. This
  executable exposes a gRPC protocol to communicate with the sandboxes.

- `anvil` - The CLI for working with sandboxes. This application is a client of
  the daemon and houses the key commands to work with sandboxes.

We'll use a single repository to host both executables. The executables will 
have their own folder in `cmd/anvild` and `cmd/anvil` as the entrypoint. The 
other logic is shared between the two in the `internal` directory.

To maximize interoperability we'll expose the `grpc` API description through 
the `api` directory so people can generate their own client to talk to the 
daemon process.

[NERDBOX]: https://github.com/containerd/nerdbox
[GVISOR]: https://gvisor.dev/
[VIRTIO]: https://docs.kernel.org/driver-api/virtio/virtio.html
[BUBBLETEA]: https://github.com/charmbracelet/bubbletea
