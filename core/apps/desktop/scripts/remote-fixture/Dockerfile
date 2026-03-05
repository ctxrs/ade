FROM ubuntu:24.04

RUN apt-get update \
  && apt-get install -y --no-install-recommends openssh-server ca-certificates git procps \
  && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/ctx-remote-fixture-entrypoint
RUN chmod 755 /usr/local/bin/ctx-remote-fixture-entrypoint \
  && mkdir -p /run/sshd

EXPOSE 22

ENTRYPOINT ["/usr/local/bin/ctx-remote-fixture-entrypoint"]
