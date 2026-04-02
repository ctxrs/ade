FROM ubuntu:24.04

ARG NERDCTL_VERSION=v2.2.1

RUN apt-get update \
  && apt-get install -y --no-install-recommends \
    openssh-server \
    ca-certificates \
    curl \
    git \
    iproute2 \
    iptables \
    procps \
    sudo \
    containerd \
    containernetworking-plugins \
  && arch="$(dpkg --print-architecture)" \
  && case "${arch}" in \
    amd64) nerdctl_arch="amd64" ;; \
    arm64) nerdctl_arch="arm64" ;; \
    *) echo "unsupported fixture architecture: ${arch}" >&2; exit 1 ;; \
  esac \
  && curl -fsSL "https://github.com/containerd/nerdctl/releases/download/${NERDCTL_VERSION}/nerdctl-${NERDCTL_VERSION#v}-linux-${nerdctl_arch}.tar.gz" -o /tmp/nerdctl.tgz \
  && tar -xzf /tmp/nerdctl.tgz -C /usr/local/bin nerdctl \
  && mv /usr/local/bin/nerdctl /usr/local/bin/nerdctl-real \
  && rm -f /tmp/nerdctl.tgz \
  && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/ctx-remote-fixture-entrypoint
RUN chmod 755 /usr/local/bin/ctx-remote-fixture-entrypoint \
  && mkdir -p /run/sshd

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/ctx-remote-fixture-entrypoint"]
