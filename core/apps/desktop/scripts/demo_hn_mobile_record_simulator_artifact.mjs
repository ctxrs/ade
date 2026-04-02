#!/usr/bin/env node
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

import { runChecked } from "./demo_hn_mobile_artifact_lib.mjs";

const DEFAULT_REMOTE_HOST = "";
const DEFAULT_REMOTE_USER = "admin";
const DEFAULT_REMOTE_WORKSPACE_ROOT = "/Users/example-user/hn-mobile-baseline";
const DEFAULT_REMOTE_DEVICE_NAME = "iPhone 16";
const DEFAULT_REMOTE_DEVICE_TYPE = "com.apple.CoreSimulator.SimDeviceType.iPhone-16";
const DEFAULT_REMOTE_RUNTIME = "com.apple.CoreSimulator.SimRuntime.iOS-26-2";
const DEFAULT_REMOTE_BUNDLE_ID = "rs.ctx.hnmobile";
const DEFAULT_REMOTE_CONFIG_PATH = "src-tauri/tauri.ios.recording.conf.json";
const DEFAULT_REMOTE_PORT = 0;
const DEFAULT_APPEARANCE = "light";
const DEFAULT_PREWARM_DELAY_MS = 6000;
const DEFAULT_HOME_DWELL_MS = 1000;
const DEFAULT_RELAUNCH_DELAY_MS = 800;
const DEFAULT_RECORDING_DURATION_MS = 12_000;
const DEFAULT_READY_TIMEOUT_MS = 12 * 60_000;

function runStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace("T", "-").replace("Z", "");
}

