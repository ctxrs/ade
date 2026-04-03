#!/usr/bin/env bash
set -euo pipefail

mode="${1:-}"
shift || true

data_dir=""
allow_user=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --data-dir)
      data_dir="${2:-}"
      shift 2
      ;;
    --allow-user)
      allow_user="${2:-}"
      shift 2
      ;;
    -h|--help)
      cat <<'EOF'
Usage:
  bootstrap.sh stage --data-dir <ctx-data-dir>
  bootstrap.sh status --data-dir <ctx-data-dir>
  bootstrap.sh activate --data-dir <ctx-data-dir> [--allow-user <username>]
EOF
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "${mode}" ]]; then
  echo "error: mode is required" >&2
  exit 2
fi

if [[ -z "${data_dir}" ]]; then
  echo "error: --data-dir is required" >&2
  exit 2
fi

bootstrap_root="${data_dir%/}/linux-sandbox-runtime"
cache_dir="${bootstrap_root}/cache"
downloads_dir="${cache_dir}/downloads"
debs_dir="${cache_dir}/debs"
status_path="${bootstrap_root}/status.json"
ready_marker="${bootstrap_root}/runtime-ready"
managed_nerdctl_path="/usr/local/bin/nerdctl"
wrapper_path="/usr/local/bin/ctx-rootful-nerdctl"
system_containerd_address="/run/containerd/containerd.sock"
system_containerd_namespace="default"
nerdctl_version="v2.2.1"

mkdir -p "${bootstrap_root}" "${downloads_dir}" "${debs_dir}"

json_escape() {
  printf '%s' "${1:-}" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

write_status() {
  local state="$1"
  local supported="$2"
  local message="${3:-}"
  local distro="${4:-}"
  cat > "${status_path}" <<EOF
{"state":"$(json_escape "${state}")","supported":${supported},"message":"$(json_escape "${message}")","distro":"$(json_escape "${distro}")"}
EOF
}

status_json() {
  if [[ -f "${status_path}" ]]; then
    cat "${status_path}"
    return 0
  fi
  cat <<'EOF'
{"state":"download_pending","supported":false,"message":"","distro":""}
EOF
}

detect_arch() {
  case "$(uname -m)" in
    x86_64|amd64)
      printf '%s' "amd64"
      ;;
    aarch64|arm64)
      printf '%s' "arm64"
      ;;
    *)
      echo "unsupported"
      ;;
  esac
}

nerdctl_expected_sha256() {
  case "$1" in
    amd64)
      printf '%s' "34144de7f12756aa4b9dc42a907fd95b0c5eb82a63566a650ca10c8abe7a26a0"
      ;;
    arm64)
      printf '%s' "abc83c9ac3d843c3442eedfb61c6456b8b59b1e4cd69f69598ca1582acc7c094"
      ;;
    *)
      return 1
      ;;
  esac
}

detect_distro() {
  if [[ ! -f /etc/os-release ]]; then
    echo "unknown"
    return 0
  fi
  # shellcheck disable=SC1091
  . /etc/os-release
  printf '%s' "${ID:-unknown}"
}

distro_supported() {
  case "$(detect_distro)" in
    ubuntu|debian)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

nerdctl_tarball_name() {
  local arch="$1"
  printf 'nerdctl-%s-linux-%s.tar.gz' "${nerdctl_version#v}" "${arch}"
}

staged_nerdctl_archive_path() {
  local arch="$1"
  printf '%s/%s' "${downloads_dir}" "$(nerdctl_tarball_name "${arch}")"
}

verify_nerdctl_checksum() {
  local arch="$1"
  local archive_path="$2"
  local expected
  expected="$(nerdctl_expected_sha256 "${arch}")"
  if [[ ! -f "${archive_path}" ]]; then
    return 1
  fi
  local actual
  actual="$(sha256sum "${archive_path}" | awk '{print $1}')"
  [[ "${actual}" == "${expected}" ]]
}

download_nerdctl() {
  local arch="$1"
  local tarball
  tarball="$(nerdctl_tarball_name "${arch}")"
  local url="https://github.com/containerd/nerdctl/releases/download/${nerdctl_version}/${tarball}"
  local dest
  dest="$(staged_nerdctl_archive_path "${arch}")"
  if verify_nerdctl_checksum "${arch}" "${dest}"; then
    return 0
  fi
  rm -f "${dest}"
  curl -fsSL "${url}" -o "${dest}"
  if ! verify_nerdctl_checksum "${arch}" "${dest}"; then
    rm -f "${dest}"
    echo "error: staged nerdctl archive failed checksum verification" >&2
    exit 1
  fi
}

stage_apt_debs() {
  if ! command -v apt >/dev/null 2>&1; then
    return 1
  fi
  (
    cd "${debs_dir}"
    apt download containerd containernetworking-plugins >/dev/null
  )
}

