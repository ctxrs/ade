# syntax=docker/dockerfile:1.6

# Hermetic Linux release image for desktop packaging lanes.
# This image is intentionally tool-complete so release jobs do not mutate runner state.

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
      libglib2.0-dev \
      librsvg2-dev \
      libsoup-3.0-dev \
      libssl-dev \
      libwebkit2gtk-4.1-dev \
      libgtk-3-bin \
      libgtk-3-dev \
      liblzma-dev \
      libcap-dev \
      musl \
      patchelf \
      pkg-config \
      python3 \
      python3-pip \
      squashfs-tools \
      tar \
      tk \
      unzip \
      xdg-utils \
      xz-utils \
      zsync \
    && rm -rf /var/lib/apt/lists/*

# linuxdeploy AppImage lanes occasionally require legacy SONAME resolution for
# libc++.so.9.0; bake a compatible copy into the image so release jobs remain
# hermetic and avoid mutable runner patching.
RUN bash -lc 'set -euo pipefail; \
    libcxx_src="$(ldconfig -p | awk '"'"'/libc\+\+\.so\.1(\.0)? / {print $NF; exit}'"'"')" && \
    if [ -z "${libcxx_src}" ] || [ ! -f "${libcxx_src}" ]; then \
      echo "error: unable to locate libc++.so.1 while building release image" >&2; \
      exit 1; \
    fi && \
    cp -f "${libcxx_src}" /usr/local/lib/libc++.so.9.0 && \
    patchelf --set-soname libc++.so.9.0 /usr/local/lib/libc++.so.9.0 && \
    ldconfig'

RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get update && \
    apt-get install -y --no-install-recommends nodejs && \
    npm install --global pnpm@9.15.1 && \
    rm -rf /var/lib/apt/lists/*

RUN rustc --version && cargo --version && node --version && pnpm --version && python3 --version

CMD ["bash", "-lc", "sleep infinity"]
