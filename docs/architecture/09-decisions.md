# Architecture decisions

The decision records live in [`decisions/`](./decisions/).

1. [Use Ubuntu's docker.io package in the base image](./decisions/0001-docker-io-for-the-base-image.md)
   - `docker.io` over Docker CE, for a simpler, multi-arch build that Ubuntu
     patches.
2. [Start dockerd from an entrypoint script](./decisions/0002-start-dockerd-from-an-entrypoint.md)
   - a small entrypoint over an init system, so the image runs any command.
3. [Bundle tini as the sandbox init](./decisions/0003-bundle-tini-as-the-sandbox-init.md)
   - a bundled, mounted tini over a shell trap or tini in the image, so every
     image stops on `SIGTERM` and reaps orphaned processes.
