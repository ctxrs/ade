#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
toolchain="${CTX_AVF_GUEST_AGENT_TOOLCHAIN:-stable-aarch64-apple-darwin}"
cargo_home_bin="${HOME}/.cargo/bin"
default_path="${cargo_home_bin}:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
target_root="${CTX_AVF_GUEST_HELPER_TARGET_ROOT:-${repo_root}/target}"

if [[ -n "${PATH:-}" ]]; then
  export PATH="${cargo_home_bin}:${PATH}"
else
  export PATH="${default_path}"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found; install rustup first" >&2
  exit 1
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "error: rustup not found; install rustup first" >&2
  exit 1
fi

rustup_bin="$(command -v rustup)"

if ! command -v zig >/dev/null 2>&1; then
  echo "error: zig not found; install zig first" >&2
  exit 1
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "error: cargo-zigbuild not found; install with 'cargo install cargo-zigbuild'" >&2
  exit 1
fi

targets=(
  "aarch64-unknown-linux-gnu"
  "x86_64-unknown-linux-gnu"
)

if [[ -n "${CTX_AVF_GUEST_HELPER_TARGETS:-}" ]]; then
  requested_targets=()
  read -r -a raw_targets <<<"${CTX_AVF_GUEST_HELPER_TARGETS//,/ }"
  for target in "${raw_targets[@]}"; do
    [[ -n "${target}" ]] || continue
    case "${target}" in
      aarch64-unknown-linux-gnu|x86_64-unknown-linux-gnu)
        requested_targets+=("${target}")
        ;;
      *)
        echo "error: unsupported CTX_AVF_GUEST_HELPER_TARGETS entry: ${target}" >&2
        exit 1
        ;;
    esac
  done
  if ((${#requested_targets[@]} == 0)); then
    echo "error: CTX_AVF_GUEST_HELPER_TARGETS did not include any valid targets" >&2
    exit 1
  fi
  targets=("${requested_targets[@]}")
fi

packages=(
  "ctx-avf-linux-guest-agent"
  "ctx-egress-proxy"
)

extra_args=("$@")

for target in "${targets[@]}"; do
  rustup target add --toolchain "${toolchain}" "${target}" >/dev/null
  for package in "${packages[@]}"; do
    cargo_cmd=(
      "${rustup_bin}" run "${toolchain}" cargo zigbuild
      -p "${package}"
      --manifest-path "${repo_root}/Cargo.toml"
      --target "${target}"
    )
    if ((${#extra_args[@]} > 0)); then
      cargo_cmd+=("${extra_args[@]}")
    fi
    CARGO_TARGET_DIR="${target_root}" "${cargo_cmd[@]}"
  done
done

profile_dir="debug"
if ((${#extra_args[@]} > 0)); then
  for arg in "${extra_args[@]}"; do
    if [[ "${arg}" == "--release" ]]; then
      profile_dir="release"
      break
    fi
  done
fi

echo "AVF Linux guest helper build complete:"
for target in "${targets[@]}"; do
  for package in "${packages[@]}"; do
    binary_path="${target_root}/${target}/${profile_dir}/${package}"
    if [[ -f "${binary_path}" ]]; then
      printf '  %s\n' "${binary_path}"
    fi
  done
done