function shQuote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`;
}

function sleep(delayMs) {
  return new Promise((resolve) => {
    setTimeout(resolve, delayMs);
  });
}

function parseArgs(argv) {
  const stamp = runStamp();
  const options = {
    appearance: DEFAULT_APPEARANCE,
    force: false,
    homeDwellMs: DEFAULT_HOME_DWELL_MS,
    keepRemoteSession: false,
    outputPath: "",
    prewarmDelayMs: DEFAULT_PREWARM_DELAY_MS,
    readyTimeoutMs: DEFAULT_READY_TIMEOUT_MS,
    relaunchDelayMs: DEFAULT_RELAUNCH_DELAY_MS,
    recordingDurationMs: DEFAULT_RECORDING_DURATION_MS,
    remoteBundleId: DEFAULT_REMOTE_BUNDLE_ID,
    remoteConfigPath: DEFAULT_REMOTE_CONFIG_PATH,
    remoteDeviceName: DEFAULT_REMOTE_DEVICE_NAME,
    remoteDeviceType: DEFAULT_REMOTE_DEVICE_TYPE,
    remoteHost: DEFAULT_REMOTE_HOST,
    remotePort: DEFAULT_REMOTE_PORT,
    remoteRuntime: DEFAULT_REMOTE_RUNTIME,
    remoteUser: DEFAULT_REMOTE_USER,
    remoteWorkspaceRoot: DEFAULT_REMOTE_WORKSPACE_ROOT,
    resetRemoteDevice: false,
    workspaceRoot: "",
    sessionName: `ctx-hn-mobile-record-${stamp}`.replace(/[^a-zA-Z0-9_-]/g, "-"),
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--workspace-root") {
      options.workspaceRoot = path.resolve(next);
      index += 1;
    } else if (arg === "--output") {
      options.outputPath = path.resolve(next);
      index += 1;
    } else if (arg === "--remote-host") {
      options.remoteHost = next;
      index += 1;
    } else if (arg === "--remote-user") {
      options.remoteUser = next;
      index += 1;
    } else if (arg === "--remote-workspace-root") {
      options.remoteWorkspaceRoot = next;
      index += 1;
    } else if (arg === "--remote-device") {
      options.remoteDeviceName = next;
      index += 1;
    } else if (arg === "--remote-device-type") {
      options.remoteDeviceType = next;
      index += 1;
    } else if (arg === "--remote-runtime") {
      options.remoteRuntime = next;
      index += 1;
    } else if (arg === "--remote-bundle-id") {
      options.remoteBundleId = next;
      index += 1;
    } else if (arg === "--remote-config") {
      options.remoteConfigPath = next;
      index += 1;
    } else if (arg === "--remote-port") {
      options.remotePort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--appearance") {
      options.appearance = next;
      index += 1;
    } else if (arg === "--prewarm-delay-ms") {
      options.prewarmDelayMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--home-dwell-ms") {
      options.homeDwellMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--relaunch-delay-ms") {
      options.relaunchDelayMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--recording-duration-ms") {
      options.recordingDurationMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--ready-timeout-ms") {
      options.readyTimeoutMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--session-name") {
      options.sessionName = next;
      index += 1;
    } else if (arg === "--keep-remote-session") {
      options.keepRemoteSession = true;
    } else if (arg === "--reset-remote-device") {
      options.resetRemoteDevice = true;
    } else if (arg === "--force") {
      options.force = true;
    }
  }

  if (!options.workspaceRoot) {
    throw new Error("--workspace-root is required");
  }
  if (!options.outputPath) {
    throw new Error("--output is required");
  }
  if (!options.remoteHost) {
    throw new Error("--remote-host is required");
  }
  if (!["light", "dark"].includes(options.appearance)) {
    throw new Error("--appearance must be light or dark");
  }
  if (!Number.isFinite(options.prewarmDelayMs) || options.prewarmDelayMs < 0) {
    throw new Error("--prewarm-delay-ms must be a non-negative integer");
  }
  if (!Number.isFinite(options.homeDwellMs) || options.homeDwellMs < 0) {
    throw new Error("--home-dwell-ms must be a non-negative integer");
  }
  if (!Number.isFinite(options.relaunchDelayMs) || options.relaunchDelayMs < 0) {
    throw new Error("--relaunch-delay-ms must be a non-negative integer");
  }
  if (!Number.isFinite(options.recordingDurationMs) || options.recordingDurationMs <= 0) {
    throw new Error("--recording-duration-ms must be a positive integer");
  }

  return options;
}

function runCommandCapture(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    const stderr = String(result.stderr ?? "").trim();
    const stdout = String(result.stdout ?? "").trim();
    const details = [stderr, stdout].filter(Boolean).join("\n");
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}${details ? `\n${details}` : ""}`);
  }
  return String(result.stdout ?? "");
}

function buildSshArgs(target, script) {
  return ["-o", "BatchMode=yes", target, script];
}

function sshCapture(target, script, options = {}) {
  return runCommandCapture("ssh", buildSshArgs(target, script), options);
}

function sshRun(target, script, options = {}) {
  const result = spawnSync("ssh", buildSshArgs(target, script), {
    stdio: "inherit",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`ssh ${target} failed with status ${result.status}`);
  }
}

function buildRsyncArgs(options) {
  return [
    "-az",
    "--delete",
    "--exclude", ".git",
    "--exclude", "node_modules",
    "--exclude", "dist",
    "--exclude", "src-tauri/target",
    "--exclude", ".DS_Store",
    "-e", "ssh -o BatchMode=yes",
    `${options.workspaceRoot}/`,
    `${options.remoteUser}@${options.remoteHost}:${options.remoteWorkspaceRoot}/`,
  ];
}

function resolveRemoteGeneratedConfigPath(options) {
  return `/tmp/${options.sessionName}.tauri.ios.recording.conf.json`;
}

function resolveRemoteServerLogPath(options) {
  return `${options.remoteWorkspaceRoot}/.demo-hn-mobile-record-server.log`;
}

function resolveRemoteBuildLogPath(options) {
  return `${options.remoteWorkspaceRoot}/.demo-hn-mobile-record-build.log`;
}

function resolveRemoteVideoPath(options) {
  return `/tmp/${options.sessionName}.mov`;
}

