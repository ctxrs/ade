#!/usr/bin/env node
import { copyFileSync, createWriteStream, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";

import { startDemoRelay } from "./demo-relay/index.mjs";
import { bootstrapDemoFixture } from "./demo_fixture_bootstrap.mjs";
import { configureProviderEndpoint } from "./demo_configure_provider_endpoint.mjs";
import {
  api,
  getProviderStatus,
  installProviderAndWait,
  readDaemonAuth,
  waitFor,
  waitForFile,
  waitForHttpOk,
} from "./demo_lib.mjs";
import { findBundledProviderRuntime, seedProviderRuntimeConfig } from "./demo_ping_pong_playback.mjs";
import { inferVideoArtifactMimeType } from "./demo_video_artifacts.mjs";

const SCRIPT_DIR = path.dirname(new URL(import.meta.url).pathname);
const REPO_ROOT = path.resolve(SCRIPT_DIR, "../../../..");

function runStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace("T", "-").replace("Z", "");
}

function pickUnusedPortSync(fallback) {
  const script = [
    "const net = require('node:net');",
    "const server = net.createServer();",
    "server.on('error', () => process.exit(2));",
    "server.listen(0, '127.0.0.1', () => {",
    "  const addr = server.address();",
    "  const port = addr && typeof addr === 'object' ? addr.port : 0;",
    "  server.close(() => {",
    "    if (!port) process.exit(3);",
    "    process.stdout.write(String(port));",
    "  });",
    "});",
  ].join("\n");
  const result = spawnSync(process.execPath, ["-e", script], { encoding: "utf8" });
  if (result.status !== 0) {
    return fallback;
  }
  const parsed = Number.parseInt(String(result.stdout || "").trim(), 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

function parseArgs(argv) {
  const stamp = runStamp();
  const options = {
    relayArtifactDir: `/tmp/ctx-demo-hn-mobile-capture-${stamp}`,
    workspaceRoot: `/tmp/ctx-demo-hn-mobile-capture-workspace-${stamp}`,
    daemonDataDir: `/tmp/ctx-demo-hn-mobile-capture-daemon-${stamp}`,
    fixturePath: path.resolve(REPO_ROOT, "core/apps/desktop/automation/fixtures/demo-hn-mobile-fixture.json"),
    scenarioOut: path.resolve(REPO_ROOT, "core/apps/desktop/automation/fixtures/demo-relay/codex-hn-mobile.replay.json"),
    scenarioId: "codex-hn-mobile",
    promptText: null,
    sessionArtifactPath: "",
    recordArtifactPath: "",
    daemonPort: pickUnusedPortSync(4416),
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--relay-artifacts") {
      options.relayArtifactDir = path.resolve(next);
      index += 1;
    } else if (arg === "--workspace-root") {
      options.workspaceRoot = path.resolve(next);
      index += 1;
    } else if (arg === "--daemon-data-dir") {
      options.daemonDataDir = path.resolve(next);
      index += 1;
    } else if (arg === "--fixture") {
      options.fixturePath = path.resolve(next);
      index += 1;
    } else if (arg === "--scenario-out") {
      options.scenarioOut = path.resolve(next);
      index += 1;
    } else if (arg === "--scenario-id") {
      options.scenarioId = next;
      index += 1;
    } else if (arg === "--prompt") {
      options.promptText = next;
      index += 1;
    } else if (arg === "--session-artifact") {
      options.sessionArtifactPath = path.resolve(next);
      index += 1;
    } else if (arg === "--record-artifact-out") {
      options.recordArtifactPath = path.resolve(next);
      index += 1;
    } else if (arg === "--daemon-port") {
      options.daemonPort = Number.parseInt(next, 10);
      index += 1;
    }
  }
  return options;
}

function resolveFixturePath(fixturePath, maybeRelativePath) {
  if (!maybeRelativePath) {
    return "";
  }
  if (path.isAbsolute(maybeRelativePath)) {
    return maybeRelativePath;
  }
  return path.resolve(path.dirname(fixturePath), maybeRelativePath);
}

export function resolveCapturePrompt(fixture, explicitPrompt) {
  return explicitPrompt || fixture.capture_prompt || fixture.next_prompt || "";
}

async function startDaemon(dataDir, bind) {
  mkdirSync(dataDir, { recursive: true });
  const [host, port] = bind.split(":");
  const logPath = path.join(dataDir, "daemon.log");
  const logStream = createWriteStream(logPath, { flags: "a" });
  const daemonBin = path.resolve(REPO_ROOT, "core/target/debug/ctx");
  if (!existsSync(daemonBin)) {
    runChecked("cargo", ["build", "-p", "ctx-http", "--bin", "ctx"], {
      cwd: path.resolve(REPO_ROOT, "core"),
      env: {
        ...process.env,
        CTX_DEV_MODE: "1",
      },
    });
  }
  const proc = spawn(
    daemonBin,
    ["serve", "--bind", `${host}:${port}`, "--data-dir", dataDir],
    {
      cwd: REPO_ROOT,
      env: {
        ...process.env,
        CTX_DEV_MODE: "1",
      },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  proc.stdout.pipe(logStream, { end: false });
  proc.stderr.pipe(logStream, { end: false });
  await waitForFile(path.join(dataDir, "daemon_auth.json"), { timeoutMs: 300_000, label: "daemon_auth.json" });
  await waitForHttpOk(`http://${host}:${port}/api/health`, { timeoutMs: 300_000, label: "daemon health" });
  return {
    proc,
    dataDir,
    logPath,
    daemonUrl: `http://${host}:${port}`,
  };
}

async function waitForTaskSession(baseUrl, token, taskId) {
  return waitFor(async () => {
    const sessions = await api(baseUrl, token, "GET", `/api/tasks/${taskId}/sessions`);
    return Array.isArray(sessions) && sessions.length > 0 ? sessions[0] : null;
  }, {
    timeoutMs: 60_000,
    intervalMs: 1_000,
    label: `session creation for task ${taskId}`,
  });
}

async function readTaskRecord(baseUrl, token, workspaceId, taskId) {
  const tasks = await api(baseUrl, token, "GET", `/api/workspaces/${workspaceId}/tasks`);
  if (!Array.isArray(tasks)) {
    return null;
  }
  return tasks.find((task) => String(task.id) === String(taskId)) ?? null;
}

async function waitForSessionCompletion(baseUrl, token, sessionId) {
  return waitFor(async () => {
    const events = await api(baseUrl, token, "GET", `/api/sessions/${sessionId}/events?limit=400`);
    const list = Array.isArray(events) ? events : Array.isArray(events?.events) ? events.events : [];
    const failed = list.find((event) =>
      String(event.event_type) === "turn_finished"
      && String(event.payload_json?.status || "").toLowerCase() === "failed");
    if (failed) {
      const errorEvent = [...list].reverse().find((event) => String(event.event_type) === "error");
      const detail = errorEvent?.payload_json?.error || failed?.payload_json?.status || "turn failed";
      throw new Error(`session ${sessionId} failed: ${detail}`);
    }
    return list.find((event) =>
      String(event.event_type) === "turn_finished"
      && String(event.payload_json?.status || "").toLowerCase() === "completed") || null;
  }, {
    timeoutMs: 20 * 60_000,
    intervalMs: 1_500,
    label: `session ${sessionId} completion`,
  });
}

async function maybeAttachArtifact(baseUrl, token, sessionId, artifactPath, worktreeRoot) {
  if (!artifactPath) return null;
  if (!existsSync(artifactPath)) {
    throw new Error(`artifact path does not exist: ${artifactPath}`);
  }
  const stagedPath = stageSessionArtifactPath(worktreeRoot, artifactPath);
  return api(baseUrl, token, "POST", `/api/sessions/${sessionId}/artifacts`, {
    artifacts: [
      {
        absolute_file_path: stagedPath,
        name: path.basename(stagedPath),
        mime_type: inferVideoArtifactMimeType(stagedPath),
      },
    ],
  });
}

function stageSessionArtifactPath(worktreeRoot, artifactPath) {
  if (!artifactPath) return null;
  if (!worktreeRoot) {
    throw new Error("cannot attach a session artifact without a task worktree root");
  }
  const resolvedPath = path.resolve(artifactPath);
  const relativePath = path.relative(worktreeRoot, resolvedPath);
  if (
    relativePath
    && !relativePath.startsWith("..")
    && !path.isAbsolute(relativePath)
  ) {
    return resolvedPath;
  }

  const stagedDir = path.join(worktreeRoot, ".ctx-session-artifacts");
  mkdirSync(stagedDir, { recursive: true });
  const stagedPath = path.join(stagedDir, path.basename(resolvedPath));
  copyFileSync(resolvedPath, stagedPath);
  return stagedPath;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (!String(process.env.OPENROUTER_API_KEY || "").trim()) {
    throw new Error("OPENROUTER_API_KEY is required for live hn-mobile capture.");
  }

  mkdirSync(options.relayArtifactDir, { recursive: true });

  const fixture = JSON.parse(readFileSync(options.fixturePath, "utf8"));
  const promptText = resolveCapturePrompt(fixture, options.promptText);
  if (!promptText) {
    throw new Error("hn-mobile capture prompt is empty");
  }
  const recordedArtifactPath = options.recordArtifactPath || resolveFixturePath(options.fixturePath, fixture.session_artifact_path);

  let relayServer = null;
  let daemonProc = null;

  try {
    relayServer = await startDemoRelay({
      mode: "proxy",
      listenHost: "127.0.0.1",
      listenPort: 0,
      artifactDir: options.relayArtifactDir,
      scenarioPath: null,
      upstreamBaseUrl: "https://openrouter.ai/api/v1",
      upstreamApiKey: process.env.OPENROUTER_API_KEY,
    });

    const bundledCodexRuntime = findBundledProviderRuntime("codex", "codex-crp");
    if (bundledCodexRuntime) {
      seedProviderRuntimeConfig(options.daemonDataDir, "codex", bundledCodexRuntime);
    }

    const daemon = await startDaemon(options.daemonDataDir, `127.0.0.1:${options.daemonPort}`);
    daemonProc = daemon.proc;
    const auth = readDaemonAuth({
      daemonUrl: daemon.daemonUrl,
      dataDir: daemon.dataDir,
    });

    const providerStatusBefore = await getProviderStatus(auth.daemonUrl, auth.authToken, "codex", "host");
    if (!providerStatusBefore.installed) {
      await installProviderAndWait(auth.daemonUrl, auth.authToken, "codex", { target: "host" });
    }

    const bootstrap = await bootstrapDemoFixture({
      scenarioPath: options.fixturePath,
      workspaceRoot: options.workspaceRoot,
      daemonUrl: auth.daemonUrl,
      authToken: auth.authToken,
      dataDir: daemon.dataDir,
    });

    const endpointStamp = runStamp();
    await configureProviderEndpoint({
      providerId: "codex",
      endpointName: `demo-hn-mobile-capture-${endpointStamp}`,
      endpointId: `demo-hn-mobile-capture-${endpointStamp}`,
      baseUrl: `http://127.0.0.1:${relayServer.port}/v1`,
      modelId: fixture.model_id || "gpt-5.4",
      apiShape: "openai_responses",
      authType: "bearer",
      apiKey: "demo-key",
      daemonUrl: auth.daemonUrl,
      authToken: auth.authToken,
      dataDir: daemon.dataDir,
    });
    await api(auth.daemonUrl, auth.authToken, "POST", `/api/workspaces/${bootstrap.workspace_id}/providers/codex/verify`, {});

    const task = await api(auth.daemonUrl, auth.authToken, "POST", `/api/workspaces/${bootstrap.workspace_id}/tasks`, {
      title: "HN Mobile Saved Stories Demo",
      create_default_session: false,
    });
    const session = await api(auth.daemonUrl, auth.authToken, "POST", `/api/tasks/${task.id}/sessions`, {
      provider_id: fixture.provider_id || "codex",
      model_id: fixture.model_id || "gpt-5.4",
    });

    await api(auth.daemonUrl, auth.authToken, "POST", `/api/sessions/${session.id}/messages`, {
      content: promptText,
      delivery: "immediate",
      attachments: [],
    });

    await waitForTaskSession(auth.daemonUrl, auth.authToken, task.id);
    await waitForSessionCompletion(auth.daemonUrl, auth.authToken, session.id);
    const taskRecord = await readTaskRecord(auth.daemonUrl, auth.authToken, bootstrap.workspace_id, task.id);
    const worktreeRoot = taskRecord?.primary_worktree_id
      ? path.join(daemon.dataDir, "worktrees", bootstrap.workspace_id, taskRecord.primary_worktree_id)
      : null;

    let effectiveArtifactPath = options.sessionArtifactPath;
    if (recordedArtifactPath) {
      if (!worktreeRoot) {
        throw new Error("cannot record artifact because the task primary worktree is unavailable");
      }
      const artifactRecorderScript = path.resolve(REPO_ROOT, "core/apps/desktop/scripts/demo_hn_mobile_record_artifact.mjs");
      runChecked("pnpm", [
        "-C", path.resolve(REPO_ROOT, "core"),
        "exec",
        "node",
        artifactRecorderScript,
        "--workspace-root", worktreeRoot,
        "--output", recordedArtifactPath,
      ], { cwd: REPO_ROOT });
      effectiveArtifactPath = recordedArtifactPath;
    }

    const artifacts = await maybeAttachArtifact(
      auth.daemonUrl,
      auth.authToken,
      session.id,
      effectiveArtifactPath,
      worktreeRoot,
    );

    const captureScenarioScript = path.resolve(REPO_ROOT, "core/apps/desktop/scripts/demo_capture_to_scenario.mjs");
    runChecked(process.execPath, [
      captureScenarioScript,
      "--requests-log", path.join(options.relayArtifactDir, "requests.jsonl"),
      "--responses-log", path.join(options.relayArtifactDir, "responses.jsonl"),
      "--scenario", options.scenarioOut,
      "--scenario-id", options.scenarioId,
      "--match-text", promptText,
      "--response-delay-ms", "80",
    ], { cwd: REPO_ROOT });

    const result = {
      status: "ok",
      relay_artifact_dir: options.relayArtifactDir,
      workspace_root: options.workspaceRoot,
      daemon_data_dir: options.daemonDataDir,
      daemon_url: auth.daemonUrl,
      workspace_id: bootstrap.workspace_id,
      worktree_root: worktreeRoot,
      task_id: task.id,
      session_id: session.id,
      scenario_out: options.scenarioOut,
      recorded_artifact_path: effectiveArtifactPath || null,
      artifacts,
    };
    writeFileSync(path.join(options.relayArtifactDir, "capture-result.json"), `${JSON.stringify(result, null, 2)}\n`, "utf8");
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } finally {
    if (relayServer) {
      await relayServer.close();
    }
    if (daemonProc) {
      daemonProc.kill("SIGTERM");
    }
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
