# Crosscutting concepts

## How are sandboxes executed

When running a sandbox, we're talking about an actual OCI container running in 
a VM. We use [containerd](https://containerd.io/) to run containers. So in a 
way, this is just another container management tool.

All this doesn't sound terribly isolated, and you're right. But before we talk 
about isolation there's one more concept to understand. The containerd runtime
runs containers using shims. Whenever you restart containerd, the containers 
themselves remain running. That's because they run behind a shim that has a 
one-on-one relationship with the actual container process. 

It's the shim that we need to work with to provide proper isolation. We use 
[nerdbox](https://github.com/containerd/nerdbox) as a shim so that the container
isn't a cgroup, but a virtual machine.

You won't see a lot of interaction in this application with nerdbox other than
us specifying that a new container must run with the nerdbox shim. The rest of 
the logic in this application only deals with containerd.

As long as you run containerd rootless, you're good to go!