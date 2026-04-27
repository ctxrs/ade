#!/usr/bin/env node

const fs = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const ARTIFACT_IDENTITY_FILENAME = "artifact_identity.json";
const HEALTH_TIMEOUT_MS = parsePositiveIntegerEnv("CTX_DESKTOP_IDENTITY_GATE_HEALTH_TIMEOUT_MS", 45_000);
const HEALTH_RETRY_MS = 250;
const AUTH_FILENAME = "daemon_auth.json";
const BUNDLED_COMMAND_TIMEOUT_MS = parsePositiveIntegerEnv(
  "CTX_DESKTOP_IDENTITY_GATE_COMMAND_TIMEOUT_MS",
  10_000,
);
const DAEMON_SHUTDOWN_TIMEOUT_MS = parsePositiveIntegerEnv(
  "CTX_DESKTOP_IDENTITY_GATE_DAEMON_SHUTDOWN_TIMEOUT_MS",
  5_000,
);

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function parsePositiveIntegerEnv(name, fallback) {
  const raw = process.env[name];
  if (raw === undefined || raw.trim() === "") {
    return fallback;
  }
  const value = Number.parseInt(raw, 10);
  if (!Number.isSafeInteger(value) || value <= 0) {
    fail(`${name} must be a positive integer when set`);
  }
  return value;
}

function parseArgs(argv) {
  const out = {
    app: "",
    platform: "",
  };
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--app") {
      out.app = argv[++i] || "";
      continue;
    }
    if (arg === "--platform") {
      out.platform = argv[++i] || "";
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      console.log("Usage: node core/scripts/release_desktop_identity_gate.cjs --app <ctx.app> --platform <platform>");
      process.exit(0);
    }
    fail(`unknown argument: ${arg}`);
  }
  if (!out.app) fail("missing required --app");
  if (!out.platform) fail("missing required --platform");
  return out;
}

function targetTripleForPlatform(platform) {
  switch (platform) {
    case "macos-arm64":
      return "aarch64-apple-darwin";
    case "macos-x64":
      return "x86_64-apple-darwin";
    case "linux-arm64":
      return "aarch64-unknown-linux-gnu";
    case "linux-x64":
      return "x86_64-unknown-linux-gnu";
    case "windows-x64":
      return "x86_64-pc-windows-msvc";
    default:
      fail(`unsupported --platform value: ${platform}`);
  }
}

function shouldUseStaticOnlyValidation({
  platform,
  hostPlatform = process.platform,
  hostArch = process.arch,
} = {}) {
  return platform === "macos-arm64" && hostPlatform === "darwin" && hostArch === "x64";
}

function resourcePaths(appPath, platform) {
  const resourcesRoot = path.join(appPath, "Contents", "Resources");
  const bundleDir = path.join(resourcesRoot, "bundles");
  const targetTriple = targetTripleForPlatform(platform);
  const binExt = platform.startsWith("windows-") ? ".exe" : "";
  const daemonBin = path.join(resourcesRoot, "bin", `ctx-daemon-${targetTriple}${binExt}`);
  const mcpBin = path.join(resourcesRoot, "bin", `ctx-mcp-${targetTriple}${binExt}`);
  const avfLinuxHelper = path.join(resourcesRoot, "bin", `ctx-avf-linux-helper-${targetTriple}${binExt}`);
  return {
    bundleDir,
    daemonBin,
    mcpBin,
    avfLinuxHelper,
  };
}

function readArtifactIdentity(bundleDir) {
  const identityPath = path.join(bundleDir, ARTIFACT_IDENTITY_FILENAME);
  if (!fs.existsSync(identityPath)) {
    fail(`missing artifact identity: ${identityPath}`);
  }
  const identity = JSON.parse(fs.readFileSync(identityPath, "utf8"));
  if (!identity || identity.schemaVersion !== 1) {
    fail(`invalid artifact identity schema in ${identityPath}`);
  }
  for (const key of ["exactVersion", "buildId", "compatibilityToken"]) {
    if (!String(identity[key] || "").trim()) {
      fail(`artifact identity missing ${key} in ${identityPath}`);
    }
  }
  return identity;
}

