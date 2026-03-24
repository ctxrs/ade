#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir=""
runtime_arch=""
prepared_runtime_dir=""
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
  --prepared-runtime DIR Reuse an already prepared AVF guest runtime instead of downloading one
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
    --prepared-runtime)
      prepared_runtime_dir="${2:-}"
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
ctx_bin="${guest_target_root}/debug/ctx"
guest_target="$(linux_guest_target "$runtime_arch")"
guest_agent_bin="${guest_target_root}/${guest_target}/release/ctx-avf-linux-guest-agent"
egress_proxy_bin="${guest_target_root}/${guest_target}/release/ctx-egress-proxy"
runtime_dir="${artifact_dir}/runtime"
bundle_dir="${artifact_dir}/bundle"
data_root="${artifact_dir}/daemon-data"
repo_dir="${artifact_dir}/repo"
probe_json="${artifact_dir}/helper-probe.json"
layout_json="${artifact_dir}/runtime-layout.json"
initial_state_json="${artifact_dir}/workspace-vm-state.initial.json"
start_json="${artifact_dir}/workspace-vm-state.start.json"
stop1_json="${artifact_dir}/workspace-vm-state.stop1.json"
start2_json="${artifact_dir}/workspace-vm-state.start2.json"
stop2_json="${artifact_dir}/workspace-vm-state.stop2.json"
prepared_json="${artifact_dir}/guest-worktree.json"
prepared2_json="${artifact_dir}/guest-worktree.restore.json"
nonpty_stdout="${artifact_dir}/guest-exec.stdout.log"
nonpty_stderr="${artifact_dir}/guest-exec.stderr.log"
pty_stdout="${artifact_dir}/guest-exec-pty.stdout.log"
pty_stderr="${artifact_dir}/guest-exec-pty.stderr.log"
nonpty2_stdout="${artifact_dir}/guest-exec.restore.stdout.log"
nonpty2_stderr="${artifact_dir}/guest-exec.restore.stderr.log"
staging_json="${artifact_dir}/staging-report.json"
daemon_stdout="${artifact_dir}/ctx-daemon.stdout.log"
daemon_stderr="${artifact_dir}/ctx-daemon.stderr.log"
daemon_health_stdout="${artifact_dir}/daemon-health.stdout.log"
daemon_health_stderr="${artifact_dir}/daemon-health.stderr.log"
daemon_health_restricted_stdout="${artifact_dir}/daemon-health.restricted.stdout.log"
daemon_health_restricted_stderr="${artifact_dir}/daemon-health.restricted.stderr.log"
restricted_block_stdout="${artifact_dir}/restricted-block.stdout.log"
restricted_block_stderr="${artifact_dir}/restricted-block.stderr.log"
report_json="${artifact_dir}/report.json"

toolchain="${CTX_AVF_GUEST_AGENT_TOOLCHAIN:-$(toolchain_for_host_arch)}"
guest_python_fetch_code='import sys, urllib.request; print(urllib.request.urlopen(sys.argv[1], timeout=10).read().decode(), end="")'
ctx_daemon_port=43991
ctx_daemon_loopback_bind="127.0.0.1:${ctx_daemon_port}"
ctx_daemon_gateway_bind="192.168.64.1:${ctx_daemon_port}"
ctx_daemon_gateway_url="http://192.168.64.1:${ctx_daemon_port}/api/health"
ctx_daemon_loopback_url="http://127.0.0.1:${ctx_daemon_port}/api/health"
daemon_pid=""

mkdir -p "$artifact_dir" "$bundle_dir" "$data_root"

