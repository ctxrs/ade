# syntax=docker/dockerfile:1.6

FROM debian:bookworm-slim

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
      bash \
      build-essential \
      ca-certificates \
      curl \
      file \
      git \
      jq \
      openssh-client \
      pkg-config \
      python3 \
      rsync \
      sudo \
      tar \
      unzip \
      xz-utils \
      zip \
    && rm -rf /var/lib/apt/lists/*

CMD ["bash", "-lc", "sleep infinity"]
