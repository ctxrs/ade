# syntax=docker/dockerfile:1.6

ARG RUST_IMAGE=rust:1.88-bookworm
FROM ${RUST_IMAGE}

ARG DEBIAN_FRONTEND=noninteractive
ENV PNPM_HOME=/usr/local/share/pnpm
ENV PATH=/usr/local/cargo/bin:${PNPM_HOME}:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      appstream \
      build-essential \
      ca-certificates \
      curl \
      desktop-file-utils \
      file \
      git \
      jq \
      libappindicator3-dev \
      libayatana-appindicator3-dev \
      libc++1 \
      libcap-dev \
      libglib2.0-dev \
      libgtk-3-bin \
      libgtk-3-dev \
      liblzma-dev \
      librsvg2-dev \
      libsoup-3.0-dev \
      libssl-dev \
      libwebkit2gtk-4.1-dev \
      musl \
      openssh-client \
      patchelf \
      pkg-config \
      python3 \
      python3-pip \
      rsync \
      sudo \
      squashfs-tools \
      tar \
      unzip \
      xdg-utils \
      xz-utils \
      zsync \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get update && \
    apt-get install -y --no-install-recommends nodejs && \
    npm install --global pnpm@9.15.1 && \
    rm -rf /var/lib/apt/lists/*

RUN rustc --version && cargo --version && node --version && pnpm --version && python3 --version

CMD ["bash", "-lc", "sleep infinity"]
