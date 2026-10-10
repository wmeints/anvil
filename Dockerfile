# syntax=docker/dockerfile:1
FROM ubuntu:26.04

ARG DEBIAN_FRONTEND=noninteractive

# Base tooling, a native build toolchain, sudo, procps (sysctl), tini and the mise apt repository.
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
    && apt-get update \
    && apt-get install -y --no-install-recommends mise \
    && rm -rf /var/lib/apt/lists/*

# Microsandbox gives the guest IPv6 even when the host can't route it, and then resets IPv6
# connections after the handshake, so clients never fall back to IPv4. Disable guest IPv6
# until microsandbox handles this: https://github.com/superradcompany/microsandbox/issues/1226
COPY <<EOF /etc/sysctl.d/99-disable-ipv6.conf
net.ipv6.conf.all.disable_ipv6=1
net.ipv6.conf.default.disable_ipv6=1
EOF

# anvild hands PID 1 to /sbin/init in sandboxes that run this image. The script applies the
# sysctl settings and hands PID 1 to tini, which reaps zombie processes.
COPY --chmod=755 <<EOF /sbin/init
#!/bin/sh
sysctl -q -p /etc/sysctl.d/99-disable-ipv6.conf
exec /usr/bin/tini -- sleep infinity
EOF

# SSH clients send their own TERM, which the guest may have no terminfo entry for, such as
# `xterm-ghostty`. Interactive shells fall back to xterm-256color for those.
RUN echo 'infocmp "$TERM" >/dev/null 2>&1 || export TERM=xterm-256color' >> /etc/bash.bashrc

# Replace the default ubuntu user (uid/gid 1000) with the agent user.
RUN userdel --remove ubuntu \
    && groupadd --gid 1000 agent \
    && useradd --uid 1000 --gid 1000 --create-home --shell /bin/bash agent \
    && echo "agent ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/agent \
    && chmod 0440 /etc/sudoers.d/agent

USER agent
WORKDIR /home/agent

# Activate mise for interactive shells and expose its shims to everything else.
RUN echo 'eval "$(mise activate bash)"' >> /home/agent/.bashrc
ENV PATH="/home/agent/.local/share/mise/shims:${PATH}"

CMD ["/bin/bash"]
