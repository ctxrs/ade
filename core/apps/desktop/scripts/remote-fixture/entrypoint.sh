#!/bin/sh
set -eu

fixture_user="${CTX_FIXTURE_USER:-ctxfixture}"
fixture_home="${CTX_FIXTURE_HOME:-/home/${fixture_user}}"
authorized_keys_path="${CTX_FIXTURE_AUTHORIZED_KEYS:-}"
authorized_keys_b64="${CTX_FIXTURE_AUTHORIZED_KEYS_B64:-}"
auth_mode="${CTX_FIXTURE_AUTH_MODE:-key}"
password="${CTX_FIXTURE_PASSWORD:-}"

if ! getent passwd "${fixture_user}" >/dev/null 2>&1; then
  useradd -m -d "${fixture_home}" -s /bin/sh "${fixture_user}"
fi

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

exec /usr/sbin/sshd -D -e \
  ${sshd_auth_flags} \
  -o PermitRootLogin=no \
  -o UsePAM=no
