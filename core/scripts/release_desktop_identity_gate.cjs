#!/usr/bin/env node

const fs = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const ARTIFACT_IDENTITY_FILENAME = "artifact_identity.json";
const HEALTH_TIMEOUT_MS = 45_000;
const HEALTH_RETRY_MS = 250;

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
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

function requestJson(url) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => {
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

async function waitForHealth(url, child, logPath) {
  const deadline = Date.now() + HEALTH_TIMEOUT_MS;
  let lastError = null;
  while (Date.now() < deadline) {
    try {
      return await requestJson(url);
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
  const identity = readArtifactIdentity(bundleDir);
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
  const child = childProcess.spawn(
    daemonBin,
    ["serve", "--bind", bindAddr, "--data-dir", dataDir],
    {
      env: daemonEnv,
      stdio: ["ignore", "ignore", "pipe"],
    },
  );
  child.stderr.pipe(logStream);

  let health;
  try {
    health = await waitForHealth(`http://${bindAddr}/api/health`, child, logPath);
  } finally {
    child.kill("SIGTERM");
    await new Promise((resolve) => child.once("exit", resolve));
    logStream.end();
    fs.rmSync(tempRoot, { recursive: true, force: true });
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
  if (mismatches.length > 0) {
    fail(`bundled daemon identity mismatch (${mismatches.join("; ")})`);
  }
  console.log(
    `release_desktop_identity_gate: OK (version=${identity.exactVersion} build_id=${identity.buildId} compatibility_token=${identity.compatibilityToken})`,
  );
}

main().catch((error) => {
  fail(error instanceof Error ? error.message : String(error));
});