function resolveRemoteRecordLogPath(options) {
  return `/tmp/${options.sessionName}.record.log`;
}

function resolveRemoteServerPidPath(options) {
  return `${options.remoteWorkspaceRoot}/.demo-hn-mobile-record-server.pid`;
}

function resolveRemoteBuildPidPath(options) {
  return `${options.remoteWorkspaceRoot}/.demo-hn-mobile-record-build.pid`;
}

function buildRemoteRecordingConfigNodeScript(options) {
  const generatedConfigPath = resolveRemoteGeneratedConfigPath(options);
  return [
    "node - <<'NODE'",
    "const fs = require('node:fs');",
    `const generatedPath = ${JSON.stringify(generatedConfigPath)};`,
    `const port = ${Number(options.remotePort)};`,
    `const config = JSON.parse(fs.readFileSync(${JSON.stringify(options.remoteConfigPath)}, 'utf8'));`,
    "config.build = {",
    "  ...(config.build || {}),",
    "  devUrl: `http://127.0.0.1:${port}`,",
    "  beforeDevCommand: undefined,",
    "};",
    "config.app.windows = (config.app.windows || []).map((windowConfig) => ({",
    "  ...windowConfig,",
    "  url: String(windowConfig.url || '').replace('127.0.0.1:1420', `127.0.0.1:${port}`),",
    "}));",
    "fs.writeFileSync(generatedPath, JSON.stringify(config, null, 2));",
    "NODE",
  ].join("\n");
}

export function buildRemotePrepareDeviceCommand(options) {
  const deviceName = shQuote(options.remoteDeviceName);
  const deviceType = shQuote(options.remoteDeviceType);
  const runtime = shQuote(options.remoteRuntime);
  const resetFlag = options.resetRemoteDevice ? "1" : "0";
  return [
    "set -euo pipefail",
    "export PATH=$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH",
    `export CTX_HN_REMOTE_DEVICE_NAME=${deviceName}`,
    `export CTX_HN_REMOTE_DEVICE_TYPE=${deviceType}`,
    `export CTX_HN_REMOTE_RUNTIME=${runtime}`,
    `export CTX_HN_RESET_REMOTE_DEVICE=${shQuote(resetFlag)}`,
    [
      "python3 - <<'PY'",
      "import json",
      "import os",
      "import subprocess",
      "",
      "device_name = os.environ['CTX_HN_REMOTE_DEVICE_NAME']",
      "device_type = os.environ['CTX_HN_REMOTE_DEVICE_TYPE']",
      "runtime = os.environ['CTX_HN_REMOTE_RUNTIME']",
      "reset_device = os.environ['CTX_HN_RESET_REMOTE_DEVICE'] == '1'",
      "",
      "raw_devices = subprocess.check_output(['xcrun', 'simctl', 'list', 'devices', 'available', '-j'], text=True)",
      "devices = json.loads(raw_devices).get('devices', {})",
      "existing_udid = ''",
      "for runtime_devices in devices.values():",
      "    for device in runtime_devices:",
      "        if device.get('name') == device_name and device.get('isAvailable', True):",
      "            existing_udid = device['udid']",
      "            break",
      "    if existing_udid:",
      "        break",
      "",
      "if existing_udid and reset_device:",
      "    subprocess.run(['xcrun', 'simctl', 'shutdown', existing_udid], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)",
      "    subprocess.run(['xcrun', 'simctl', 'erase', existing_udid], check=True)",
      "",
      "if not existing_udid:",
      "    existing_udid = subprocess.check_output(['xcrun', 'simctl', 'create', device_name, device_type, runtime], text=True).strip()",
      "",
      "subprocess.run(['xcrun', 'simctl', 'boot', existing_udid], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)",
      "subprocess.run(['xcrun', 'simctl', 'bootstatus', existing_udid, '-b'], check=True)",
      "print(existing_udid)",
      "PY",
    ].join("\n"),
  ].join("\n");
}

