#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install_linux_sandbox_runtime_cli.sh [--require-reachable]

Installs the Linux sandbox runtime CLI (`nerdctl`) on the current machine if it is
not already available. On Linux CI runners this is used to make native sandbox
lanes prove the intended runtime contract instead of silently falling back.

Options:
  --require-reachable   Also require `nerdctl info` to succeed after install.
  -h, --help            Show help.
EOF
}

require_reachable=0
case "${1:-}" in
  "")
    ;;
  --require-reachable)
    require_reachable=1
    ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    echo "error: unknown argument: ${1}" >&2
    usage >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: this installer only supports Linux runners" >&2
  exit 1
fi

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)
      printf '%s' "amd64"
      ;;
    aarch64|arm64)
      printf '%s' "arm64"
      ;;
    *)
      echo "error: unsupported Linux architecture: $(uname -m)" >&2
      exit 1
      ;;
  esac
}

ensure_sudo_prefix() {
  if [[ -w "/usr/local/bin" ]]; then
    return 0
  fi
  if [[ "$(id -u)" -eq 0 ]]; then
    return 0
  fi
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: /usr/local/bin is not writable and sudo is unavailable" >&2
    exit 1
  fi
}

run_with_optional_sudo() {
  if [[ "$(id -u)" -eq 0 ]]; then
    "$@"
  else
    sudo "$@"
  fi
}

install_nerdctl() {
  local arch="$1"
  local version="${NERDCTL_VERSION:-v2.2.1}"
  local version_trimmed="${version#v}"
  local url="https://github.com/containerd/nerdctl/releases/download/${version}/nerdctl-${version_trimmed}-linux-${arch}.tar.gz"
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "${tmp_dir}"' RETURN

  echo "installing nerdctl ${version} for linux/${arch} from ${url}"
  curl -fsSL "${url}" -o "${tmp_dir}/nerdctl.tar.gz"
  tar -xzf "${tmp_dir}/nerdctl.tar.gz" -C "${tmp_dir}" nerdctl

  ensure_sudo_prefix
  if [[ -w "/usr/local/bin" || "$(id -u)" -eq 0 ]]; then
    install -m 0755 "${tmp_dir}/nerdctl" /usr/local/bin/nerdctl
  else
    sudo install -m 0755 "${tmp_dir}/nerdctl" /usr/local/bin/nerdctl
  fi
}

resolve_nerdctl_path() {
  if command -v nerdctl >/dev/null 2>&1; then
    command -v nerdctl
    return 0
  fi
  if [[ -x "/usr/local/bin/nerdctl" ]]; then
    printf '%s\n' "/usr/local/bin/nerdctl"
    return 0
  fi
  echo "error: nerdctl was not found after install" >&2
  exit 1
}

ensure_containerd_reachable() {
  local nerdctl_path="$1"
  if "${nerdctl_path}" info >/dev/null 2>&1; then
    return 0
  fi
  if ! command -v systemctl >/dev/null 2>&1; then
    echo "error: nerdctl is installed but the native sandbox runtime is not reachable and systemctl is unavailable" >&2
    exit 1
  fi
  if ! command -v containerd >/dev/null 2>&1; then
    if ! command -v apt-get >/dev/null 2>&1; then
      echo "error: containerd is missing and apt-get is unavailable on this runner" >&2
      exit 1
    fi
    ensure_sudo_prefix
    run_with_optional_sudo apt-get update
    run_with_optional_sudo apt-get install -y containerd
  fi
  ensure_sudo_prefix
  if ! run_with_optional_sudo systemctl enable --now containerd.service; then
    run_with_optional_sudo systemctl status containerd.service --no-pager || true
    echo "error: failed to start containerd.service on this runner" >&2
    exit 1
  fi
  local attempt
  for attempt in $(seq 1 15); do
    if "${nerdctl_path}" info >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  run_with_optional_sudo systemctl status containerd.service --no-pager || true
  "${nerdctl_path}" info || true
  echo "error: nerdctl is installed but the native sandbox runtime is not reachable after starting containerd.service" >&2
  exit 1
}

if ! command -v nerdctl >/dev/null 2>&1; then
  install_nerdctl "$(detect_arch)"
fi

nerdctl_path="$(resolve_nerdctl_path)"
echo "nerdctl ready: ${nerdctl_path}"
"${nerdctl_path}" --version

if [[ "${require_reachable}" == "1" ]]; then
  ensure_containerd_reachable "${nerdctl_path}"
  echo "nerdctl info: reachable"
fi
