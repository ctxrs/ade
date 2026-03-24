#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir=""
runtime_arch=""
real_exec_mode="best-effort"

if [[ -d "${HOME}/.cargo/bin" ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH:-/usr/bin:/bin:/usr/sbin:/sbin}"
fi

usage() {
  cat <<'EOF'
usage: avf_linux_ci_smoke.sh --artifact-dir DIR [options]

Run a narrow macOS AVF Linux CI smoke:
  - build the desktop helper
  - build the Linux guest-agent
  - prepare a local Ubuntu guest runtime
  - stage that runtime through desktop_sync_resources
  - probe/helper lifecycle smoke
  - optionally attempt a real guest boot + exec smoke

Required:
  --artifact-dir DIR     Directory to write logs and JSON reports

Options:
  --runtime-arch ARCH    Guest runtime arch: x86_64 or arm64 (default: host arch)
  --real-exec MODE       one of: best-effort, required, skip (default: best-effort)
  -h, --help             Show this help
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || die "missing required command: $cmd"
}

normalize_arch() {
  case "$1" in
    x86_64|amd64) printf '%s' "x86_64" ;;
    arm64|aarch64) printf '%s' "arm64" ;;
    *) die "unsupported runtime arch: $1" ;;
  esac
}

host_arch() {
  case "$(uname -m)" in
    x86_64|amd64) printf '%s' "x86_64" ;;
    arm64|aarch64) printf '%s' "arm64" ;;
    *) die "unsupported host arch: $(uname -m)" ;;
  esac
}

linux_guest_target() {
  case "$1" in
    x86_64) printf '%s' "x86_64-unknown-linux-gnu" ;;
    arm64) printf '%s' "aarch64-unknown-linux-gnu" ;;
    *) die "unsupported runtime arch: $1" ;;
  esac
}

toolchain_for_host_arch() {
  case "$(host_arch)" in
    x86_64) printf '%s' "stable-x86_64-apple-darwin" ;;
    arm64) printf '%s' "stable-aarch64-apple-darwin" ;;
    *) die "unsupported host arch" ;;
  esac
}

json_get() {
  local file="$1"
  local expr="$2"
  node -e '
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
const expr = process.argv[2];
const value = Function("input", `return (${expr});`)(input);
if (value === undefined || value === null) process.exit(2);
if (typeof value === "object") {
  process.stdout.write(JSON.stringify(value));
} else {
  process.stdout.write(String(value));
}
' "$file" "$expr"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir)
      artifact_dir="${2:-}"
      shift 2
      ;;
    --runtime-arch)
      runtime_arch="${2:-}"
      shift 2
      ;;
    --real-exec)
      real_exec_mode="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "$artifact_dir" ]] || die "--artifact-dir is required"
artifact_dir="$(mkdir -p "$artifact_dir" && cd "$artifact_dir" && pwd)"

case "$real_exec_mode" in
  best-effort|required|skip) ;;
  *) die "unsupported --real-exec mode: $real_exec_mode" ;;
esac

if [[ -z "$runtime_arch" ]]; then
  runtime_arch="$(host_arch)"
else
  runtime_arch="$(normalize_arch "$runtime_arch")"
fi

need_cmd node
need_cmd cargo
need_cmd git
need_cmd qemu-img
need_cmd zig
need_cmd cargo-zigbuild
need_cmd codesign

helper_manifest="${repo_root}/apps/desktop/src-tauri/Cargo.toml"
helper_entitlements="${repo_root}/apps/desktop/src-tauri/ctx-avf-linux-helper.entitlements"
helper_target_root="${CARGO_TARGET_DIR:-${repo_root}/apps/desktop/src-tauri/target}"
guest_target_root="${CARGO_TARGET_DIR:-${repo_root}/target}"
helper_bin="${helper_target_root}/debug/ctx-avf-linux-helper"
guest_target="$(linux_guest_target "$runtime_arch")"
guest_agent_bin="${guest_target_root}/${guest_target}/release/ctx-avf-linux-guest-agent"
runtime_dir="${artifact_dir}/runtime"
bundle_dir="${artifact_dir}/bundle"
data_root="${artifact_dir}/daemon-data"
repo_dir="${artifact_dir}/repo"
probe_json="${artifact_dir}/helper-probe.json"
layout_json="${artifact_dir}/runtime-layout.json"
initial_state_json="${artifact_dir}/shared-vm-state.initial.json"
start_json="${artifact_dir}/shared-vm-state.start.json"
prepared_json="${artifact_dir}/guest-worktree.json"
nonpty_stdout="${artifact_dir}/guest-exec.stdout.log"
nonpty_stderr="${artifact_dir}/guest-exec.stderr.log"
pty_stdout="${artifact_dir}/guest-exec-pty.stdout.log"
pty_stderr="${artifact_dir}/guest-exec-pty.stderr.log"
staging_json="${artifact_dir}/staging-report.json"
report_json="${artifact_dir}/report.json"

