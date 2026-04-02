#!/usr/bin/env node
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { spawn, spawnSync } from "node:child_process";
import path from "node:path";
import readline from "node:readline";

import {
  buildScreenStudioProjectBundle,
  buildScreenStudioRecordConfig,
  extractDotenvVariable,
  findScreenStudioCaptureWindow,
  SCREEN_STUDIO_APP_PATH,
  SCREEN_STUDIO_DEFAULT_PROJECTS_DIR,
  SCREEN_STUDIO_DEFAULT_RECORDINGS_DIR,
  SCREEN_STUDIO_LIST_WINDOWS_PATH,
  SCREEN_STUDIO_POLYRECORDER_PATH,
} from "./demo_screenstudio_local.mjs";

const REPO_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../..");
const PLAYBACK_SCRIPT_PATH = path.resolve(REPO_ROOT, "core/apps/desktop/scripts/demo_hn_mobile_playback.mjs");
const DEFAULT_WINDOW_TITLE = "hn-mobile";
const DEFAULT_READY_PREROLL_MS = 3000;
const SIDEBAR_READY_FILENAME = "sidebar-task-state.json";
const DEFAULT_SIDEBAR_READY_TIMEOUT_MS = 12 * 60_000;

function runStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace("T", "-").replace("Z", "");
}

function writeJson(pathname, value) {
  writeFileSync(pathname, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: "pipe",
    encoding: "utf8",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}: ${String(result.stderr || "").trim()}`);
  }
  return result;
}

export function parseArgs(argv) {
  const stamp = runStamp();
  const outputTitle = `ctx-hn-muted-domains-screenstudio-${stamp}`;
  const options = {
    artifactDir: `/tmp/ctx-demo-hn-mobile-screenstudio-${stamp}`,
    outputProjectPath: path.join(SCREEN_STUDIO_DEFAULT_PROJECTS_DIR, `${outputTitle}.screenstudio`),
    outputTitle,
    rawRecordingDir: path.join(SCREEN_STUDIO_DEFAULT_RECORDINGS_DIR, outputTitle),
    tauriTargetDir: "/tmp/ctx-demo-desktop-target-screenstudio-local",
    daemonDataDir: "/tmp/ctx-demo-daemon-hn-mobile-screenstudio-local",
    windowTitle: DEFAULT_WINDOW_TITLE,
    readyPrerollMs: DEFAULT_READY_PREROLL_MS,
    sidebarReadyTimeoutMs: DEFAULT_SIDEBAR_READY_TIMEOUT_MS,
    openProject: true,
    playbackArgs: [],
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--artifact-dir") {
      options.artifactDir = path.resolve(next);
      index += 1;
    } else if (arg === "--output-project") {
      options.outputProjectPath = path.resolve(next);
      options.outputTitle = path.basename(options.outputProjectPath, ".screenstudio");
      options.rawRecordingDir = path.join(SCREEN_STUDIO_DEFAULT_RECORDINGS_DIR, options.outputTitle);
      index += 1;
    } else if (arg === "--tauri-target-dir") {
      options.tauriTargetDir = path.resolve(next);
      index += 1;
    } else if (arg === "--daemon-data-dir") {
      options.daemonDataDir = path.resolve(next);
      index += 1;
    } else if (arg === "--window-title") {
      options.windowTitle = next;
      index += 1;
    } else if (arg === "--ready-preroll-ms") {
      options.readyPrerollMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--sidebar-ready-timeout-ms") {
      options.sidebarReadyTimeoutMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--no-open-project") {
      options.openProject = false;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    } else {
      options.playbackArgs.push(arg);
    }
  }
  return options;
}

function printHelp() {
  process.stdout.write(`demo_hn_mobile_record_screenstudio_local

Usage:
  node core/apps/desktop/scripts/demo_hn_mobile_record_screenstudio_local.mjs

Options:
  --artifact-dir <dir>
  --output-project <path.screenstudio>
  --tauri-target-dir <dir>
  --daemon-data-dir <dir>
  --window-title <title>
  --ready-preroll-ms <ms>
  --sidebar-ready-timeout-ms <ms>
  --no-open-project

Additional arguments are passed through to demo_hn_mobile_playback.mjs.
`);
}

function createDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function waitForPath(pathname, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  return new Promise((resolve, reject) => {
    const poll = () => {
      if (existsSync(pathname)) {
        resolve(pathname);
        return;
      }
      if (Date.now() >= deadline) {
        reject(new Error(`timed out waiting for ${pathname}`));
        return;
      }
      setTimeout(poll, 250);
    };
    poll();
  });
}

function startPlaybackProcess(options) {
  mkdirSync(options.artifactDir, { recursive: true });
  const args = [
    PLAYBACK_SCRIPT_PATH,
    "--artifact-dir",
    options.artifactDir,
    "--tauri-target-dir",
    options.tauriTargetDir,
    "--daemon-data-dir",
    options.daemonDataDir,
    "--ready-preroll-ms",
    String(options.readyPrerollMs),
    ...options.playbackArgs,
  ];
  const proc = spawn(process.execPath, args, {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      CN_API_KEY: options.cnApiKey,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  proc.stdout.setEncoding("utf8");
  proc.stderr.setEncoding("utf8");
  proc.stdout.on("data", (chunk) => process.stdout.write(chunk));
  proc.stderr.on("data", (chunk) => process.stderr.write(chunk));
  return proc;
}

function waitForProcessExit(proc, label) {
  return new Promise((resolve, reject) => {
    proc.once("error", reject);
    proc.once("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${label} exited with code ${code ?? "null"} signal ${signal ?? "null"}`));
    });
  });
}