function formatExecFailure(result, label, timeoutMs) {
  if (result?.error?.code === "ETIMEDOUT") {
    return `${label} timed out after ${timeoutMs}ms`;
  }
  if (result?.error) {
    const code = String(result.error.code || "").trim();
    const message = String(result.error.message || result.error).trim();
    const details = [code, message].filter(Boolean).join(": ");
    return details ? `${label} failed to launch: ${details}` : `${label} failed to launch`;
  }
  const stderr = String(result?.stderr || "").trim();
  const stdout = String(result?.stdout || "").trim();
  if (stderr) {
    return `${label} failed: ${stderr}`;
  }
  if (stdout) {
    return `${label} failed: ${stdout}`;
  }
  if (result?.signal) {
    return `${label} terminated with signal ${result.signal}`;
  }
  return `${label} failed`;
}

function parseLastJsonLine(stdout, label) {
  const lines = String(stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length === 0) {
    fail(`${label} produced no JSON output`);
  }
  try {
    return JSON.parse(lines[lines.length - 1]);
  } catch (error) {
    fail(`${label} returned invalid JSON: ${error instanceof Error ? error.message : String(error)}`);
  }
}

function isShebangScript(filePath) {
  const stat = fs.statSync(filePath);
  if (!stat.isFile() || stat.size < 2) {
    return false;
  }
  const header = fs.readFileSync(filePath, { encoding: null, flag: "r" }).subarray(0, 2);
  return header[0] === 0x23 && header[1] === 0x21;
}

function runBundledJsonCommand(binaryPath, args, { env, label, input = "" }) {
  if (!fs.existsSync(binaryPath)) {
    fail(`missing bundled binary for ${label}: ${binaryPath}`);
  }
  const command = isShebangScript(binaryPath) ? "/bin/sh" : binaryPath;
  const commandArgs = command === binaryPath ? args : [binaryPath, ...args];
  const result = childProcess.spawnSync(command, commandArgs, {
    env: {
      ...env,
    },
    input,
    encoding: "utf8",
    timeout: BUNDLED_COMMAND_TIMEOUT_MS,
  });
  if (result.status !== 0) {
    fail(formatExecFailure(result, label, BUNDLED_COMMAND_TIMEOUT_MS));
  }
  return parseLastJsonLine(result.stdout, label);
}

function spawnBundledCommand(binaryPath, args, options) {
  if (!fs.existsSync(binaryPath)) {
    fail(`missing bundled binary: ${binaryPath}`);
  }
  const command = isShebangScript(binaryPath) ? "/bin/sh" : binaryPath;
  const commandArgs = command === binaryPath ? args : [binaryPath, ...args];
  return childProcess.spawn(command, commandArgs, options);
}

function probeBundledHelper(helperPath, bundleDir) {
  return runBundledJsonCommand(helperPath, ["probe"], {
    env: {
      ...process.env,
      CTX_BUNDLE_DIR: bundleDir,
    },
    label: "bundled helper probe",
  });
}

function initializeBundledMcp(mcpBin, bundleDir) {
  const request = {
    jsonrpc: "2.0",
    id: "identity-gate",
    method: "initialize",
    params: {
      protocolVersion: "2025-11-25",
      capabilities: {},
      clientInfo: {
        name: "release_desktop_identity_gate",
        version: "1",
      },
    },
  };
  const response = runBundledJsonCommand(mcpBin, ["--stdio"], {
    env: {
      ...process.env,
      CTX_BUNDLE_DIR: bundleDir,
    },
    label: "bundled MCP initialize",
    input: `${JSON.stringify(request)}\n`,
  });
  const version = String(response?.result?.serverInfo?.version || "").trim();
  if (!version) {
    fail("bundled MCP initialize response missing serverInfo.version");
  }
  return version;
}

