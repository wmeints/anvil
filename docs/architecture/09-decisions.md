# Architecture decisions

The decision records live in [`decisions/`](./decisions/).

1. [Use Ubuntu's docker.io package in the base image](./decisions/0001-docker-io-for-the-base-image.md)
   - `docker.io` over Docker CE, for a simpler, multi-arch build that Ubuntu
     patches.
2. [Start dockerd from an entrypoint script](./decisions/0002-start-dockerd-from-an-entrypoint.md)
   - a small entrypoint over an init system, so the image runs any command.
3. [Run tini as the init of the base image](./decisions/0003-tini-in-the-base-image.md)
   - tini in the image's `ENTRYPOINT` over a shell init or a tini that anvil
     bundles and mounts, so anvil ships no third-party binaries.