cleanup() {
  if [[ -n "$daemon_pid" ]]; then
    kill "$daemon_pid" >/dev/null 2>&1 || true
    wait "$daemon_pid" >/dev/null 2>&1 || true
  fi
  if [[ -x "$helper_bin" ]]; then
    "$helper_bin" stop-workspace-vm "$data_root" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

echo "==> Building ctx-avf-linux-helper"
# `apps/desktop/src-tauri` is an intentionally standalone Cargo package and does not keep its own
# checked-in Cargo.lock. Match the normal `desktop:prep` path instead of forcing `--locked` here.
CTX_DESKTOP_SKIP_TAURI_BUILD=1 cargo build --manifest-path "$helper_manifest" --bin ctx-avf-linux-helper
/usr/bin/codesign --force --sign - --entitlements "$helper_entitlements" "$helper_bin"

echo "==> Probing helper"
"$helper_bin" probe | tee "$probe_json"
helper_host_arch="$(json_get "$probe_json" 'input.host_arch')"

echo "==> Building ctx daemon"
cargo build --locked --manifest-path "${repo_root}/Cargo.toml" -p ctx-http --bin ctx
[[ -x "$ctx_bin" ]] || die "ctx binary missing at $ctx_bin"

if [[ -n "$prepared_runtime_dir" ]]; then
  runtime_dir="$(cd "$prepared_runtime_dir" && pwd)"
  echo "==> Reusing prepared AVF guest runtime at ${runtime_dir}"
else
  echo "==> Building Linux guest-agent (${guest_target})"
  CTX_AVF_GUEST_AGENT_TOOLCHAIN="$toolchain" bash "${repo_root}/scripts/build_avf_linux_guest_agent.sh" --release
  [[ -f "$guest_agent_bin" ]] || die "guest-agent binary missing at $guest_agent_bin"
  [[ -f "$egress_proxy_bin" ]] || die "egress-proxy binary missing at $egress_proxy_bin"

  echo "==> Preparing local AVF guest runtime"
  bash "${repo_root}/scripts/prepare_avf_linux_guest_runtime.sh" \
    --output-dir "$runtime_dir" \
    --arch "$runtime_arch" \
    --guest-agent "$guest_agent_bin" \
    --egress-proxy "$egress_proxy_bin" \
    --force
fi

if [[ -n "$prepared_runtime_dir" ]]; then
  echo "==> Recording reused runtime metadata"
  node - "$runtime_dir" "$staging_json" <<'NODE'
const fs = require("node:fs");
const runtimeDir = process.argv[2];
const reportPath = process.argv[3];
const helpersDir = `${runtimeDir}/helpers`;
const report = {
  reusedPreparedRuntime: true,
  sourceDir: runtimeDir,
  rootfsPath: `${runtimeDir}/rootfs.raw`,
  kernelPath: `${helpersDir}/kernel`,
  initrdPath: `${helpersDir}/initrd`,
  guestAgentPath: `${helpersDir}/guest-agent`,
  egressProxyPath: `${helpersDir}/egress-proxy`,
};
fs.writeFileSync(reportPath, JSON.stringify(report, null, 2));
NODE
else
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
fi

echo "==> Helper lifecycle smoke"
"$helper_bin" prepare-runtime-layout "$data_root" | tee "$layout_json"
"$helper_bin" workspace-vm-state "$data_root" | tee "$initial_state_json"

real_exec_status="skipped"
real_exec_reason="real guest-exec smoke disabled"
restore_status="skipped"
restore_reason="workspace VM restore smoke disabled"
daemon_probe_status="skipped"
daemon_probe_reason="daemon reachability smoke disabled"
restricted_network_status="skipped"
restricted_network_reason="restricted-network smoke disabled"

if [[ "$real_exec_mode" != "skip" ]]; then
  real_exec_status="failed"
  real_exec_reason="real guest-exec smoke did not complete"
  restore_status="failed"
  restore_reason="workspace VM restore smoke did not complete"
  daemon_probe_status="failed"
  daemon_probe_reason="daemon reachability smoke did not complete"
  restricted_network_status="failed"
  restricted_network_reason="restricted-network smoke did not complete"

  rm -rf "$repo_dir"
  mkdir -p "$repo_dir"
  git -C "$repo_dir" init -q
  git -C "$repo_dir" config user.email ci@example.invalid
  git -C "$repo_dir" config user.name "ctx AVF CI"
  printf 'hello\n' > "${repo_dir}/hello.txt"
  git -C "$repo_dir" add hello.txt
  git -C "$repo_dir" commit -q -m "initial"
  base_commit="$(git -C "$repo_dir" rev-parse HEAD)"

  echo "==> Starting workspace VM"
  if "$helper_bin" start-workspace-vm \
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
      echo "==> Starting ctx daemon for guest reachability checks"
      set +e
      "$ctx_bin" serve \
        --data-dir "${artifact_dir}/ctx-daemon-data" \
        --bind "$ctx_daemon_loopback_bind" \
        --bind "$ctx_daemon_gateway_bind" \
        >"$daemon_stdout" 2>"$daemon_stderr" &
      daemon_pid=$!
      set -e

      daemon_host_ok=0
      for _ in $(seq 1 30); do
        if curl -fsS "$ctx_daemon_loopback_url" >"${artifact_dir}/ctx-daemon.health.json" 2>/dev/null; then
          daemon_host_ok=1
          break
        fi
        sleep 1
      done

      if [[ "$daemon_host_ok" -eq 1 ]]; then
        set +e
        "$helper_bin" guest-exec \
          --data-root "$data_root" \
          --workspace-id "$workspace_id" \
          --worktree-id "$worktree_id" \
          --cwd "$guest_root" \
          --command /usr/bin/env \
          -- \
          python3 -c "$guest_python_fetch_code" "$ctx_daemon_gateway_url" \
          >"$daemon_health_stdout" 2>"$daemon_health_stderr"
        daemon_probe_guest_status=$?
        set -e

        if [[ "$daemon_probe_guest_status" -eq 0 ]]; then
          daemon_probe_status="passed"
          daemon_probe_reason="guest reached ctx daemon over the AVF gateway address"

          proxy_config_json='{"listen":"127.0.0.1:8787","mode":"allowlist","allowlist":[],"max_peek_bytes":16384}'
          read -r -d '' restricted_setup_script <<EOF || true
set -e
command -v iptables >/dev/null 2>&1
test -x /usr/local/bin/ctx-egress-proxy
mkdir -p /var/lib/ctx
printf '%s\n' '${proxy_config_json}' > /var/lib/ctx/egress-proxy.json
chmod 0644 /var/lib/ctx/egress-proxy.json
pid_file="/tmp/ctx-egress-proxy.pid"
if [ -f "\$pid_file" ]; then
  old_pid="\$(cat "\$pid_file" 2>/dev/null || true)"
  if [ -n "\$old_pid" ]; then
    kill "\$old_pid" || true
  fi
  rm -f "\$pid_file"
fi
if command -v nohup >/dev/null 2>&1; then
  nohup /usr/local/bin/ctx-egress-proxy --config /var/lib/ctx/egress-proxy.json >/tmp/ctx-egress-proxy.log 2>&1 &
elif command -v setsid >/dev/null 2>&1; then
  setsid /usr/local/bin/ctx-egress-proxy --config /var/lib/ctx/egress-proxy.json >/tmp/ctx-egress-proxy.log 2>&1 &
else
  /usr/local/bin/ctx-egress-proxy --config /var/lib/ctx/egress-proxy.json >/tmp/ctx-egress-proxy.log 2>&1 &
fi
echo \$! > "\$pid_file"
daemon_ip="\$(getent hosts 192.168.64.1 | awk '{print \$1}' | head -n1)"
if [ -z "\$daemon_ip" ]; then
  echo "daemon host not resolvable inside guest" >&2
  exit 44
fi
iptables -t nat -F OUTPUT || true
iptables -F OUTPUT || true
iptables -P OUTPUT DROP
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
iptables -A OUTPUT -d "\$daemon_ip" -p tcp --dport ${ctx_daemon_port} -j ACCEPT
iptables -A OUTPUT -m owner --uid-owner 0 -j ACCEPT
iptables -t nat -A OUTPUT -m owner --uid-owner 0 -j RETURN
iptables -t nat -A OUTPUT -p tcp --dport 80 -j REDIRECT --to-ports 8787
iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-ports 8787
EOF

          set +e
          "$helper_bin" guest-exec \
            --data-root "$data_root" \
            --workspace-id "$workspace_id" \
            --worktree-id "$worktree_id" \
            --cwd "$guest_root" \
            --user root \
            --command /bin/sh \
            -- \
            -lc "$restricted_setup_script" \
            >"${artifact_dir}/restricted-setup.stdout.log" 2>"${artifact_dir}/restricted-setup.stderr.log"
          restricted_setup_status=$?
          set -e

          if [[ "$restricted_setup_status" -eq 0 ]]; then
            set +e
            "$helper_bin" guest-exec \
              --data-root "$data_root" \
              --workspace-id "$workspace_id" \
              --worktree-id "$worktree_id" \
              --cwd "$guest_root" \
              --command /usr/bin/env \
              -- \
              python3 -c "$guest_python_fetch_code" "$ctx_daemon_gateway_url" \
              >"$daemon_health_restricted_stdout" 2>"$daemon_health_restricted_stderr"
            daemon_restricted_status=$?
            "$helper_bin" guest-exec \
              --data-root "$data_root" \
              --workspace-id "$workspace_id" \
              --worktree-id "$worktree_id" \
              --cwd "$guest_root" \
              --command /usr/bin/env \
              -- \
              python3 -c "$guest_python_fetch_code" "http://example.com/" \
              >"$restricted_block_stdout" 2>"$restricted_block_stderr"
            blocked_probe_status=$?
            set -e

            if [[ "$daemon_restricted_status" -eq 0 && "$blocked_probe_status" -ne 0 ]]; then
              restricted_network_status="passed"
              restricted_network_reason="restricted guest egress kept daemon access while blocking a normal external HTTP target"
            elif [[ "$daemon_restricted_status" -ne 0 ]]; then
              restricted_network_reason="daemon health probe failed after restricted egress policy was applied"
            else
              restricted_network_reason="external HTTP probe unexpectedly succeeded under restricted egress policy"
            fi

            set +e
            "$helper_bin" guest-exec \
              --data-root "$data_root" \
              --workspace-id "$workspace_id" \
              --worktree-id "$worktree_id" \
              --cwd "$guest_root" \
              --user root \
              --command /bin/sh \
              -- \
              -lc 'set -e; pid_file="/tmp/ctx-egress-proxy.pid"; if [ -f "$pid_file" ]; then old_pid="$(cat "$pid_file" 2>/dev/null || true)"; if [ -n "$old_pid" ]; then kill "$old_pid" || true; fi; rm -f "$pid_file"; fi; if command -v iptables >/dev/null 2>&1; then iptables -t nat -F OUTPUT; iptables -F OUTPUT; iptables -P OUTPUT ACCEPT; fi' \
              >"${artifact_dir}/restricted-cleanup.stdout.log" 2>"${artifact_dir}/restricted-cleanup.stderr.log"
            set -e
          else
            restricted_network_reason="failed to install restricted egress policy inside the guest"
          fi
        else
          daemon_probe_reason="guest could not reach ctx daemon over the AVF gateway address"
          restricted_network_reason="skipped because daemon reachability failed"
        fi
      else
        daemon_probe_reason="ctx daemon never became healthy on host loopback"
        restricted_network_reason="skipped because daemon failed to start"
      fi

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
          set +e
          "$helper_bin" stop-workspace-vm "$data_root" >"$stop1_json"
          stop1_status=$?
          set -e

          if [[ "$stop1_status" -eq 0 ]]; then
            set +e
            "$helper_bin" start-workspace-vm \
              "$data_root" \
              "$runtime_dir" \
              "${runtime_dir}/rootfs.raw" \
              "${runtime_dir}/helpers/kernel" \
              "${runtime_dir}/helpers/initrd" \
              "ci-${runtime_arch}" >"$start2_json"
            start2_status=$?
            set -e

            if [[ "$start2_status" -eq 0 ]]; then
              prepare2_ok=0
              for _ in $(seq 1 60); do
                if "$helper_bin" prepare-guest-worktree \
                  "$data_root" \
                  "$workspace_id" \
                  "$worktree_id" \
                  "$repo_dir" \
                  "$base_commit" \
                  "ctx/avf-ci-smoke" >"$prepared2_json" 2>"${artifact_dir}/prepare-guest-worktree.restore.stderr.log"
                then
                  prepare2_ok=1
                  break
                fi
                sleep 2
              done

              if [[ "$prepare2_ok" -eq 1 ]]; then
                set +e
                "$helper_bin" guest-exec \
                  --data-root "$data_root" \
                  --workspace-id "$workspace_id" \
                  --worktree-id "$worktree_id" \
                  --cwd "$guest_root" \
                  --command /bin/sh \
                  -- \
                  -lc 'printf "ci-avf-restore-ok\n"; pwd; test -f hello.txt; cat hello.txt' \
                  >"$nonpty2_stdout" 2>"$nonpty2_stderr"
                nonpty2_status=$?
                set -e

                if [[ "$nonpty2_status" -eq 0 ]]; then
                  set +e
                  "$helper_bin" stop-workspace-vm "$data_root" >"$stop2_json"
                  stop2_status=$?
                  set -e

                  if [[ "$stop2_status" -eq 0 ]]; then
                    if grep -q "restored workspace VM state" "${data_root}/managed/vms/avf-linux/macos/${helper_host_arch}/shared/logs/shared-vm.log" 2>/dev/null; then
                      restore_status="passed"
                      restore_reason="workspace VM resumed from saved state and second guest exec succeeded"
                      real_exec_status="passed"
                      real_exec_reason="workspace VM booted, guest exec succeeded, and second start/exec completed"
                    elif [[ "$(host_arch)" == "arm64" ]]; then
                      restore_status="failed"
                      restore_reason="workspace VM second start completed but did not restore saved state on an Apple silicon host"
                      real_exec_reason="workspace VM second start skipped save/restore on an Apple silicon host"
                    else
                      restore_status="passed"
                      restore_reason="workspace VM restarted and second guest exec succeeded without a saved-state restore"
                      real_exec_status="passed"
                      real_exec_reason="workspace VM booted, guest exec succeeded, and second start/exec completed"
                    fi
                  else
                    restore_reason="second workspace-vm stop failed with status ${stop2_status}"
                    real_exec_reason="workspace VM restore smoke failed after the second guest exec"
                  fi
                else
                  restore_reason="second non-PTY guest-exec failed with status ${nonpty2_status}"
                  real_exec_reason="workspace VM restore smoke failed after the second start"
                fi
              else
                restore_reason="prepare-guest-worktree after second start did not become ready within timeout"
                real_exec_reason="workspace VM restore smoke failed after the second start"
              fi
            else
              restore_reason="second start-workspace-vm failed with status ${start2_status}"
              real_exec_reason="workspace VM restore smoke failed after the first stop"
            fi
          else
            restore_reason="first workspace-vm stop failed with status ${stop1_status}"
            real_exec_reason="workspace VM stop/save failed after the first guest exec"
          fi
        else
          real_exec_reason="PTY guest-exec failed with status ${pty_status}"
        fi
      else
        real_exec_reason="non-PTY guest-exec failed with status ${nonpty_status}"
      fi
    else
      real_exec_reason="prepare-guest-worktree did not become ready within timeout"
      daemon_probe_reason="prepare-guest-worktree did not become ready within timeout"
      restricted_network_reason="prepare-guest-worktree did not become ready within timeout"
    fi
  else
    real_exec_reason="start-workspace-vm failed"
    daemon_probe_reason="start-workspace-vm failed"
    restricted_network_reason="start-workspace-vm failed"
  fi

  if [[ "$real_exec_mode" == "required" ]]; then
    if [[ "$daemon_probe_status" != "passed" ]]; then
      echo "error: ${daemon_probe_reason}" >&2
    elif [[ "$restricted_network_status" != "passed" ]]; then
      echo "error: ${restricted_network_reason}" >&2
    elif [[ "$real_exec_status" != "passed" ]]; then
      echo "error: ${real_exec_reason}" >&2
    fi
  fi
fi

node - "$report_json" \
  "$probe_json" \
  "$layout_json" \
  "$initial_state_json" \
  "$start_json" \
  "$stop1_json" \
  "$start2_json" \
  "$stop2_json" \
  "$prepared_json" \
  "$prepared2_json" \
  "$staging_json" \
  "$runtime_dir" \
  "$artifact_dir" \
  "$real_exec_status" \
  "$real_exec_reason" \
  "$restore_status" \
  "$restore_reason" \
  "$daemon_probe_status" \
  "$daemon_probe_reason" \
  "$restricted_network_status" \
  "$restricted_network_reason" \
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
  stop1Path,
  start2Path,
  stop2Path,
  preparedPath,
  prepared2Path,
  stagingPath,
  runtimeDir,
  artifactDir,
  realExecStatus,
  realExecReason,
  restoreStatus,
  restoreReason,
  daemonProbeStatus,
  daemonProbeReason,
  restrictedNetworkStatus,
  restrictedNetworkReason,
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
  stop1_shared_vm_state: maybeJson(stop1Path),
  start2_shared_vm_state: maybeJson(start2Path),
  stop2_shared_vm_state: maybeJson(stop2Path),
  guest_worktree: maybeJson(preparedPath),
  guest_worktree_restore: maybeJson(prepared2Path),
  staging: maybeJson(stagingPath),
  runtime_dir: runtimeDir,
  artifact_dir: artifactDir,
  real_exec: {
    status: realExecStatus,
    reason: realExecReason,
  },
  restore_exec: {
    status: restoreStatus,
    reason: restoreReason,
  },
  daemon_probe: {
    status: daemonProbeStatus,
    reason: daemonProbeReason,
  },
  restricted_network: {
    status: restrictedNetworkStatus,
    reason: restrictedNetworkReason,
  },
};

fs.writeFileSync(reportPath, JSON.stringify(report, null, 2));
NODE

echo "==> AVF CI smoke report: ${report_json}"

if [[ "$real_exec_mode" == "required" && ( "$daemon_probe_status" != "passed" || "$restricted_network_status" != "passed" || "$real_exec_status" != "passed" ) ]]; then
  exit 1
fi
