#!/bin/sh
set -eu

fixture_user="${CTX_FIXTURE_USER:-ctxfixture}"
fixture_home="${CTX_FIXTURE_HOME:-/home/${fixture_user}}"
authorized_keys_path="${CTX_FIXTURE_AUTHORIZED_KEYS:-}"
authorized_keys_b64="${CTX_FIXTURE_AUTHORIZED_KEYS_B64:-}"
auth_mode="${CTX_FIXTURE_AUTH_MODE:-key}"
password="${CTX_FIXTURE_PASSWORD:-}"
containerd_config_path="/etc/containerd/config.toml"
cni_config_path="/etc/cni/net.d/10-nerdctl.conflist"
nerdctl_real="/usr/local/bin/nerdctl-real"
nerdctl_wrapper="/usr/local/bin/nerdctl"
containerd_log="/var/log/ctx-fixture-containerd.log"
containerd_sock="/run/containerd/containerd.sock"

if ! getent passwd "${fixture_user}" >/dev/null 2>&1; then
  useradd -m -d "${fixture_home}" -s /bin/sh "${fixture_user}"
fi

mkdir -p /etc/sudoers.d /etc/cni/net.d /etc/containerd /run/containerd /var/lib/containerd /var/log
cat > "${containerd_config_path}" <<EOF
version = 2
root = "/var/lib/containerd"
state = "/run/containerd"
[grpc]
  address = "${containerd_sock}"
[plugins."io.containerd.grpc.v1.cri".containerd]
  snapshotter = "native"
EOF

cat > "${cni_config_path}" <<'EOF'
{
  "cniVersion": "1.0.0",
  "name": "bridge",
  "plugins": [
    {
      "type": "bridge",
      "bridge": "nerdctl0",
      "isGateway": true,
      "ipMasq": true,
      "promiscMode": true,
      "ipam": {
        "type": "host-local",
        "ranges": [[{ "subnet": "10.88.0.0/16" }]],
        "routes": [{ "dst": "0.0.0.0/0" }]
      }
    },
    { "type": "portmap", "capabilities": { "portMappings": true } },
    { "type": "firewall" },
    { "type": "tuning" }
  ]
}
EOF

