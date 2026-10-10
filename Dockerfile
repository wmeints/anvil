# syntax=docker/dockerfile:1
FROM ubuntu:26.04

ARG DEBIAN_FRONTEND=noninteractive

# Base tooling, a native build toolchain, sudo, procps (sysctl), tini, the mise apt repository
# and the Docker engine from Docker's apt repository.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        git \
        gpg \
        libssl-dev \
        pkg-config \
        procps \
        sudo \
        tini \
    && install -dm 755 /etc/apt/keyrings \
    && curl -fsSL https://mise.jdx.dev/gpg-key.pub | gpg --dearmor -o /etc/apt/keyrings/mise-archive-keyring.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/mise-archive-keyring.gpg arch=$(dpkg --print-architecture)] https://mise.jdx.dev/deb stable main" \
        > /etc/apt/sources.list.d/mise.list \
    && curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc \
    && chmod a+r /etc/apt/keyrings/docker.asc \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
        > /etc/apt/sources.list.d/docker.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        containerd.io \
        docker-buildx-plugin \
        docker-ce \
        docker-ce-cli \
        docker-compose-plugin \
        mise \
    && rm -rf /var/lib/apt/lists/*

# Microsandbox gives the guest IPv6 even when the host can't route it, and then resets IPv6
# connections after the handshake, so clients never fall back to IPv4. Disable guest IPv6
# until microsandbox handles this: https://github.com/superradcompany/microsandbox/issues/1226
COPY <<EOF /etc/sysctl.d/99-disable-ipv6.conf
net.ipv6.conf.all.disable_ipv6=1
net.ipv6.conf.default.disable_ipv6=1
EOF

# fbkd hands PID 1 to /sbin/init in sandboxes that run this image. The script applies the
# sysctl settings, starts dockerd in the background without waiting for it, and hands PID 1
# to tini, which reaps zombie processes. dockerd keeps its data on the ext4 disk that fbkd
# attaches at /var/lib/docker, and logs to /var/log/dockerd.log. /run lives on the sandbox's
# persistent root instead of a tmpfs, so the init removes the runtime state of the previous
# boot first; otherwise a stale docker.pid stops dockerd from starting after fbk stop/start.
COPY --chmod=755 <<EOF /sbin/init
#!/bin/sh
sysctl -q -p /etc/sysctl.d/99-disable-ipv6.conf
rm -rf /run/docker.pid /run/docker /run/containerd
/usr/bin/dockerd >/var/log/dockerd.log 2>&1 &
exec /usr/bin/tini -- sleep infinity
EOF

# SSH clients send their own TERM, which the guest may have no terminfo entry for, such as
# `xterm-ghostty`. Interactive shells fall back to xterm-256color for those.
RUN echo 'infocmp "$TERM" >/dev/null 2>&1 || export TERM=xterm-256color' >> /etc/bash.bashrc

# Replace the default ubuntu user (uid/gid 1000) with the agent user, who can use docker
# without sudo through the docker group.
RUN userdel --remove ubuntu \
    && groupadd --gid 1000 agent \
    && useradd --uid 1000 --gid 1000 --groups docker --create-home --shell /bin/bash agent \
    && echo "agent ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/agent \
    && chmod 0440 /etc/sudoers.d/agent

USER agent
WORKDIR /home/agent

# Activate mise for interactive shells and expose its shims to everything else.
RUN echo 'eval "$(mise activate bash)"' >> /home/agent/.bashrc
ENV PATH="/home/agent/.local/share/mise/shims:${PATH}"

CMD ["/bin/bash"]