toolchain="${CTX_AVF_GUEST_AGENT_TOOLCHAIN:-$(toolchain_for_host_arch)}"

mkdir -p "$artifact_dir" "$bundle_dir" "$data_root"

cleanup() {
  if [[ -x "$helper_bin" ]]; then
    "$helper_bin" stop-shared-vm "$data_root" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

echo "==> Building ctx-avf-linux-helper"
CTX_DESKTOP_SKIP_TAURI_BUILD=1 cargo build --locked --manifest-path "$helper_manifest" --bin ctx-avf-linux-helper
/usr/bin/codesign --force --sign - --entitlements "$helper_entitlements" "$helper_bin"

echo "==> Probing helper"
"$helper_bin" probe | tee "$probe_json"

echo "==> Building Linux guest-agent (${guest_target})"
CTX_AVF_GUEST_AGENT_TOOLCHAIN="$toolchain" bash "${repo_root}/scripts/build_avf_linux_guest_agent.sh" --release
[[ -f "$guest_agent_bin" ]] || die "guest-agent binary missing at $guest_agent_bin"

echo "==> Preparing local AVF guest runtime"
bash "${repo_root}/scripts/prepare_avf_linux_guest_runtime.sh" \
  --output-dir "$runtime_dir" \
  --arch "$runtime_arch" \
  --guest-agent "$guest_agent_bin" \
  --force

echo "==> Staging runtime through desktop_sync_resources"
CTX_AVF_LINUX_GUEST_RUNTIME_DIR="$runtime_dir" CTX_AVF_CI_CORE_ROOT="$repo_root" node - "$bundle_dir" "$staging_json" <<'NODE'
const fs = require("node:fs");
const path = require("node:path");
const bundleDir = process.argv[2];
const reportPath = process.argv[3];
const coreRoot = process.env.CTX_AVF_CI_CORE_ROOT;
const { stageAvfLinuxGuestRuntime } = require(path.join(coreRoot, "scripts", "desktop_sync_resources.cjs"));

fs.mkdirSync(bundleDir, { recursive: true });
fs.writeFileSync(
  path.join(bundleDir, "manifest.json"),
  JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2),
);

const staged = stageAvfLinuxGuestRuntime(bundleDir);
fs.writeFileSync(reportPath, JSON.stringify(staged, null, 2));
NODE

echo "==> Helper lifecycle smoke"
"$helper_bin" prepare-runtime-layout "$data_root" | tee "$layout_json"
"$helper_bin" shared-vm-state "$data_root" | tee "$initial_state_json"

real_exec_status="skipped"
real_exec_reason="real guest-exec smoke disabled"