export function buildRemoteBootstrapCommand(options) {
  const generatedConfigPath = resolveRemoteGeneratedConfigPath(options);
  const serverLogPath = resolveRemoteServerLogPath(options);
  const buildLogPath = resolveRemoteBuildLogPath(options);
  const serverPidPath = resolveRemoteServerPidPath(options);
  const buildPidPath = resolveRemoteBuildPidPath(options);
  return [
    "set -euo pipefail",
    `mkdir -p ${shQuote(options.remoteWorkspaceRoot)}`,
    `cd ${shQuote(options.remoteWorkspaceRoot)}`,
    "export PATH=/Users/example-user/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH",
    `if [ -f ${shQuote(serverPidPath)} ]; then kill $(cat ${shQuote(serverPidPath)}) >/dev/null 2>&1 || true; fi`,
    `if [ -f ${shQuote(buildPidPath)} ]; then kill $(cat ${shQuote(buildPidPath)}) >/dev/null 2>&1 || true; fi`,
    `rm -f ${shQuote(serverLogPath)} ${shQuote(buildLogPath)} ${shQuote(serverPidPath)} ${shQuote(buildPidPath)}`,
    "pnpm install --frozen-lockfile",
    "find $HOME/Library/Developer/Xcode/DerivedData -maxdepth 1 -name 'hn_mobile-*' -exec rm -rf {} + 2>/dev/null || true",
    "if ! find src-tauri/gen/apple -maxdepth 1 -name '*.xcodeproj' | grep -q .; then cargo tauri ios init --ci >/dev/null 2>&1; fi",
    buildRemoteRecordingConfigNodeScript(options),
    `nohup /bin/zsh -lc ${shQuote(
      [
        `cd ${shQuote(options.remoteWorkspaceRoot)}`,
        "export PATH=/Users/example-user/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH",
        `pnpm dev --host 127.0.0.1 --port ${Number(options.remotePort)}`,
      ].join(" && "),
    )} > ${shQuote(serverLogPath)} 2>&1 </dev/null & echo $! > ${shQuote(serverPidPath)}`,
    `nohup /bin/zsh -lc ${shQuote(
      [
        `cd ${shQuote(options.remoteWorkspaceRoot)}`,
        "export PATH=/Users/example-user/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH",
        "export CI=1",
        "export TOOLCHAINS=com.apple.dt.toolchain.XcodeDefault",
        `cargo tauri ios dev --no-watch -c ${shQuote(generatedConfigPath)} ${shQuote(options.remoteDeviceName)}`,
      ].join(" && "),
    )} > ${shQuote(buildLogPath)} 2>&1 </dev/null & echo $! > ${shQuote(buildPidPath)}`,
    `printf '%s\n%s' ${shQuote(serverLogPath)} ${shQuote(buildLogPath)}`,
  ].join("\n");
}

function buildRemoteReadyCheckCommand(options) {
  const buildLogPath = resolveRemoteBuildLogPath(options);
  return [
    "set -euo pipefail",
    `cd ${shQuote(options.remoteWorkspaceRoot)}`,
    "export PATH=/Users/example-user/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH",
    `curl -fsS ${shQuote(`http://127.0.0.1:${options.remotePort}/?ctxDemoScenario=muted-domains`)} >/dev/null`,
    `test -f ${shQuote(buildLogPath)}`,
    `grep -F '** BUILD SUCCEEDED **' ${shQuote(buildLogPath)} >/dev/null`,
    "APP=$(find $HOME/Library/Developer/Xcode/DerivedData -path '*Build/Products/debug-iphonesimulator/HN Mobile.app' | head -n 1)",
    "test -n \"$APP\"",
    "test -f \"$APP/HN Mobile.debug.dylib\"",
    `strings "$APP/HN Mobile.debug.dylib" | grep -F ${shQuote(`http://127.0.0.1:${options.remotePort}/?ctxDemoScenario=muted-domains&demoAutoplay=muted-domains&demoAutoplayPreset=recording`)} >/dev/null`,
  ].join("\n");
}