cat > "${nerdctl_wrapper}" <<EOF
#!/usr/bin/env bash
set -euo pipefail
filtered_args=()
explicit_snapshotter=0
while [[ \$# -gt 0 ]]; do
  case "\$1" in
    --snapshotter=*)
      explicit_snapshotter=1
      filtered_args+=("\$1")
      shift
      continue
      ;;
    --snapshotter)
      explicit_snapshotter=1
      filtered_args+=("\$1")
      shift
      if [[ \$# -gt 0 ]]; then
        filtered_args+=("\$1")
        shift
      fi
      continue
      ;;
    --userns=keep-id)
      shift
      continue
      ;;
    --network=slirp4netns:allow_host_loopback=true|--net=slirp4netns:allow_host_loopback=true)
      shift
      continue
      ;;
    --network|--net)
      if [[ "\${2:-}" == "slirp4netns:allow_host_loopback=true" ]]; then
        shift 2
        continue
      fi
      filtered_args+=("\$1")
      shift
      if [[ \$# -gt 0 ]]; then
        filtered_args+=("\$1")
        shift
      fi
      continue
      ;;
    *)
      filtered_args+=("\$1")
      shift
      ;;
  esac
done
if [[ "\${explicit_snapshotter}" -eq 0 ]]; then
  filtered_args=(--snapshotter native "\${filtered_args[@]}")
fi
# Nested containerd inside the Docker SSH fixture must use the native snapshotter.
# The default overlayfs snapshotter fails in this environment with:
# `failed to create shim task: failed to mount rootfs component: invalid argument`.
# Keep the fix local to the fixture wrapper so product code still exercises the
# normal remote sandbox path, but against a substrate that is actually viable in CI.
exec sudo --non-interactive "${nerdctl_real}" --address "${containerd_sock}" --namespace default --snapshotter native "\${filtered_args[@]}"
EOF
chmod 755 "${nerdctl_wrapper}"

cat > "/etc/sudoers.d/ctx-fixture-nerdctl" <<EOF
Defaults:${fixture_user} !requiretty
${fixture_user} ALL=(root) NOPASSWD: ${nerdctl_real}
EOF
chmod 440 "/etc/sudoers.d/ctx-fixture-nerdctl"

sshd_auth_flags=""
case "${auth_mode}" in
  key)
    # Debian images can leave newly created accounts locked, which blocks SSH even
    # for pubkey auth. Unlock for fixture-only key-based login.
    usermod -U "${fixture_user}" >/dev/null 2>&1 || true
    passwd -d "${fixture_user}" >/dev/null 2>&1 || true

    mkdir -p /run/sshd "${fixture_home}/.ssh"
    if [ -n "${authorized_keys_b64}" ]; then
      printf '%s' "${authorized_keys_b64}" | base64 -d > "${fixture_home}/.ssh/authorized_keys"
    elif [ -n "${authorized_keys_path}" ] && [ -f "${authorized_keys_path}" ]; then
      cp "${authorized_keys_path}" "${fixture_home}/.ssh/authorized_keys"
    else
      echo "missing fixture authorized_keys input for key auth mode" >&2
      exit 1
    fi
    chmod 700 "${fixture_home}/.ssh"
    chmod 600 "${fixture_home}/.ssh/authorized_keys"
    chown -R "${fixture_user}:${fixture_user}" "${fixture_home}/.ssh"
    sshd_auth_flags="-o PasswordAuthentication=no -o KbdInteractiveAuthentication=no -o ChallengeResponseAuthentication=no -o PubkeyAuthentication=yes"
    ;;
  password)
    if [ -z "${password}" ]; then
      echo "missing CTX_FIXTURE_PASSWORD for password auth mode" >&2
      exit 1
    fi
    usermod -U "${fixture_user}" >/dev/null 2>&1 || true
    printf '%s:%s\n' "${fixture_user}" "${password}" | chpasswd
    mkdir -p /run/sshd "${fixture_home}/.ssh"
    if [ -n "${authorized_keys_b64}" ]; then
      printf '%s' "${authorized_keys_b64}" | base64 -d > "${fixture_home}/.ssh/authorized_keys"
      chmod 600 "${fixture_home}/.ssh/authorized_keys"
    elif [ -n "${authorized_keys_path}" ] && [ -f "${authorized_keys_path}" ]; then
      cp "${authorized_keys_path}" "${fixture_home}/.ssh/authorized_keys"
      chmod 600 "${fixture_home}/.ssh/authorized_keys"
    else
      rm -f "${fixture_home}/.ssh/authorized_keys"
    fi
    chmod 700 "${fixture_home}/.ssh"
    chown -R "${fixture_user}:${fixture_user}" "${fixture_home}/.ssh"
    sshd_auth_flags="-o PasswordAuthentication=yes -o KbdInteractiveAuthentication=yes -o ChallengeResponseAuthentication=yes -o PubkeyAuthentication=yes"
    ;;
  *)
    echo "unsupported CTX_FIXTURE_AUTH_MODE: ${auth_mode}" >&2
    exit 1
    ;;
esac

ssh-keygen -A >/dev/null 2>&1 || true

containerd --config "${containerd_config_path}" >"${containerd_log}" 2>&1 &
attempt=0
until CONTAINERD_ADDRESS="${containerd_sock}" CONTAINERD_NAMESPACE="default" "${nerdctl_real}" info >/dev/null 2>&1
do
  attempt=$((attempt + 1))
  if [ "${attempt}" -ge 30 ]; then
    echo "containerd failed to become ready in fixture" >&2
    if [ -f "${containerd_log}" ]; then
      tail -n 200 "${containerd_log}" >&2 || true
    fi
    exit 1
  fi
  sleep 1
done

exec /usr/sbin/sshd -D -e \
  ${sshd_auth_flags} \
  -o PermitRootLogin=no \
  -o UsePAM=no
