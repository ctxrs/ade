#!/usr/bin/env node
import { createWriteStream, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { spawn } from "node:child_process";

import { startDemoRelay } from "./demo-relay/index.mjs";
import { bootstrapDemoFixture } from "./demo_fixture_bootstrap.mjs";
import { configureProviderEndpoint } from "./demo_configure_provider_endpoint.mjs";
import {
  api,
  getProviderStatus,
  installProviderAndWait,
  readDaemonAuth,
  waitForFile,
  waitForHttpOk,
} from "./demo_lib.mjs";

function runStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace("T", "-").replace("Z", "");
}

function parseArgs(argv) {
  const stamp = runStamp();
  const out = {
    fixturePath: resolve("core/apps/desktop/automation/fixtures/demo-ping-pong-fixture.json"),
    workspaceRoot: `/tmp/ctx-demo-ping-pong-${stamp}`,
    relayArtifactDir: `/tmp/ctx-demo-relay-ping-pong-${stamp}`,
    relayScenarioPath: resolve("core/apps/desktop/automation/fixtures/demo-relay/codex-ping-pong.replay.json"),
    daemonDataDir: `/tmp/ctx-demo-daemon-ping-pong-${stamp}`,
    daemonBind: "127.0.0.1:4407",
    daemonUrl: "",
    authToken: "",
    launchDesktop: false,
    desktopAppPath: "",
    desktopDev: false,
    skipVerify: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = argv[i + 1];
    if (arg === "--fixture") {
      out.fixturePath = resolve(next);
      i += 1;
    } else if (arg === "--workspace-root") {
      out.workspaceRoot = resolve(next);
      i += 1;
    } else if (arg === "--relay-artifacts") {
      out.relayArtifactDir = resolve(next);
      i += 1;
    } else if (arg === "--relay-scenario") {
      out.relayScenarioPath = resolve(next);
      i += 1;
    } else if (arg === "--daemon-data-dir") {
      out.daemonDataDir = resolve(next);
      i += 1;
    } else if (arg === "--daemon-bind") {
      out.daemonBind = next;
      i += 1;
    } else if (arg === "--daemon-url") {
      out.daemonUrl = next;
      i += 1;
    } else if (arg === "--auth-token") {
      out.authToken = next;
      i += 1;
    } else if (arg === "--launch-desktop") {
      out.launchDesktop = true;
    } else if (arg === "--desktop-app-path") {
      out.desktopAppPath = next;
      i += 1;
    } else if (arg === "--desktop-dev") {
      out.desktopDev = true;
    } else if (arg === "--skip-verify") {
      out.skipVerify = true;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  return out;
}

function printHelp() {
  process.stdout.write(`demo_ping_pong_orchestrator

Usage:
  node core/apps/desktop/scripts/demo_ping_pong_orchestrator.mjs --launch-desktop --desktop-dev

This prepares a complete ping pong demo slice:
  1. load the checked-in replayable Codex Responses scenario fixture
  2. start the local replay relay
  3. start an isolated dev-mode daemon
  4. point Codex at the relay via provider endpoint mode
  5. seed a workspace/task/session transcript fixture
  6. optionally launch the desktop app against that daemon
`);
}

async function startDaemon(dataDir, bind) {
  mkdirSync(dataDir, { recursive: true });
  const [host, port] = bind.split(":");
  const logPath = join(dataDir, "daemon.log");
  const logStream = createWriteStream(logPath, { flags: "a" });
  const proc = spawn(
    "cargo",
    ["run", "-p", "ctx-http", "--bin", "ctx", "--", "serve", "--bind", `${host}:${port}`, "--data-dir", dataDir],
    {
      cwd: resolve("core"),
      env: {
        ...process.env,
        CTX_DEV_MODE: "1",
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  proc.stdout.pipe(logStream, { end: false });
  proc.stderr.pipe(logStream, { end: false });
  proc.on("exit", (code, signal) => {
    if (!logStream.destroyed) {
      logStream.write(`process exited code=${code} signal=${signal}\n`);
      logStream.end();
    }
  });
  await waitForFile(join(dataDir, "daemon_auth.json"), { timeoutMs: 300_000, label: "daemon_auth.json" });
  await waitForHttpOk(`http://${host}:${port}/api/health`, { timeoutMs: 300_000, label: "daemon health" });
  return {
    proc,
    logPath,
    dataDir,
    daemonUrl: `http://${host}:${port}`,
  };
}

async function maybeLaunchDesktop({ daemonUrl, authToken, workspaceRoot, desktopAppPath, desktopDev }) {
  const env = {
    ...process.env,
    CTX_DESKTOP_DAEMON_URL: daemonUrl,
    CTX_DESKTOP_DAEMON_TOKEN: authToken,
    CTX_DESKTOP_START_WORKSPACE_PATHS: workspaceRoot,
  };
  if (desktopDev) {
    return spawn("pnpm", ["-C", "core", "desktop:dev"], {
      cwd: resolve("."),
      env,
      stdio: "inherit",
    });
  }
  if (desktopAppPath) {
    return spawn("open", [desktopAppPath], {
      cwd: resolve("."),
      env,
      stdio: "inherit",
    });
  }
  return null;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const stamp = runStamp();
  mkdirSync(options.relayArtifactDir, { recursive: true });

  const relay = await startDemoRelay({
    mode: "replay",
    listenHost: "127.0.0.1",
    listenPort: 0,
    artifactDir: options.relayArtifactDir,
    scenarioPath: options.relayScenarioPath,
    upstreamBaseUrl: "",
    upstreamApiKey: "",
    rewriteFrom: "",
    rewriteTo: "",
  });

  const daemon = options.daemonUrl
    ? { daemonUrl: options.daemonUrl, proc: null, dataDir: options.daemonDataDir, logPath: join(options.daemonDataDir, "daemon.log") }
    : await startDaemon(options.daemonDataDir, options.daemonBind);
  const auth = readDaemonAuth({
    daemonUrl: daemon.daemonUrl,
    authToken: options.authToken,
    dataDir: daemon.dataDir,
  });

  const providerStatusBefore = await getProviderStatus(auth.daemonUrl, auth.authToken, "codex", "host");
  const install = providerStatusBefore.installed
    ? null
    : await installProviderAndWait(auth.daemonUrl, auth.authToken, "codex", { target: "host" });
  const providerStatusAfterInstall = await getProviderStatus(auth.daemonUrl, auth.authToken, "codex", "host");

  const fixture = await bootstrapDemoFixture({
    scenarioPath: options.fixturePath,
    workspaceRoot: options.workspaceRoot,
    daemonUrl: auth.daemonUrl,
    authToken: auth.authToken,
    dataDir: daemon.dataDir,
  });

  const provider = await configureProviderEndpoint({
    providerId: "codex",
    endpointName: `demo-relay-${stamp}`,
    endpointId: `demo-relay-${stamp}`,
    baseUrl: `http://127.0.0.1:${relay.port}/v1`,
    modelId: "gpt-5.4",
    apiShape: "openai_responses",
    authType: "bearer",
    apiKey: "demo-key",
    daemonUrl: auth.daemonUrl,
    authToken: auth.authToken,
    dataDir: daemon.dataDir,
  });

  const verify = options.skipVerify
    ? null
    : await api(auth.daemonUrl, auth.authToken, "POST", `/api/workspaces/${fixture.workspace_id}/providers/codex/verify`, {});
  const desktopProc = options.launchDesktop
    ? await maybeLaunchDesktop({
        daemonUrl: auth.daemonUrl,
        authToken: auth.authToken,
        workspaceRoot: options.workspaceRoot,
        desktopAppPath: options.desktopAppPath,
        desktopDev: options.desktopDev,
      })
    : null;

  const manifest = {
    fixture,
    provider_status_before: providerStatusBefore,
    install,
    provider_status_after_install: providerStatusAfterInstall,
    provider,
    verify,
    relay: {
      artifact_dir: options.relayArtifactDir,
      scenario_path: options.relayScenarioPath,
      sse_path: null,
      port: relay.port,
    },
    daemon: {
      url: auth.daemonUrl,
      data_dir: daemon.dataDir,
      log_path: daemon.logPath,
    },
    desktop: {
      launched: Boolean(desktopProc),
      mode: options.desktopDev ? "dev" : options.desktopAppPath ? "app" : "none",
    },
    next_steps: [
      "If the desktop app was launched, focus the seeded session and let the macOS conductor type the next prompt.",
      "Run: swift core/apps/desktop/scripts/macos_demo_conductor.swift --scenario core/apps/desktop/automation/fixtures/demo-conductor-ping-pong.json",
      "Watch the real daemon use the replay relay and then open the diff pane in the app to show index.html.",
    ],
  };

  writeFileSync(join(options.relayArtifactDir, "demo-manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