function discoverMainDisplayId() {
  const logFilePath = `/tmp/screenstudio-discover-${runStamp()}.log`;
  const result = runChecked(SCREEN_STUDIO_POLYRECORDER_PATH, ["discover", "--log-file-path", logFilePath, "--once"]);
  const discovery = JSON.parse(String(result.stdout || "{}"));
  const displays = Array.isArray(discovery.displays) ? discovery.displays : [];
  const mainDisplay = displays.find((display) => display && display.isMain);
  if (!mainDisplay || !Number.isInteger(Number(mainDisplay.displayID))) {
    throw new Error("failed to discover Screen Studio main display");
  }
  return Number(mainDisplay.displayID);
}

function ensurePlaybackSidecars(tauriTargetDir) {
  mkdirSync(tauriTargetDir, { recursive: true });
  const env = {
    ...process.env,
    CARGO_TARGET_DIR: tauriTargetDir,
  };
  runChecked("cargo", ["build", "--manifest-path", "core/Cargo.toml", "-p", "ctx-http", "--bin", "ctx"], {
    cwd: REPO_ROOT,
    env,
  });
  runChecked("cargo", ["build", "--manifest-path", "core/Cargo.toml", "-p", "ctx-mcp"], {
    cwd: REPO_ROOT,
    env,
  });
}

function ensurePlaybackWebDist() {
  if (existsSync(path.join(REPO_ROOT, "core/apps/web/dist"))) {
    return;
  }
  runChecked("pnpm", ["-C", "core/apps/web", "build"], {
    cwd: REPO_ROOT,
    env: process.env,
  });
}

function ensureCnApiKey() {
  const existing = String(process.env.CN_API_KEY || "").trim();
  if (existing) {
    return existing;
  }
  const result = runChecked("infisical", ["export", "--format=dotenv"], {
    cwd: path.join(REPO_ROOT, "core"),
    env: process.env,
  });
  const cnApiKey = extractDotenvVariable(result.stdout, "CN_API_KEY");
  if (!cnApiKey) {
    throw new Error("CN_API_KEY is required for local macOS desktop automation; Infisical export did not provide it.");
  }
  return cnApiKey;
}

function listWindows() {
  const result = runChecked(SCREEN_STUDIO_LIST_WINDOWS_PATH, []);
  return JSON.parse(String(result.stdout || "[]"));
}

async function waitForCaptureWindow(expectedTitle, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      return findScreenStudioCaptureWindow(listWindows(), expectedTitle);
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 300));
    }
  }
  throw new Error(`timed out waiting for capture window titled "${expectedTitle}"`);
}

