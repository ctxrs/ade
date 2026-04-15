FROM rust:1.88-bookworm

ENV DEBIAN_FRONTEND=noninteractive
ARG DOCKER_CLI_VERSION=29.1.3

COPY scripts/install_desktop_deps_linux_ubuntu.sh /tmp/install_desktop_deps_linux_ubuntu.sh

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        docker.io \
        gettext-base \
        git \
        jq \
        openssh-client \
        python3 \
        python3-pip \
        rsync \
        sudo \
        xz-utils \
    && case "$(dpkg --print-architecture)" in \
         amd64) docker_arch=x86_64 ;; \
         arm64) docker_arch=aarch64 ;; \
         *) echo "unsupported docker cli arch: $(dpkg --print-architecture)" >&2; exit 1 ;; \
       esac \
    && curl -fsSL "https://download.docker.com/linux/static/stable/${docker_arch}/docker-${DOCKER_CLI_VERSION}.tgz" -o /tmp/docker.tgz \
    && tar -xzf /tmp/docker.tgz -C /tmp \
    && install -m 0755 /tmp/docker/docker /usr/local/bin/docker \
    && rm -rf /tmp/docker /tmp/docker.tgz \
    && bash /tmp/install_desktop_deps_linux_ubuntu.sh \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - \
    && apt-get update \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/*

RUN corepack enable \
    && corepack prepare pnpm@9.15.1 --activate \
    && cargo install sccache --locked

RUN rm -f /tmp/install_desktop_deps_linux_ubuntu.sh

RUN useradd --create-home --shell /bin/bash buildbuddy \
    && usermod -aG sudo buildbuddy \
    && printf 'buildbuddy ALL=(ALL) NOPASSWD:ALL\n' > /etc/sudoers.d/buildbuddy \
    && chmod 0440 /etc/sudoers.d/buildbuddy \
    && mkdir -p /workspace \
    && chown -R buildbuddy:buildbuddy /workspace /home/example-user

USER buildbuddy
WORKDIR /workspace