function pickUnusedPort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      server.close((closeErr) => {
        if (closeErr) {
          reject(closeErr);
          return;
        }
        resolve(address.port);
      });
    });
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function readDaemonAuthToken(dataDir, child, logPath) {
  const pathName = path.join(dataDir, AUTH_FILENAME);
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      const auth = JSON.parse(fs.readFileSync(pathName, "utf8"));
      const token = String(auth?.token || "").trim();
      if (!token) {
        throw new Error(`${AUTH_FILENAME} contains empty token`);
      }
      return token;
    } catch (error) {
      lastError = error;
      const exit = child.exitCode;
      if (exit !== null) {
        const logs = fs.existsSync(logPath) ? fs.readFileSync(logPath, "utf8") : "";
        throw new Error(`daemon exited (${exit}) before health check succeeded: ${logs || error.message}`);
      }
      await delay(HEALTH_RETRY_MS);
    }
  }
  throw new Error(`daemon auth token did not become readable: ${lastError ? lastError.message : "unknown"}`);
}

function requestJson(url, authToken = "") {
  return new Promise((resolve, reject) => {
    const headers = authToken ? { Authorization: `Bearer ${authToken}` } : {};
    const req = http.get(url, { headers }, (res) => {
      let raw = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => {
        raw += chunk;
      });
      res.on("end", () => {
        if (res.statusCode < 200 || res.statusCode >= 300) {
          reject(new Error(`status ${res.statusCode}: ${raw.trim()}`));
          return;
        }
        try {
          resolve(JSON.parse(raw));
        } catch (error) {
          reject(error);
        }
      });
    });
    req.on("error", reject);
  });
}

async function waitForHealth(url, child, logPath, authToken) {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      return await requestJson(url, authToken);
    } catch (error) {
      lastError = error;
      const exit = child.exitCode;
      if (exit !== null) {
        const logs = fs.existsSync(logPath) ? fs.readFileSync(logPath, "utf8") : "";
        throw new Error(`daemon exited (${exit}) before health check succeeded: ${logs || error.message}`);
      }
      await new Promise((resolve) => setTimeout(resolve, HEALTH_RETRY_MS));
    }
  }
  const logs = fs.existsSync(logPath) ? fs.readFileSync(logPath, "utf8") : "";
  throw new Error(`daemon did not become healthy in time: ${lastError ? lastError.message : "unknown"}${logs ? `; logs: ${logs}` : ""}`);
}

async function terminateDaemon(child) {
  if (child.exitCode !== null || child.signalCode !== null) {
    return;
  }
  child.kill("SIGTERM");
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const timeout = new Promise((resolve) => {
    setTimeout(() => {
      if (child.exitCode === null && child.signalCode === null) {
        child.kill("SIGKILL");
      }
      resolve();
    }, DAEMON_SHUTDOWN_TIMEOUT_MS);
  });
  await Promise.race([exited, timeout]);
}