if [[ "$real_exec_mode" != "skip" ]]; then
  real_exec_status="failed"
  real_exec_reason="real guest-exec smoke did not complete"

  rm -rf "$repo_dir"
  mkdir -p "$repo_dir"
  git -C "$repo_dir" init -q
  git -C "$repo_dir" config user.email ci@example.invalid
  git -C "$repo_dir" config user.name "ctx AVF CI"
  printf 'hello\n' > "${repo_dir}/hello.txt"
  git -C "$repo_dir" add hello.txt
  git -C "$repo_dir" commit -q -m "initial"
  base_commit="$(git -C "$repo_dir" rev-parse HEAD)"

  echo "==> Starting shared VM"
  if "$helper_bin" start-shared-vm \
    "$data_root" \
    "$runtime_dir" \
    "${runtime_dir}/rootfs.raw" \
    "${runtime_dir}/helpers/kernel" \
    "${runtime_dir}/helpers/initrd" \
    "ci-${runtime_arch}" | tee "$start_json"
  then
    workspace_id="ws-avf-ci-smoke"
    worktree_id="wt-avf-ci-smoke"
    guest_root="/ctx/ws/worktrees/${worktree_id}"

    prepare_ok=0
    for _ in $(seq 1 60); do
      if "$helper_bin" prepare-guest-worktree \
        "$data_root" \
        "$workspace_id" \
        "$worktree_id" \
        "$repo_dir" \
        "$base_commit" \
        "ctx/avf-ci-smoke" >"$prepared_json" 2>"${artifact_dir}/prepare-guest-worktree.stderr.log"
      then
        prepare_ok=1
        break
      fi
      sleep 2
    done

    if [[ "$prepare_ok" -eq 1 ]]; then
      set +e
      "$helper_bin" guest-exec \
        --data-root "$data_root" \
        --workspace-id "$workspace_id" \
        --worktree-id "$worktree_id" \
        --cwd "$guest_root" \
        --command /bin/sh \
        -- \
        -lc 'printf "ci-avf-ok\n"; pwd; ls -1; test -f hello.txt; cat hello.txt' \
        >"$nonpty_stdout" 2>"$nonpty_stderr"
      nonpty_status=$?
      set -e

      if [[ "$nonpty_status" -eq 0 ]]; then
        set +e
        "$helper_bin" guest-exec \
          --data-root "$data_root" \
          --workspace-id "$workspace_id" \
          --worktree-id "$worktree_id" \
          --cwd "$guest_root" \
          --pty \
          --command /bin/sh \
          -- \
          -lc 'printf "ci-avf-pty-ok\n"; pwd' \
          >"$pty_stdout" 2>"$pty_stderr"
        pty_status=$?
        set -e

        if [[ "$pty_status" -eq 0 ]]; then
          real_exec_status="passed"
          real_exec_reason="real shared VM booted and guest exec succeeded"
        else
          real_exec_reason="PTY guest-exec failed with status ${pty_status}"
        fi
      else
        real_exec_reason="non-PTY guest-exec failed with status ${nonpty_status}"
      fi
    else
      real_exec_reason="prepare-guest-worktree did not become ready within timeout"
    fi
  else
    real_exec_reason="start-shared-vm failed"
  fi

  if [[ "$real_exec_mode" == "required" && "$real_exec_status" != "passed" ]]; then
    echo "error: ${real_exec_reason}" >&2
  fi
fi

node - "$report_json" \
  "$probe_json" \
  "$layout_json" \
  "$initial_state_json" \
  "$start_json" \
  "$prepared_json" \
  "$staging_json" \
  "$runtime_dir" \
  "$artifact_dir" \
  "$real_exec_status" \
  "$real_exec_reason" \
  "$real_exec_mode" \
  "$runtime_arch" <<'NODE'
const fs = require("node:fs");
const [
  ,
  ,
  reportPath,
  probePath,
  layoutPath,
  initialStatePath,
  startPath,
  preparedPath,
  stagingPath,
  runtimeDir,
  artifactDir,
  realExecStatus,
  realExecReason,
  realExecMode,
  runtimeArch,
] = process.argv;

const maybeJson = (file) => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
};

const report = {
  generated_at: new Date().toISOString(),
  host_os: process.platform,
  host_arch: process.arch,
  runtime_arch: runtimeArch,
  real_exec_mode: realExecMode,
  helper_probe: maybeJson(probePath),
  runtime_layout: maybeJson(layoutPath),
  initial_shared_vm_state: maybeJson(initialStatePath),
  start_shared_vm_state: maybeJson(startPath),
  guest_worktree: maybeJson(preparedPath),
  staging: maybeJson(stagingPath),
  runtime_dir: runtimeDir,
  artifact_dir: artifactDir,
  real_exec: {
    status: realExecStatus,
    reason: realExecReason,
  },
};

fs.writeFileSync(reportPath, JSON.stringify(report, null, 2));
NODE

echo "==> AVF CI smoke report: ${report_json}"

if [[ "$real_exec_mode" == "required" && "$real_exec_status" != "passed" ]]; then
  exit 1
fi
