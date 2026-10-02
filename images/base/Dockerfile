# Base image for anvil sandboxes: an unprivileged agent user with passwordless
# sudo and a Docker engine that the agent can use without sudo.
FROM ubuntu:26.04

# The ubuntu user and group take UID/GID 1000, which belong to agent.
RUN userdel -r ubuntu \
    && if getent group ubuntu >/dev/null; then groupdel ubuntu; fi \
    && groupadd --gid 1000 agent \
    && useradd --uid 1000 --gid agent --create-home --home-dir /home/agent \
        --shell /bin/bash agent

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates docker.io sudo tini \
    && rm -rf /var/lib/apt/lists/* \
    && usermod --append --groups docker agent \
    && echo 'agent ALL=(ALL) NOPASSWD:ALL' > /etc/sudoers.d/agent \
    && chmod 0440 /etc/sudoers.d/agent

COPY --chmod=0755 anvil-entrypoint /usr/local/bin/anvil-entrypoint

# Keep Docker's overlay2 storage off the image's own overlay filesystem.
VOLUME /var/lib/docker

# Numeric, because anvil can't resolve user names without mounting the rootfs.
# Without a GID, so the runtime adds the docker group from /etc/group.
USER 1000
ENV HOME=/home/agent
WORKDIR /home/agent

# tini runs as PID 1, so the container stops on SIGTERM and orphaned processes
# are reaped.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/anvil-entrypoint"]
CMD ["/bin/bash"]
