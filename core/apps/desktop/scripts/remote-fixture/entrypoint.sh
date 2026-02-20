#!/bin/sh
set -eu

fixture_user="${CTX_FIXTURE_USER:-ctxfixture}"
fixture_home="${CTX_FIXTURE_HOME:-/home/${fixture_user}}"
authorized_keys_path="${CTX_FIXTURE_AUTHORIZED_KEYS:-}"

if [ -z "${authorized_keys_path}" ] || [ ! -f "${authorized_keys_path}" ]; then
  echo "missing CTX_FIXTURE_AUTHORIZED_KEYS file: ${authorized_keys_path}" >&2
  exit 1
fi

if ! getent passwd "${fixture_user}" >/dev/null 2>&1; then
  useradd -m -d "${fixture_home}" -s /bin/sh "${fixture_user}"
fi

# Debian images can leave newly created accounts locked, which blocks SSH even
# for pubkey auth. Unlock for fixture-only key-based login.
usermod -U "${fixture_user}" >/dev/null 2>&1 || true
passwd -d "${fixture_user}" >/dev/null 2>&1 || true

mkdir -p /run/sshd "${fixture_home}/.ssh"
cp "${authorized_keys_path}" "${fixture_home}/.ssh/authorized_keys"
chmod 700 "${fixture_home}/.ssh"
chmod 600 "${fixture_home}/.ssh/authorized_keys"
chown -R "${fixture_user}:${fixture_user}" "${fixture_home}/.ssh"

ssh-keygen -A >/dev/null 2>&1 || true

exec /usr/sbin/sshd -D -e \
  -o PasswordAuthentication=no \
  -o KbdInteractiveAuthentication=no \
  -o ChallengeResponseAuthentication=no \
  -o PubkeyAuthentication=yes \
  -o PermitRootLogin=no \
  -o UsePAM=no
