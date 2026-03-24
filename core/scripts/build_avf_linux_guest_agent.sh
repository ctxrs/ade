#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
toolchain="${CTX_AVF_GUEST_AGENT_TOOLCHAIN:-stable-aarch64-apple-darwin}"
cargo_home_bin="${HOME}/.cargo/bin"
default_path="${cargo_home_bin}:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"

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

extra_args=("$@")

for target in "${targets[@]}"; do
  rustup target add --toolchain "${toolchain}" "${target}" >/dev/null
  cargo_cmd=(
    cargo +"${toolchain}" zigbuild
    -p ctx-avf-linux-guest-agent
    --manifest-path "${repo_root}/Cargo.toml"
    --target "${target}"
  )
  if ((${#extra_args[@]} > 0)); then
    cargo_cmd+=("${extra_args[@]}")
  fi
  "${cargo_cmd[@]}"
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

echo "ctx-avf-linux-guest-agent build complete:"
for target in "${targets[@]}"; do
  binary_path="${repo_root}/target/${target}/${profile_dir}/ctx-avf-linux-guest-agent"
  if [[ -f "${binary_path}" ]]; then
    printf '  %s\n' "${binary_path}"
  fi
done
