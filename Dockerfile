FROM ubuntu:26.04

ARG DEBIAN_FRONTEND=noninteractive

# Base tooling, sudo and the mise apt repository.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        gpg \
        sudo \
    && install -dm 755 /etc/apt/keyrings \
    && curl -fsSL https://mise.jdx.dev/gpg-key.pub | gpg --dearmor -o /etc/apt/keyrings/mise-archive-keyring.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/mise-archive-keyring.gpg arch=$(dpkg --print-architecture)] https://mise.jdx.dev/deb stable main" \
        > /etc/apt/sources.list.d/mise.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends mise \
    && rm -rf /var/lib/apt/lists/*

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