function startPolyrecorder(config) {
  const proc = spawn(SCREEN_STUDIO_POLYRECORDER_PATH, ["record"], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const prepared = createDeferred();
  const resumed = createDeferred();
  const exited = createDeferred();
  const rl = readline.createInterface({ input: proc.stdout });
  proc.stderr.setEncoding("utf8");
  proc.stderr.on("data", (chunk) => process.stderr.write(chunk));
  rl.on("line", (line) => {
    process.stdout.write(`${line}\n`);
    if (!line.trim().startsWith("{")) {
      return;
    }
    try {
      const event = JSON.parse(line);
      if (event.type === "recordingPrepared") {
        prepared.resolve(event);
      } else if (event.type === "recordingResumed") {
        resumed.resolve(event);
      }
    } catch {
      // ignore non-JSON log lines
    }
  });
  proc.once("error", (error) => {
    prepared.reject(error);
    resumed.reject(error);
    exited.reject(error);
  });
  proc.once("exit", (code, signal) => {
    rl.close();
    exited.resolve({ code, signal });
  });
  proc.stdin.end(JSON.stringify(config));
  return {
    proc,
    prepared: prepared.promise,
    resumed: resumed.promise,
    exited: exited.promise,
    async start() {
      await prepared.promise;
      proc.kill("SIGCONT");
      return resumed.promise;
    },
    async stop() {
      proc.kill("SIGTERM");
      return exited.promise;
    },
  };
}

function materializeProjectBundle({ outputProjectPath, outputTitle, rawRecordingDir }) {
  const metadataPath = path.join(rawRecordingDir, "metadata.json");
  if (!existsSync(metadataPath)) {
    throw new Error(`missing Screen Studio recording metadata at ${metadataPath}`);
  }
  const recordingMetadata = JSON.parse(readFileSync(metadataPath, "utf8"));
  const durationMs = Number(recordingMetadata?.sessions?.[0]?.durationMs);
  if (!Number.isFinite(durationMs) || durationMs <= 0) {
    throw new Error(`invalid Screen Studio recording duration in ${metadataPath}`);
  }
  rmSync(outputProjectPath, { recursive: true, force: true });
  mkdirSync(outputProjectPath, { recursive: true });
  const recordingTargetPath = path.join(outputProjectPath, "recording");
  renameSync(rawRecordingDir, recordingTargetPath);
  const bundle = buildScreenStudioProjectBundle({
    projectName: outputTitle,
    durationMs,
  });
  writeJson(path.join(outputProjectPath, "project.json"), bundle.project);
  writeJson(path.join(outputProjectPath, "meta.json"), bundle.meta);
  writeJson(path.join(outputProjectPath, "recording-markers.json"), bundle.markers);
  return {
    durationMs,
    recordingMetadataPath: path.join(recordingTargetPath, "metadata.json"),
  };
}

function openProjectInScreenStudio(projectPath) {
  spawnSync("osascript", ["-e", 'tell application "Screen Studio" to quit'], { stdio: "ignore" });
  const result = spawnSync("open", ["-n", "-a", SCREEN_STUDIO_APP_PATH, projectPath], {
    stdio: "pipe",
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(`failed to open Screen Studio project: ${String(result.stderr || "").trim()}`);
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  mkdirSync(path.dirname(options.outputProjectPath), { recursive: true });
  mkdirSync(path.dirname(options.rawRecordingDir), { recursive: true });
  rmSync(options.rawRecordingDir, { recursive: true, force: true });
  options.cnApiKey = ensureCnApiKey();
  ensurePlaybackSidecars(options.tauriTargetDir);
  ensurePlaybackWebDist();

  const playbackProc = startPlaybackProcess(options);
  const playbackExitPromise = waitForProcessExit(playbackProc, "HN mobile playback");
  try {
    await waitForPath(path.join(options.artifactDir, SIDEBAR_READY_FILENAME), options.sidebarReadyTimeoutMs);
    const displayId = discoverMainDisplayId();
    const captureWindow = await waitForCaptureWindow(options.windowTitle, 20_000);
    writeJson(path.join(options.artifactDir, "screenstudio-capture-window.json"), {
      display_id: displayId,
      window: captureWindow,
    });
    const recorder = startPolyrecorder(
      buildScreenStudioRecordConfig({
        outputDirectory: options.rawRecordingDir,
        displayId,
        cropRect: captureWindow.bounds,
      }),
    );
    await recorder.start();
    await playbackExitPromise;
    await recorder.stop();
    const packaged = materializeProjectBundle({
      outputProjectPath: options.outputProjectPath,
      outputTitle: options.outputTitle,
      rawRecordingDir: options.rawRecordingDir,
    });
    if (options.openProject) {
      openProjectInScreenStudio(options.outputProjectPath);
    }
    const result = {
      status: "ok",
      artifact_dir: options.artifactDir,
      output_project_path: options.outputProjectPath,
      duration_ms: packaged.durationMs,
      recording_metadata_path: packaged.recordingMetadataPath,
    };
    writeJson(path.join(options.artifactDir, "screenstudio-result.json"), result);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } finally {
    if (playbackProc.exitCode === null) {
      playbackProc.kill("SIGTERM");
    }
  }
}

if (import.meta.main) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.stack || error.message : String(error)}\n`);
    process.exitCode = 1;
  });
}
