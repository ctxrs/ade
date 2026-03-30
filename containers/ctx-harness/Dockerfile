# syntax=docker/dockerfile:1.6

# ctx-harness: a Linux container image used for ctx tool execution.
# It includes:
# - iptables (for transparent egress guard enforcement)
# - ctx-egress-proxy (transparent proxy that enforces allowlist/llm-only modes)

ARG UBUNTU_VERSION=24.04

# Build per-target-platform under buildx/qemu so the embedded proxy binary matches the image arch.
FROM --platform=$TARGETPLATFORM rust:1.88-bookworm AS builder

WORKDIR /src

# Build just the proxy binary from the workspace. Buildkit will cache this well when the repo is
# in a git checkout, and multi-arch builds will build per-target under buildx/qemu.
COPY core/Cargo.toml core/Cargo.lock core/
COPY core/crates core/crates
# core/Cargo.toml lists workspace members under core/tools/.
# Include them so Cargo can load the workspace without missing-member errors.
COPY core/tools core/tools

# NOTE: Multi-platform buildx runs platforms concurrently; lock cache mounts to avoid corruption.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/core/target,sharing=locked \
    cd core && \
    cargo build -p ctx-egress-proxy --release -j 2 && \
    install -Dm0755 /src/core/target/release/ctx-egress-proxy /out/ctx-egress-proxy

FROM ubuntu:${UBUNTU_VERSION}

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      gawk \
      git \
      iptables \
      libc-bin \
      netbase \
      sudo \
      tar \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/ctx-egress-proxy /usr/local/bin/ctx-egress-proxy

RUN groupadd --system ctx-user && \
    useradd --system --gid ctx-user --home-dir /home/example-user --create-home --shell /bin/bash ctx-user && \
    printf 'ctx-user ALL=(ALL) NOPASSWD:ALL\n' >/etc/sudoers.d/ctx-user && \
    chmod 0440 /etc/sudoers.d/ctx-user

CMD ["/bin/sh", "-lc", "while true; do sleep 100000; done"]