install_apt_requirements() {
  if ! command -v apt-get >/dev/null 2>&1; then
    echo "error: apt-get is required for Linux sandbox activation" >&2
    exit 1
  fi
  local deb_matches=()
  shopt -s nullglob
  deb_matches=("${debs_dir}"/containerd*.deb "${debs_dir}"/containernetworking-plugins*.deb)
  shopt -u nullglob
  if [[ ${#deb_matches[@]} -gt 0 ]]; then
    apt-get update
    apt-get install -y "${deb_matches[@]}"
  else
    apt-get update
    apt-get install -y containerd containernetworking-plugins
  fi
}

install_managed_nerdctl() {
  local arch="$1"
  local tarball
  tarball="$(staged_nerdctl_archive_path "${arch}")"
  if ! verify_nerdctl_checksum "${arch}" "${tarball}"; then
    echo "error: staged nerdctl archive is missing or invalid" >&2
    exit 1
  fi
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  tar -xzf "${tarball}" -C "${tmp_dir}" nerdctl
  install -m 0755 "${tmp_dir}/nerdctl" "${managed_nerdctl_path}"
  rm -rf "${tmp_dir}"
}

install_rootful_wrapper() {
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  cat > "${tmp_dir}/ctx-rootful-nerdctl" <<EOF
#!/usr/bin/env bash
set -euo pipefail
filtered_args=()
explicit_snapshotter=0
while [[ \$# -gt 0 ]]; do
  case "\$1" in
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
    *)
      filtered_args+=("\$1")
      shift
      ;;
  esac
done
if [[ "\${explicit_snapshotter}" -eq 0 ]]; then
  filtered_args=(--snapshotter native "\${filtered_args[@]}")
fi
exec sudo --non-interactive "${managed_nerdctl_path}" --address "${system_containerd_address}" --namespace "${system_containerd_namespace}" "\${filtered_args[@]}"
EOF
  install -m 0755 "${tmp_dir}/ctx-rootful-nerdctl" "${wrapper_path}"
  rm -rf "${tmp_dir}"
}

install_sudoers_rule() {
  local user_name="$1"
  if [[ -z "${user_name}" ]]; then
    echo "error: activation requires --allow-user" >&2
    exit 1
  fi
  local sanitized
  sanitized="$(printf '%s' "${user_name}" | tr -c 'A-Za-z0-9._-' '_')"
  local sudoers_path="/etc/sudoers.d/ctx-managed-nerdctl-${sanitized}"
  cat > "${sudoers_path}" <<EOF
Defaults:${user_name} !requiretty
${user_name} ALL=(root) NOPASSWD: ${managed_nerdctl_path}
EOF
  chmod 0440 "${sudoers_path}"
}

ensure_containerd_running() {
  if ! command -v systemctl >/dev/null 2>&1; then
    echo "error: systemctl is required for Linux sandbox activation" >&2
    exit 1
  fi
  systemctl enable --now containerd.service
  for _ in $(seq 1 20); do
    if [[ -S "${system_containerd_address}" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "error: containerd socket did not become ready" >&2
  exit 1
}

mark_ready() {
  mkdir -p "${bootstrap_root}"
  : > "${ready_marker}"
}

emit_current_status() {
  if [[ -f "${ready_marker}" && -x "${wrapper_path}" ]]; then
    write_status "ready" true "" "${distro}"
  elif [[ -f "${staged_archive_path}" ]]; then
    write_status "downloaded_not_activated" true "" "${distro}"
  else
    write_status "download_pending" true "" "${distro}"
  fi
  status_json
}

if [[ "$(uname -s)" != "Linux" ]]; then
  write_status "manual_runtime_required" false "Linux sandbox bootstrap is only available on Linux." ""
  status_json
  exit 0
fi

arch="$(detect_arch)"
if [[ "${arch}" == "unsupported" ]]; then
  write_status "failed" false "Unsupported Linux architecture." "$(detect_distro)"
  status_json
  exit 1
fi

distro="$(detect_distro)"
if ! distro_supported; then
  write_status "manual_runtime_required" false "Managed sandbox bootstrap is currently supported on Ubuntu/Debian only." "${distro}"
  status_json
  exit 0
fi

staged_archive_path="$(staged_nerdctl_archive_path "${arch}")"

if [[ "${mode}" == "status" ]]; then
  emit_current_status
  exit 0
fi

if [[ "${mode}" == "stage" ]]; then
  if [[ -f "${ready_marker}" && -x "${wrapper_path}" ]]; then
    write_status "ready" true "" "${distro}"
    status_json
    exit 0
  fi
  write_status "downloading" true "" "${distro}"
  download_nerdctl "${arch}"
  stage_apt_debs || true
  write_status "downloaded_not_activated" true "" "${distro}"
  status_json
  exit 0
fi

if [[ "${mode}" == "activate" ]]; then
  if [[ "$(id -u)" -ne 0 ]]; then
    echo "error: activation must run as root" >&2
    exit 1
  fi
  install_managed_nerdctl "${arch}"
  install_rootful_wrapper
  install_sudoers_rule "${allow_user}"
  install_apt_requirements
  ensure_containerd_running
  mark_ready
  write_status "ready" true "" "${distro}"
  status_json
  exit 0
fi

echo "error: unsupported mode: ${mode}" >&2
exit 2