function buildRemoteLogTailCommand(options) {
  const serverLogPath = resolveRemoteServerLogPath(options);
  const buildLogPath = resolveRemoteBuildLogPath(options);
  return [
    "set +e",
    `printf '%s\\n' '===== remote server log ====='`,
    `tail -80 ${shQuote(serverLogPath)} 2>/dev/null || true`,
    `printf '%s\\n' '===== remote build log ====='`,
    `tail -120 ${shQuote(buildLogPath)} 2>/dev/null || true`,
  ].join("\n");
}

export function buildRemoteCleanupCommand(options) {
  const serverPidPath = resolveRemoteServerPidPath(options);
  const buildPidPath = resolveRemoteBuildPidPath(options);
  return [
    "set -euo pipefail",
    `if [ -f ${shQuote(serverPidPath)} ]; then kill $(cat ${shQuote(serverPidPath)}) >/dev/null 2>&1 || true; fi`,
    `if [ -f ${shQuote(buildPidPath)} ]; then kill $(cat ${shQuote(buildPidPath)}) >/dev/null 2>&1 || true; fi`,
    `rm -f ${shQuote(serverPidPath)} ${shQuote(buildPidPath)}`,
  ].join("\n");
}

export function buildRemoteRecordCommand(options) {
  const remoteVideoPath = resolveRemoteVideoPath(options);
  const remoteRecordLogPath = resolveRemoteRecordLogPath(options);
  const deviceTarget = shQuote(options.remoteDeviceName);
  return [
    "set -euo pipefail",
    "export PATH=/Users/example-user/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$PATH",
    "APP=$(find $HOME/Library/Developer/Xcode/DerivedData -path '*Build/Products/debug-iphonesimulator/HN Mobile.app' | head -n 1)",
    "test -n \"$APP\"",
    `xcrun simctl ui ${deviceTarget} appearance ${shQuote(options.appearance)}`,
    `xcrun simctl terminate ${deviceTarget} ${shQuote(options.remoteBundleId)} >/dev/null 2>&1 || true`,
    `xcrun simctl uninstall ${deviceTarget} ${shQuote(options.remoteBundleId)} >/dev/null 2>&1 || true`,
    `xcrun simctl install ${deviceTarget} "$APP" >/dev/null`,
    "sleep 1",
    `xcrun simctl launch ${deviceTarget} ${shQuote(options.remoteBundleId)} >/tmp/${options.sessionName}.prewarm.log 2>&1 || true`,
    `sleep ${(options.prewarmDelayMs / 1000).toFixed(3)}`,
    `xcrun simctl launch ${deviceTarget} com.apple.springboard >/tmp/${options.sessionName}.home.log 2>&1 || true`,
    `sleep ${(options.homeDwellMs / 1000).toFixed(3)}`,
    `rm -f ${shQuote(remoteVideoPath)} ${shQuote(remoteRecordLogPath)}`,
    `xcrun simctl io ${deviceTarget} recordVideo --codec=h264 --mask=black ${shQuote(remoteVideoPath)} > ${shQuote(remoteRecordLogPath)} 2>&1 &`,
    "REC_PID=$!",
    "while ! grep -q 'Recording started' " + shQuote(remoteRecordLogPath) + " 2>/dev/null; do sleep 0.1; done",
    `sleep ${(options.relaunchDelayMs / 1000).toFixed(3)}`,
    `xcrun simctl launch ${deviceTarget} ${shQuote(options.remoteBundleId)} >/tmp/${options.sessionName}.launch.log 2>&1 || true`,
    `sleep ${(options.recordingDurationMs / 1000).toFixed(3)}`,
    "kill -INT \"$REC_PID\"",
    "wait \"$REC_PID\" 2>/dev/null || true",
  ].join("\n");
}