async function main() {
  const args = parseArgs(process.argv);
  const appPath = path.resolve(args.app);
  if (!fs.existsSync(appPath)) {
    fail(`app path does not exist: ${appPath}`);
  }
  const { bundleDir, daemonBin, mcpBin, avfLinuxHelper } = resourcePaths(appPath, args.platform);
  if (!fs.existsSync(bundleDir)) {
    fail(`missing bundles dir in app: ${bundleDir}`);
  }
  if (!fs.existsSync(daemonBin)) {
    fail(`missing bundled daemon binary: ${daemonBin}`);
  }
  if (!fs.existsSync(mcpBin)) {
    fail(`missing bundled MCP binary: ${mcpBin}`);
  }
  if (!fs.existsSync(avfLinuxHelper)) {
    fail(`missing bundled AVF helper binary: ${avfLinuxHelper}`);
  }
  const identity = readArtifactIdentity(bundleDir);
  if (shouldUseStaticOnlyValidation({ platform: args.platform })) {
    console.log(
      `release_desktop_identity_gate: OK (static_only=1 reason=host_cannot_execute_target version=${identity.exactVersion} build_id=${identity.buildId} compatibility_token=${identity.compatibilityToken})`,
    );
    return;
  }
  const helperProbe = probeBundledHelper(avfLinuxHelper, bundleDir);
  const mcpVersion = initializeBundledMcp(mcpBin, bundleDir);
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-identity-gate-"));
  const dataDir = path.join(tempRoot, "data");
  const logPath = path.join(tempRoot, "daemon.log");
  fs.mkdirSync(dataDir, { recursive: true });
  const port = await pickUnusedPort();
  const bindAddr = `127.0.0.1:${port}`;
  const daemonEnv = {
    ...process.env,
    CTX_BUNDLE_DIR: bundleDir,
    ...(fs.existsSync(mcpBin) ? { CTX_MCP_COMMAND: mcpBin } : {}),
    ...(fs.existsSync(avfLinuxHelper) ? { CTX_AVF_LINUX_HELPER_PATH: avfLinuxHelper } : {}),
  };
  const logStream = fs.createWriteStream(logPath, { flags: "a" });
  const child = spawnBundledCommand(daemonBin, ["serve", "--bind", bindAddr, "--data-dir", dataDir], {
    env: daemonEnv,
    stdio: ["ignore", "ignore", "pipe"],
  });
  child.stderr.pipe(logStream);

  let health;
  try {
    const authToken = await readDaemonAuthToken(dataDir, child, logPath);
    health = await waitForHealth(`http://${bindAddr}/api/health`, child, logPath, authToken);
  } finally {
    await terminateDaemon(child);
    await new Promise((resolve) => logStream.end(resolve));
    fs.rmSync(tempRoot, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
  }

  const compatibility = health?.compatibility || {};
  const mismatches = [];
  if (String(compatibility.desktop_exact_version || "").trim() !== identity.exactVersion) {
    mismatches.push(`version expected=${identity.exactVersion} actual=${compatibility.desktop_exact_version || "<empty>"}`);
  }
  if (String(compatibility.desktop_build_id || "").trim() !== identity.buildId) {
    mismatches.push(`build_id expected=${identity.buildId} actual=${compatibility.desktop_build_id || "<empty>"}`);
  }
  if (String(compatibility.desktop_dev_instance_id || "").trim() !== identity.compatibilityToken) {
    mismatches.push(`compatibility_token expected=${identity.compatibilityToken} actual=${compatibility.desktop_dev_instance_id || "<empty>"}`);
  }
  if (String(helperProbe.helper_version || "").trim() !== identity.exactVersion) {
    mismatches.push(`helper_version expected=${identity.exactVersion} actual=${helperProbe.helper_version || "<empty>"}`);
  }
  if (String(helperProbe.exact_version || "").trim() !== identity.exactVersion) {
    mismatches.push(`helper_exact_version expected=${identity.exactVersion} actual=${helperProbe.exact_version || "<empty>"}`);
  }
  if (String(helperProbe.build_id || "").trim() !== identity.buildId) {
    mismatches.push(`helper_build_id expected=${identity.buildId} actual=${helperProbe.build_id || "<empty>"}`);
  }
  if (String(helperProbe.compatibility_token || "").trim() !== identity.compatibilityToken) {
    mismatches.push(`helper_compatibility_token expected=${identity.compatibilityToken} actual=${helperProbe.compatibility_token || "<empty>"}`);
  }
  if (mcpVersion !== identity.exactVersion) {
    mismatches.push(`mcp_version expected=${identity.exactVersion} actual=${mcpVersion || "<empty>"}`);
  }
  if (mismatches.length > 0) {
    fail(`bundled artifact identity mismatch (${mismatches.join("; ")})`);
  }
  console.log(
    `release_desktop_identity_gate: OK (version=${identity.exactVersion} build_id=${identity.buildId} compatibility_token=${identity.compatibilityToken} helper_version=${helperProbe.helper_version} mcp_version=${mcpVersion})`,
  );
}

if (require.main === module) {
  main().catch((error) => {
    fail(error instanceof Error ? error.message : String(error));
  });
}

module.exports = {
  shouldUseStaticOnlyValidation,
};
