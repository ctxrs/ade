# syntax=docker/dockerfile:1.6

# ctx-harness: a Linux container image used for ctx tool execution.
# It includes:
# - iptables (for transparent egress guard enforcement)
# - ctx-egress-proxy (transparent proxy that enforces allowlist/llm-only modes)

ARG UBUNTU_VERSION=24.04

# Keep this reasonably current so Cargo can parse crates that opt into the latest stable editions.
FROM --platform=$BUILDPLATFORM rust:1.88-bookworm AS builder

WORKDIR /src

# Build just the proxy binary from the workspace. Buildkit will cache this well when the repo is
# in a git checkout, and multi-arch builds will build per-target under buildx/qemu.
COPY core/Cargo.toml core/Cargo.lock core/
COPY core/crates core/crates
# core/Cargo.toml lists workspace members under core/tools/.
# Include them so Cargo can load the workspace without missing-member errors.
COPY core/tools core/tools
# ctx-http embeds the web session worker bundle at build time via include_str!().
COPY core/packages core/packages

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/core/target \
    cd core && \
    cargo build -p ctx-http --bin ctx-egress-proxy --release && \
    install -Dm0755 /src/core/target/release/ctx-egress-proxy /out/ctx-egress-proxy

FROM ubuntu:${UBUNTU_VERSION}

RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates \
      gawk \
      iptables \
      libc-bin \
      netbase \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /out/ctx-egress-proxy /usr/local/bin/ctx-egress-proxy

CMD ["/bin/sh", "-lc", "while true; do sleep 100000; done"]
