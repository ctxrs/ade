#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
artifact_dir=""
runtime_arch=""
prepared_runtime_dir=""
real_exec_mode="best-effort"
restore_smoke_mode="required"

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
  --restore-smoke MODE   one of: required, skip (default: required)
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

cargo_target_dir() {
  local manifest="$1"
  cargo metadata --manifest-path "$manifest" --format-version 1 --no-deps | node -e '
const fs = require("node:fs");
const input = JSON.parse(fs.readFileSync(0, "utf8"));
const dir = typeof input.target_directory === "string" ? input.target_directory.trim() : "";
if (!dir) process.exit(1);
process.stdout.write(dir);
'
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

cleanup_artifact_dir() {
  local dir="$1"
  shopt -s dotglob nullglob
  rm -rf "${dir}"/*
  shopt -u dotglob nullglob
}

tcp_bind_available() {
  local bind="$1"
  node - "$bind" <<'NODE'
const net = require("node:net");
const bind = process.argv[2];
const separator = bind.lastIndexOf(":");
if (separator <= 0) process.exit(1);
const host = bind.slice(0, separator);
const port = Number(bind.slice(separator + 1));
const server = net.createServer();
server.once("error", () => process.exit(1));
server.listen({ host, port, exclusive: true }, () => {
  server.close(() => process.exit(0));
});
NODE
}

wait_for_daemon_health() {
  local target_url="$1"
  local output_path="$2"
  local ok=0
  for _ in $(seq 1 30); do
    if curl -fsS "$target_url" >"$output_path" 2>/dev/null; then
      ok=1
      break
    fi
    sleep 1
  done
  [[ "$ok" -eq 1 ]]
}

wait_for_file() {
  local target_path="$1"
  local ok=0
  for _ in $(seq 1 30); do
    if [[ -f "$target_path" ]]; then
      ok=1
      break
    fi
    sleep 1
  done
  [[ "$ok" -eq 1 ]]
}

wait_for_daemon_auth_token() {
  local target_path="$1"
  local token=""
  for _ in $(seq 1 30); do
    if [[ -f "$target_path" ]]; then
      token="$(json_get "$target_path" 'input.token' 2>/dev/null || true)"
      if [[ -n "$token" ]]; then
        printf '%s' "$token"
        return 0
      fi
    fi
    sleep 1
  done
  return 1
}

run_daemon_harness_container_smoke() {
  local phase="$1"
  local workspace_id="${2:-}"
  local daemon_auth_token="$3"
  local smoke_json="$4"
  local harness_container_json="$5"
  local terminal_json="$6"
  local terminal_output_log="$7"

  node --experimental-websocket - \
    "$ctx_daemon_loopback_base_url" \
    "$repo_dir" \
    "$phase" \
    "$workspace_id" \
    "$daemon_auth_token" \
    "$smoke_json" \
    "$harness_container_json" \
    "$terminal_json" \
    "$terminal_output_log" <<'NODE'
const fs = require("node:fs");
const { setTimeout: delay } = require("node:timers/promises");

const [
  ,
  ,
  baseUrl,
  repoDir,
  phase,
  requestedWorkspaceId,
  authToken,
  reportPath,
  harnessContainerPath,
  terminalPath,
  terminalOutputPath,
] = process.argv;

const report = {
  phase,
  base_url: baseUrl,
  repo_dir: repoDir,
  requested_workspace_id: requestedWorkspaceId || null,
  started_at: new Date().toISOString(),
};

const toWsUrl = (url) => {
  const value = new URL(url);
  value.protocol = value.protocol === "https:" ? "wss:" : "ws:";
  return value.toString();
};

const toText = async (value) => {
  if (typeof value === "string") return value;
  if (value instanceof ArrayBuffer) return Buffer.from(value).toString("utf8");
  if (ArrayBuffer.isView(value)) {
    return Buffer.from(value.buffer, value.byteOffset, value.byteLength).toString("utf8");
  }
  if (typeof Blob !== "undefined" && value instanceof Blob) {
    return Buffer.from(await value.arrayBuffer()).toString("utf8");
  }
  return String(value);
};

const request = async (method, pathname, body) => {
  const headers = {
    authorization: `Bearer ${authToken}`,
  };
  if (body) {
    headers["content-type"] = "application/json";
  }
  const response = await fetch(new URL(pathname, baseUrl), {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await response.text();
  let json = null;
  if (text.trim()) {
    try {
      json = JSON.parse(text);
    } catch {
      json = null;
    }
  }
  return {
    status: response.status,
    ok: response.ok,
    text,
    json,
  };
};

const deleteTerminal = async (terminalId) => {
  try {
    await request("DELETE", `/api/terminals/${terminalId}`);
  } catch {
    // Best-effort cleanup for CI diagnostics.
  }
};

const runTerminalSmoke = async (terminalId, expectedWorktreeRoot) => {
  const wsPath = new URL(`/api/terminals/${terminalId}/stream`, baseUrl);
  wsPath.searchParams.set("token", authToken);
  const wsUrl = toWsUrl(wsPath);
  const ws = new WebSocket(wsUrl);
  const sentinel = `__CTX_AVF_DAEMON_${phase.replace(/[^a-z0-9]+/gi, "_").toUpperCase()}__`;
  const command = [
    `printf '${sentinel}\\n'`,
    "pwd",
    "test -f hello.txt",
    "cat hello.txt",
    "exit",
  ].join("\n") + "\n";
  const outputChunks = [];
  const summary = {
    sentinel,
    saw_sentinel: false,
    saw_expected_worktree_root: false,
    saw_hello: false,
  };

  await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      try {
        ws.close();
      } catch {
        // Ignore close failures during timeout cleanup.
      }
      reject(new Error(`terminal smoke timed out for phase ${phase}`));
    }, 30000);
    let settled = false;

    const fail = (error) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeout);
      reject(error);
    };

    const succeed = () => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeout);
      try {
        ws.close();
      } catch {
        // Ignore close failures during success cleanup.
      }
      resolve();
    };

    ws.addEventListener("open", () => {
      ws.send(command);
    });

    ws.addEventListener("message", async (event) => {
      try {
        const text = await toText(event.data);
        outputChunks.push(text);
        if (text.includes(sentinel)) summary.saw_sentinel = true;
        if (text.includes(expectedWorktreeRoot)) summary.saw_expected_worktree_root = true;
        if (text.includes("hello")) summary.saw_hello = true;
        if (
          summary.saw_sentinel &&
          summary.saw_expected_worktree_root &&
          summary.saw_hello
        ) {
          succeed();
        }
      } catch (error) {
        fail(error);
      }
    });

    ws.addEventListener("close", () => {
      succeed();
    });

    ws.addEventListener("error", (event) => {
      const error = event?.error instanceof Error ? event.error : new Error(`terminal websocket failed for phase ${phase}`);
      fail(error);
    });
  });

  const output = outputChunks.join("");
  fs.writeFileSync(terminalOutputPath, output, "utf8");

  if (!summary.saw_sentinel || !summary.saw_expected_worktree_root || !summary.saw_hello) {
    throw new Error(
      `terminal smoke missing expected output for phase ${phase}: ${JSON.stringify(summary)}`,
    );
  }

  return {
    ws_url: wsUrl,
    summary,
    output_excerpt: output.slice(-4000),
  };
};

const main = async () => {
  let terminalId = "";
  try {
    let workspaceResp;
    if (requestedWorkspaceId && requestedWorkspaceId.trim()) {
      workspaceResp = await request("GET", `/api/workspaces/${requestedWorkspaceId.trim()}`);
    } else {
      workspaceResp = await request("POST", "/api/workspaces", {
        root_path: repoDir,
        name: `avf-linux-ci-${phase}`,
      });
    }
    report.workspace_response = {
      status: workspaceResp.status,
      body: workspaceResp.json ?? workspaceResp.text,
    };
    if (workspaceResp.status !== 200 || !workspaceResp.json || !workspaceResp.json.id) {
      throw new Error(`workspace request failed (${workspaceResp.status})`);
    }
    if (
      requestedWorkspaceId &&
      requestedWorkspaceId.trim() &&
      workspaceResp.json.id !== requestedWorkspaceId.trim()
    ) {
      throw new Error(`workspace id mismatch: ${JSON.stringify(workspaceResp.json.id)}`);
    }
    report.workspace = workspaceResp.json;

    const configResp = await request("POST", `/api/workspaces/${report.workspace.id}/execution_config`, {
      environment: "sandbox",
      network_mode: "all",
    });
    report.execution_config_response = {
      status: configResp.status,
      body: configResp.json ?? configResp.text,
    };
    if (configResp.status !== 200) {
      throw new Error(`execution config update failed (${configResp.status})`);
    }

    const ensureResp = await request("POST", `/api/workspaces/${report.workspace.id}/harness_container/ensure`);
    report.ensure_response = {
      status: ensureResp.status,
      body: ensureResp.text,
    };
    if (ensureResp.status !== 204) {
      throw new Error(`harness_container ensure failed (${ensureResp.status})`);
    }

    await delay(250);

    const harnessResp = await request("GET", `/api/workspaces/${report.workspace.id}/harness_container`);
    report.harness_container_response = {
      status: harnessResp.status,
      body: harnessResp.json ?? harnessResp.text,
    };
    if (harnessResp.status !== 200 || !harnessResp.json || harnessResp.json.running !== true) {
      throw new Error(`harness container status was not running after ensure (${harnessResp.status})`);
    }
    fs.writeFileSync(harnessContainerPath, `${JSON.stringify(harnessResp.json, null, 2)}\n`, "utf8");
    report.harness_container = harnessResp.json;

    const taskResp = await request("POST", `/api/workspaces/${report.workspace.id}/tasks`, {
      title: `avf-linux-ci-${phase}`,
    });
    report.task_response = {
      status: taskResp.status,
      body: taskResp.json ?? taskResp.text,
    };
    if (taskResp.status !== 200 || !taskResp.json || !taskResp.json.id) {
      throw new Error(`task creation failed (${taskResp.status})`);
    }
    if (!taskResp.json.primary_worktree_id) {
      throw new Error("task creation did not return a primary_worktree_id");
    }
    report.task = taskResp.json;

    const worktreeResp = await request("GET", `/api/worktrees/${taskResp.json.primary_worktree_id}`);
    report.worktree_response = {
      status: worktreeResp.status,
      body: worktreeResp.json ?? worktreeResp.text,
    };
    if (worktreeResp.status !== 200 || !worktreeResp.json || !worktreeResp.json.id) {
      throw new Error(`worktree fetch failed (${worktreeResp.status})`);
    }
    if (!String(worktreeResp.json.root_path || "").startsWith("/ctx/ws/worktrees/")) {
      throw new Error(`unexpected worktree root: ${JSON.stringify(worktreeResp.json.root_path)}`);
    }
    report.worktree = worktreeResp.json;

    const terminalResp = await request("POST", `/api/workspaces/${report.workspace.id}/terminals`, {
      task_id: taskResp.json.id,
      worktree_id: taskResp.json.primary_worktree_id,
      shell: "/bin/bash",
    });
    report.terminal_response = {
      status: terminalResp.status,
      body: terminalResp.json ?? terminalResp.text,
    };
    if (terminalResp.status !== 200 || !terminalResp.json || !terminalResp.json.id) {
      throw new Error(`workspace terminal creation failed (${terminalResp.status})`);
    }
    terminalId = terminalResp.json.id;
    fs.writeFileSync(terminalPath, `${JSON.stringify(terminalResp.json, null, 2)}\n`, "utf8");
    if (!String(terminalResp.json.cwd || "").startsWith("/ctx/ws/worktrees/")) {
      throw new Error(`unexpected terminal cwd: ${JSON.stringify(terminalResp.json.cwd)}`);
    }
    if (terminalResp.json.cwd !== worktreeResp.json.root_path) {
      throw new Error(`terminal cwd mismatch: ${JSON.stringify(terminalResp.json.cwd)}`);
    }
    if (terminalResp.json.task_id !== taskResp.json.id) {
      throw new Error(`terminal task mismatch: ${JSON.stringify(terminalResp.json.task_id)}`);
    }
    if (terminalResp.json.worktree_id !== taskResp.json.primary_worktree_id) {
      throw new Error(`terminal worktree mismatch: ${JSON.stringify(terminalResp.json.worktree_id)}`);
    }
    report.terminal = terminalResp.json;
    report.terminal_smoke = await runTerminalSmoke(terminalId, worktreeResp.json.root_path);
    report.ok = true;
  } catch (error) {
    report.ok = false;
    report.error = error instanceof Error ? error.message : String(error);
    throw error;
  } finally {
    if (terminalId) {
      await deleteTerminal(terminalId);
    }
    report.finished_at = new Date().toISOString();
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
  }
};

main().catch((error) => {
  console.error(error instanceof Error ? error.stack || error.message : String(error));
  process.exit(1);
});
NODE
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
    --restore-smoke)
      restore_smoke_mode="${2:-}"
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
cleanup_artifact_dir "$artifact_dir"

case "$real_exec_mode" in
  best-effort|required|skip) ;;
  *) die "unsupported --real-exec mode: $real_exec_mode" ;;
esac

case "$restore_smoke_mode" in
  required|skip) ;;
  *) die "unsupported --restore-smoke mode: $restore_smoke_mode" ;;
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
need_cmd curl

export CTX_SESSION_ID="${CTX_SESSION_ID:-avf-linux-ci-smoke-${runtime_arch:-auto}-${$}}"
eval "$(node "${repo_root}/scripts/print_ctx_cache_env.cjs" --mode workspace --cwd "${repo_root}" --format shell --mkdir)"

helper_manifest="${repo_root}/apps/desktop/src-tauri/Cargo.toml"
helper_entitlements="${repo_root}/apps/desktop/src-tauri/ctx-avf-linux-helper.entitlements"
helper_target_root="${CARGO_TARGET_DIR:-$(cargo_target_dir "$helper_manifest")}"
guest_target_root="${CARGO_TARGET_DIR:-$(cargo_target_dir "${repo_root}/Cargo.toml")}"
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
daemon_health_json="${artifact_dir}/ctx-daemon.health.json"
daemon_health_stdout="${artifact_dir}/daemon-health.stdout.log"
daemon_health_stderr="${artifact_dir}/daemon-health.stderr.log"
daemon_health_restricted_stdout="${artifact_dir}/daemon-health.restricted.stdout.log"
daemon_health_restricted_stderr="${artifact_dir}/daemon-health.restricted.stderr.log"
restricted_block_stdout="${artifact_dir}/restricted-block.stdout.log"
restricted_block_stderr="${artifact_dir}/restricted-block.stderr.log"
daemon_smoke_json="${artifact_dir}/daemon-harness-container-smoke.json"
daemon_harness_container_json="${artifact_dir}/daemon-harness-container.json"
daemon_terminal_json="${artifact_dir}/daemon-terminal.json"
daemon_terminal_output="${artifact_dir}/daemon-terminal.output.log"
daemon_smoke_restore_json="${artifact_dir}/daemon-harness-container.restore-smoke.json"
daemon_harness_container_restore_json="${artifact_dir}/daemon-harness-container.restore.json"
daemon_terminal_restore_json="${artifact_dir}/daemon-terminal.restore.json"
daemon_terminal_restore_output="${artifact_dir}/daemon-terminal.restore.output.log"
report_json="${artifact_dir}/report.json"
staged_runtime_dir=""
staged_rootfs_path=""
staged_kernel_path=""
staged_initrd_path=""

toolchain="${CTX_AVF_GUEST_AGENT_TOOLCHAIN:-$(toolchain_for_host_arch)}"
guest_python_fetch_code='import sys, urllib.request; print(urllib.request.urlopen(sys.argv[1], timeout=10).read().decode(), end="")'
ctx_daemon_port=43991
ctx_daemon_loopback_base_url="http://127.0.0.1:${ctx_daemon_port}"
ctx_daemon_loopback_bind="127.0.0.1:${ctx_daemon_port}"
ctx_daemon_gateway_bind=""
ctx_daemon_gateway_url="http://192.168.64.1:${ctx_daemon_port}/api/health"
ctx_daemon_loopback_url="${ctx_daemon_loopback_base_url}/api/health"
daemon_auth_file="${data_root}/daemon_auth.json"
daemon_pid=""
ctx_daemon_gateway_required=0

if tcp_bind_available "192.168.64.1:${ctx_daemon_port}"; then
  ctx_daemon_gateway_bind="192.168.64.1:${ctx_daemon_port}"
  ctx_daemon_gateway_required=1
else
  echo "==> AVF guest gateway bind 192.168.64.1:${ctx_daemon_port} unavailable on this host; skipping direct guest daemon reachability probes"
fi

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
CTX_DESKTOP_SKIP_TAURI_BUILD=1 node "${repo_root}/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${repo_root}" -- cargo build --manifest-path "$helper_manifest" --bin ctx-avf-linux-helper
/usr/bin/codesign --force --sign - --entitlements "$helper_entitlements" "$helper_bin"

echo "==> Probing helper"
"$helper_bin" probe | tee "$probe_json"

echo "==> Building ctx daemon"
node "${repo_root}/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${repo_root}" -- cargo build --locked --manifest-path "${repo_root}/Cargo.toml" -p ctx-http --bin ctx
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
staged_runtime_dir="$(json_get "$staging_json" 'input.runtimeRootDir')"
staged_rootfs_path="$(json_get "$staging_json" 'input.rootfsPath')"
staged_kernel_path="$(json_get "$staging_json" 'input.kernelPath')"
staged_initrd_path="$(json_get "$staging_json" 'input.initrdPath')"
[[ -n "$staged_runtime_dir" ]] || die "staging report missing runtimeRootDir"
[[ -d "$staged_runtime_dir" ]] || die "staged runtime root missing at ${staged_runtime_dir}"
[[ -f "$staged_rootfs_path" ]] || die "staged rootfs missing at ${staged_rootfs_path}"
[[ -f "$staged_kernel_path" ]] || die "staged kernel missing at ${staged_kernel_path}"
[[ -f "$staged_initrd_path" ]] || die "staged initrd missing at ${staged_initrd_path}"

echo "==> Copying canonical runtime lock into staged bundle"
cp "${repo_root}/apps/desktop/src-tauri/bundles/runtime_lock.v2.json" "${bundle_dir}/runtime_lock.v2.json"

echo "==> Helper lifecycle smoke"
"$helper_bin" prepare-runtime-layout "$data_root" | tee "$layout_json"
"$helper_bin" workspace-vm-state "$data_root" | tee "$initial_state_json"

real_exec_status="skipped"
real_exec_reason="real guest-exec smoke disabled"
restore_status="skipped"
restore_reason="workspace VM restore smoke disabled"
harness_container_status="skipped"
harness_container_reason="daemon harness_container/ensure smoke disabled"
daemon_probe_status="skipped"
daemon_probe_reason="daemon reachability smoke disabled"
restricted_network_status="skipped"
restricted_network_reason="restricted-network smoke disabled"

if [[ "$real_exec_mode" != "skip" ]]; then
  real_exec_status="failed"
  real_exec_reason="real guest-exec smoke did not complete"
  restore_status="failed"
  restore_reason="workspace VM restore smoke did not complete"
  harness_container_status="failed"
  harness_container_reason="daemon harness_container/ensure smoke did not complete"
  daemon_probe_status="failed"
  daemon_probe_reason="daemon reachability smoke did not complete"
  restricted_network_status="failed"
  restricted_network_reason="restricted-network smoke did not complete"
  if [[ "$ctx_daemon_gateway_required" -ne 1 ]]; then
    daemon_probe_status="skipped"
    daemon_probe_reason="AVF guest gateway bind unavailable on host; skipping direct guest daemon reachability smoke"
    restricted_network_status="skipped"
    restricted_network_reason="AVF guest gateway bind unavailable on host; skipping restricted-network guest daemon reachability smoke"
  fi

  rm -rf "$repo_dir"
  mkdir -p "$repo_dir"
  git -C "$repo_dir" init -q
  git -C "$repo_dir" config user.email ci@example.invalid
  git -C "$repo_dir" config user.name "ctx AVF CI"
  printf 'hello\n' > "${repo_dir}/hello.txt"
  git -C "$repo_dir" add hello.txt
  git -C "$repo_dir" commit -q -m "initial"
  base_commit="$(git -C "$repo_dir" rev-parse HEAD)"

  echo "==> Starting ctx daemon with shared data_root/bundle_dir"
  set +e
  daemon_bind_args=(--bind "$ctx_daemon_loopback_bind")
  if [[ -n "$ctx_daemon_gateway_bind" ]]; then
    daemon_bind_args+=(--bind "$ctx_daemon_gateway_bind")
  fi
  CTX_AVF_LINUX_HELPER_PATH="$helper_bin" \
  CTX_BUNDLE_DIR="$bundle_dir" \
  CTX_BUNDLE_MANIFEST="$bundle_dir/manifest.json" \
  CTX_RUNTIME_PROFILE=parity \
    "$ctx_bin" serve \
      --data-dir "$data_root" \
      "${daemon_bind_args[@]}" \
      >"$daemon_stdout" 2>"$daemon_stderr" &
  daemon_pid=$!
  set -e

  if wait_for_daemon_health "$ctx_daemon_loopback_url" "$daemon_health_json"; then
    if daemon_auth_token="$(wait_for_daemon_auth_token "$daemon_auth_file")"; then
      echo "==> Exercising daemon /harness_container/ensure in the real AVF VM"
      if run_daemon_harness_container_smoke \
        "initial" \
        "" \
        "$daemon_auth_token" \
        "$daemon_smoke_json" \
        "$daemon_harness_container_json" \
        "$daemon_terminal_json" \
        "$daemon_terminal_output"
      then
        harness_container_status="passed"
        harness_container_reason="daemon /harness_container/ensure created a running harness container and task/worktree terminal smoke succeeded"

        workspace_id="$(json_get "$daemon_smoke_json" 'input.workspace.id')"
        task_id="$(json_get "$daemon_smoke_json" 'input.task.id')"
        worktree_id="$(json_get "$daemon_smoke_json" 'input.task.primary_worktree_id')"
        harness_container_name="$(json_get "$daemon_smoke_json" 'input.harness_container.name')"
        branch_name="ctx/${task_id}/${worktree_id}"
        guest_root="/ctx/ws/worktrees/${worktree_id}"
        "$helper_bin" workspace-vm-state "$data_root" | tee "$start_json"

        prepare_ok=0
        for _ in $(seq 1 60); do
          if "$helper_bin" prepare-guest-worktree \
            "$data_root" \
            "$workspace_id" \
            "$worktree_id" \
            "$repo_dir" \
            "$base_commit" \
            "$branch_name" >"$prepared_json" 2>"${artifact_dir}/prepare-guest-worktree.stderr.log"
          then
            prepare_ok=1
            break
          fi
          sleep 2
        done

        if [[ "$prepare_ok" -eq 1 ]]; then
        if [[ "$ctx_daemon_gateway_required" -eq 1 ]]; then
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
          daemon_probe_reason="AVF guest gateway bind unavailable on host; skipping direct guest daemon reachability smoke"
          restricted_network_reason="AVF guest gateway bind unavailable on host; skipping restricted-network guest daemon reachability smoke"
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
            printf '' | "$helper_bin" guest-exec \
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
            if [[ "$restore_smoke_mode" == "skip" ]]; then
              real_exec_status="passed"
              real_exec_reason="daemon harness_container/ensure and both guest-exec smoke probes succeeded"
              restore_status="skipped"
              restore_reason="workspace VM save/restore smoke disabled by --restore-smoke skip"
            else
              set +e
              "$helper_bin" stop-workspace-vm "$data_root" >"$stop1_json"
              stop1_status=$?
              set -e

              if [[ "$stop1_status" -eq 0 ]]; then
                echo "==> Re-exercising daemon /harness_container/ensure after stop/save"
                if run_daemon_harness_container_smoke \
                  "restore" \
                  "$workspace_id" \
                  "$daemon_auth_token" \
                  "$daemon_smoke_restore_json" \
                  "$daemon_harness_container_restore_json" \
                  "$daemon_terminal_restore_json" \
                  "$daemon_terminal_restore_output"
                then
                  restore_harness_container_name="$(json_get "$daemon_smoke_restore_json" 'input.harness_container.name')"
                  if [[ -z "$harness_container_name" || -z "$restore_harness_container_name" ]]; then
                    harness_container_status="failed"
                    harness_container_reason="daemon harness_container/ensure smoke did not report stable container names across restore"
                    restore_reason="$harness_container_reason"
                    real_exec_reason="workspace VM restore smoke could not confirm harness container identity"
                  elif [[ "$restore_harness_container_name" != "$harness_container_name" ]]; then
                    harness_container_status="failed"
                    harness_container_reason="daemon harness_container/ensure recreated the AVF harness container across restore"
                    restore_reason="$harness_container_reason"
                    real_exec_reason="workspace VM restore smoke failed because harness container identity changed"
                  else
                    restore_task_id="$(json_get "$daemon_smoke_restore_json" 'input.task.id')"
                    restore_worktree_id="$(json_get "$daemon_smoke_restore_json" 'input.task.primary_worktree_id')"
                    restore_branch_name="ctx/${restore_task_id}/${restore_worktree_id}"
                    restore_guest_root="/ctx/ws/worktrees/${restore_worktree_id}"
                    "$helper_bin" workspace-vm-state "$data_root" | tee "$start2_json"

                    prepare2_ok=0
                    for _ in $(seq 1 60); do
                      if "$helper_bin" prepare-guest-worktree \
                        "$data_root" \
                        "$workspace_id" \
                        "$restore_worktree_id" \
                        "$repo_dir" \
                        "$base_commit" \
                        "$restore_branch_name" >"$prepared2_json" 2>"${artifact_dir}/prepare-guest-worktree.restore.stderr.log"
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
                        --worktree-id "$restore_worktree_id" \
                        --cwd "$restore_guest_root" \
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
                          restore_start_outcome="$(json_get "$start2_json" 'input.last_start_outcome' 2>/dev/null || true)"
                          if [[ "$restore_start_outcome" == "restored" ]]; then
                            restore_status="passed"
                            restore_reason="workspace VM resumed from saved state after the second daemon harness_container smoke and second guest exec succeeded"
                            harness_container_reason="daemon /harness_container/ensure and task/worktree terminal smoke passed before and after restore"
                            real_exec_status="passed"
                            real_exec_reason="daemon harness_container/ensure and guest-exec smoke passed before and after restore"
                          elif [[ "$(host_arch)" == "arm64" ]]; then
                            restore_status="failed"
                            restore_reason="second daemon harness_container smoke completed but did not restore saved state on an Apple silicon host"
                            harness_container_status="failed"
                            harness_container_reason="second daemon harness_container smoke completed but did not restore saved state on an Apple silicon host"
                            real_exec_reason="workspace VM second daemon harness_container smoke skipped save/restore on an Apple silicon host"
                          else
                            restore_status="passed"
                            restore_reason="workspace VM restarted after the second daemon harness_container smoke and second guest exec succeeded without a saved-state restore"
                            harness_container_reason="daemon /harness_container/ensure and task/worktree terminal smoke passed before and after restart"
                            real_exec_status="passed"
                            real_exec_reason="daemon harness_container/ensure and guest-exec smoke passed before and after restart"
                          fi
                        else
                          restore_reason="second workspace-vm stop failed with status ${stop2_status}"
                          real_exec_reason="workspace VM restore smoke failed after the second guest exec"
                        fi
                      else
                        restore_reason="second non-PTY guest-exec failed with status ${nonpty2_status}"
                        real_exec_reason="workspace VM restore smoke failed after the second daemon harness_container smoke"
                      fi
                    else
                      restore_reason="prepare-guest-worktree after the second daemon harness_container smoke did not become ready within timeout"
                      real_exec_reason="workspace VM restore smoke failed after the second daemon harness_container smoke"
                    fi
                  fi
                else
                  harness_container_status="failed"
                  harness_container_reason="second daemon harness_container smoke failed"
                  restore_reason="second daemon harness_container smoke failed"
                  real_exec_reason="workspace VM restore smoke failed before the second guest exec"
                fi
              else
                restore_reason="first workspace-vm stop failed with status ${stop1_status}"
                real_exec_reason="workspace VM stop/save failed after the first guest exec"
              fi
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
      harness_container_reason="daemon harness_container/ensure smoke failed; see daemon-harness-container-smoke.json"
      real_exec_reason="daemon harness_container/ensure smoke failed before guest-exec coverage"
      daemon_probe_reason="daemon harness_container/ensure smoke failed before guest reachability checks"
      restricted_network_reason="daemon harness_container/ensure smoke failed before restricted-network checks"
    fi
    else
      harness_container_reason="daemon auth token never became readable"
      real_exec_reason="daemon auth token never became readable"
      daemon_probe_reason="daemon auth token never became readable"
      restricted_network_reason="daemon auth token never became readable"
    fi
  else
    harness_container_reason="ctx daemon never became healthy on host loopback"
    real_exec_reason="ctx daemon never became healthy on host loopback"
    daemon_probe_reason="ctx daemon never became healthy on host loopback"
    restricted_network_reason="ctx daemon never became healthy on host loopback"
  fi

  if [[ "$real_exec_mode" == "required" ]]; then
    if [[ "$harness_container_status" != "passed" ]]; then
      echo "error: ${harness_container_reason}" >&2
    elif [[ "$ctx_daemon_gateway_required" -eq 1 && "$daemon_probe_status" != "passed" ]]; then
      echo "error: ${daemon_probe_reason}" >&2
    elif [[ "$ctx_daemon_gateway_required" -eq 1 && "$restricted_network_status" != "passed" ]]; then
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
  "$staged_runtime_dir" \
  "$artifact_dir" \
  "$real_exec_status" \
  "$real_exec_reason" \
  "$restore_status" \
  "$restore_reason" \
  "$harness_container_status" \
  "$harness_container_reason" \
  "$daemon_probe_status" \
  "$daemon_probe_reason" \
  "$restricted_network_status" \
  "$restricted_network_reason" \
  "$ctx_daemon_gateway_required" \
  "$real_exec_mode" \
  "$runtime_arch" \
  "$daemon_smoke_json" \
  "$daemon_smoke_restore_json" <<'NODE'
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
  stagedRuntimeDir,
  artifactDir,
  realExecStatus,
  realExecReason,
  restoreStatus,
  restoreReason,
  harnessContainerStatus,
  harnessContainerReason,
  daemonProbeStatus,
  daemonProbeReason,
  restrictedNetworkStatus,
  restrictedNetworkReason,
  guestGatewayRequired,
  realExecMode,
  runtimeArch,
  daemonSmokePath,
  daemonSmokeRestorePath,
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
  avf_guest_gateway_required: guestGatewayRequired === "1",
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
  staged_runtime_dir: stagedRuntimeDir,
  artifact_dir: artifactDir,
  daemon_harness_container: maybeJson(daemonSmokePath),
  daemon_harness_container_restore: maybeJson(daemonSmokeRestorePath),
  real_exec: {
    status: realExecStatus,
    reason: realExecReason,
  },
  restore_exec: {
    status: restoreStatus,
    reason: restoreReason,
  },
  harness_container: {
    status: harnessContainerStatus,
    reason: harnessContainerReason,
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

if [[ "$real_exec_mode" == "required" && ( "$harness_container_status" != "passed" || "$real_exec_status" != "passed" || ( "$ctx_daemon_gateway_required" -eq 1 && ( "$daemon_probe_status" != "passed" || "$restricted_network_status" != "passed" ) ) ) ]]; then
  exit 1
fi