export function resolveLocalVideoPlan(outputPath) {
  const extension = path.extname(outputPath).toLowerCase();
  if (extension === ".mov") {
    return { finalPath: outputPath, needsTranscode: false };
  }
  if (extension === ".mp4") {
    return { finalPath: outputPath, needsTranscode: true };
  }
  throw new Error("--output must end with .mov or .mp4");
}

function resolveRemotePort(target, requestedPort) {
  if (requestedPort && requestedPort > 0) {
    return requestedPort;
  }

  const script = [
    "python3 - <<'PY'",
    "import socket",
    "sock = socket.socket()",
    "sock.bind(('127.0.0.1', 0))",
    "print(sock.getsockname()[1])",
    "sock.close()",
    "PY",
  ].join("\n");
  const output = sshCapture(target, script, { stdio: ["ignore", "pipe", "pipe"] }).trim();
  const port = Number.parseInt(output, 10);
  if (!Number.isFinite(port) || port <= 0) {
    throw new Error(`Unable to resolve a free remote port from output: ${output}`);
  }
  return port;
}

async function waitForRemoteReady(target, options) {
  const deadline = Date.now() + options.readyTimeoutMs;
  let lastError = null;

  while (Date.now() < deadline) {
    try {
      sshCapture(target, buildRemoteReadyCheckCommand(options), { stdio: ["ignore", "pipe", "pipe"] });
      return;
    } catch (error) {
      lastError = error;
      await sleep(2_000);
    }
  }

  const tail = sshCapture(target, buildRemoteLogTailCommand(options), { stdio: ["ignore", "pipe", "pipe"] });
  throw new Error(`Timed out waiting for remote HN simulator app to become ready.\n${String(lastError ?? "")}\n${tail}`);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (existsSync(options.outputPath) && !options.force) {
    throw new Error(`Output already exists: ${options.outputPath}`);
  }

  mkdirSync(path.dirname(options.outputPath), { recursive: true });
  const tempDir = mkdtempSync(path.join(os.tmpdir(), "ctx-hn-mobile-sim-record-"));
  const localMovPath = path.join(tempDir, `${options.sessionName}.mov`);
  const sshTarget = `${options.remoteUser}@${options.remoteHost}`;
  options.remotePort = resolveRemotePort(sshTarget, options.remotePort);
  const localVideoPlan = resolveLocalVideoPlan(options.outputPath);

  try {
    sshCapture(sshTarget, buildRemotePrepareDeviceCommand(options), { stdio: ["ignore", "pipe", "pipe"] });
    sshRun(sshTarget, `mkdir -p ${shQuote(options.remoteWorkspaceRoot)}`);
    runChecked("rsync", buildRsyncArgs(options));

    sshCapture(sshTarget, buildRemoteBootstrapCommand(options)).trim();
    const remoteServerLogPath = resolveRemoteServerLogPath(options);
    const remoteBuildLogPath = resolveRemoteBuildLogPath(options);
    await waitForRemoteReady(sshTarget, options);

    sshCapture(sshTarget, buildRemoteRecordCommand(options)).trim();
    const remoteVideoPath = resolveRemoteVideoPath(options);
    runChecked("scp", ["-q", `${sshTarget}:${remoteVideoPath}`, localMovPath]);

    if (localVideoPlan.needsTranscode) {
      runChecked("ffmpeg", [
        "-y",
        "-i", localMovPath,
        "-vf", "format=yuv420p",
        "-c:v", "libx264",
        "-pix_fmt", "yuv420p",
        "-movflags", "+faststart",
        options.outputPath,
      ]);
    } else {
      runChecked("cp", [localMovPath, options.outputPath]);
    }

    if (!options.keepRemoteSession) {
      sshRun(sshTarget, buildRemoteCleanupCommand(options));
    }

    process.stdout.write(`${JSON.stringify({
      status: "ok",
      output: options.outputPath,
      remote_server_log: remoteServerLogPath,
      remote_build_log: remoteBuildLogPath,
      session_name: options.sessionName,
    }, null, 2)}\n`);
  } finally {
    rmSync(tempDir, { recursive: true, force: true });
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
