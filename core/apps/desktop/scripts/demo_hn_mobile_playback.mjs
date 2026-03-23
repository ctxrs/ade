#!/usr/bin/env node
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

import {
  buildAutomationAppIfNeeded,
  captureArtifactsPaneState,
  captureDiffPaneState,
  openArtifactsPane,
  openDiffPane,
  prepareAutomationAppForLaunch,
  primeDemoDesktopConnectionToWorkspace,
  readTokenFromManifest,
  startCrabNebulaStack,
  startSetupProcess,
  submitComposerPrompt,
  terminateAutomationAppProcesses,
  waitForArtifactsPane,
  waitForDiffPane,
  connectBrowser,
} from "./demo_ping_pong_playback.mjs";
import { api, sleep, waitFor } from "./demo_lib.mjs";

const REPO_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../..");
const DEMO_APP_PRODUCT_NAME = "ctx-demo";
const DEFAULT_PROMPT = "Add saved pages and attach a short recording.";
const DEFAULT_SESSION_ARTIFACT_PATH = path.resolve(REPO_ROOT, "core/apps/desktop/automation/fixtures/demo-artifacts/hn-mobile-saved-stories.mp4");
const DEFAULT_EXPECTED_FINAL_APP_PATH = path.resolve(
  REPO_ROOT,
  "core/apps/desktop/automation/fixtures/demo-workspaces/hn-mobile-saved-stories/src/app.js",
);
const PROMPT_CHARACTER_DELAY_MS = 10;
const HARNESS_MENU_DWELL_MS = 460;
const HARNESS_SELECTED_DWELL_MS = 280;
const POST_HARNESS_DWELL_MS = 120;
const POST_PROMPT_DWELL_MS = 100;
const RESPONSE_PRE_DIFF_DWELL_MS = 900;
const DIFF_PANE_DWELL_MS = 2400;
const POST_ARTIFACT_ATTACH_DWELL_MS = 250;
const ARTIFACT_PANE_DWELL_MS = 2600;

function runChecked(command, args, options = {}) {
  const result = spawnSync(command, args, {
    stdio: "inherit",
    ...options,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed with status ${result.status}`);
  }
}

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

function parseArgs(argv) {
  const stamp = runStamp();
  const artifactDir = `/tmp/ctx-demo-hn-mobile-${stamp}`;
  const tauriTargetDir = `/tmp/ctx-demo-desktop-target-hn-mobile-${stamp}`;
  const options = {
    artifactDir,
    workspaceRoot: `/tmp/ctx-demo-hn-mobile-workspace-${stamp}`,
    daemonDataDir: `/tmp/ctx-demo-daemon-hn-mobile-${stamp}`,
    tauriTargetDir,
    appPath: path.join(tauriTargetDir, `debug/bundle/macos/${DEMO_APP_PRODUCT_NAME}.app`),
    promptText: null,
    skipBuild: false,
    keepAlive: false,
    backendPort: pickUnusedPortSync(3000),
    driverPort: pickUnusedPortSync(4451),
    daemonPort: pickUnusedPortSync(4416),
    harnessLabel: "Codex",
    fixturePath: path.resolve(REPO_ROOT, "core/apps/desktop/automation/fixtures/demo-hn-mobile-fixture.json"),
    relayScenarioPath: path.resolve(REPO_ROOT, "core/apps/desktop/automation/fixtures/demo-relay/codex-hn-mobile.replay.json"),
    sessionArtifactPath: null,
    expectedFinalAppPath: DEFAULT_EXPECTED_FINAL_APP_PATH,
    recordVideoOut: null,
    recordFps: 12,
    captureMilestones: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--artifact-dir") {
      options.artifactDir = path.resolve(next);
      index += 1;
    } else if (arg === "--workspace-root") {
      options.workspaceRoot = path.resolve(next);
      index += 1;
    } else if (arg === "--daemon-data-dir") {
      options.daemonDataDir = path.resolve(next);
      index += 1;
    } else if (arg === "--tauri-target-dir") {
      options.tauriTargetDir = path.resolve(next);
      options.appPath = path.join(options.tauriTargetDir, `debug/bundle/macos/${DEMO_APP_PRODUCT_NAME}.app`);
      index += 1;
    } else if (arg === "--app-path") {
      options.appPath = path.resolve(next);
      index += 1;
    } else if (arg === "--prompt") {
      options.promptText = next;
      index += 1;
    } else if (arg === "--skip-build") {
      options.skipBuild = true;
    } else if (arg === "--keep-alive") {
      options.keepAlive = true;
    } else if (arg === "--backend-port") {
      options.backendPort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--driver-port") {
      options.driverPort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--daemon-port") {
      options.daemonPort = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--fixture") {
      options.fixturePath = path.resolve(next);
      index += 1;
    } else if (arg === "--relay-scenario") {
      options.relayScenarioPath = path.resolve(next);
      index += 1;
    } else if (arg === "--session-artifact") {
      options.sessionArtifactPath = path.resolve(next);
      index += 1;
    } else if (arg === "--expected-final-app") {
      options.expectedFinalAppPath = path.resolve(next);
      index += 1;
    } else if (arg === "--harness-label") {
      options.harnessLabel = next;
      index += 1;
    } else if (arg === "--record-video-out") {
      options.recordVideoOut = path.resolve(next);
      index += 1;
    } else if (arg === "--record-fps") {
      options.recordFps = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--capture-milestones") {
      options.captureMilestones = true;
    }
  }
  return options;
}

function resolveFixturePath(fixturePath, maybeRelativePath) {
  if (!maybeRelativePath) {
    return null;
  }
  if (path.isAbsolute(maybeRelativePath)) {
    return maybeRelativePath;
  }
  return path.resolve(path.dirname(fixturePath), maybeRelativePath);
}

export function resolvePlaybackPrompt(fixture, explicitPrompt) {
  return explicitPrompt || fixture.next_prompt || DEFAULT_PROMPT;
}

export function resolvePlaybackSessionArtifactPath(fixturePath, fixture, explicitArtifactPath) {
  return explicitArtifactPath || resolveFixturePath(fixturePath, fixture.session_artifact_path) || DEFAULT_SESSION_ARTIFACT_PATH;
}

function writeJson(pathname, value) {
  writeFileSync(pathname, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

async function captureMilestone(browser, artifactDir, startedAt, milestoneName) {
  const pngBase64 = await browser.takeScreenshot();
  const screenshotPath = path.join(artifactDir, `milestone-${milestoneName}.png`);
  writeFileSync(screenshotPath, Buffer.from(pngBase64, "base64"));
  const summary = {
    name: milestoneName,
    relative_ms: Date.now() - startedAt,
    screenshot_path: screenshotPath,
  };
  writeJson(path.join(artifactDir, `milestone-${milestoneName}.json`), summary);
  return summary;
}

export function buildPromptCharacters(promptText) {
  return Array.from(String(promptText ?? ""));
}

function frameFilePath(frameDir, frameIndex) {
  return path.join(frameDir, `frame-${String(frameIndex).padStart(5, "0")}.png`);
}

export function buildFrameSequenceFfmpegArgs(frameDir, fps, outputPath) {
  return [
    "-y",
    "-framerate", String(fps),
    "-i", path.join(frameDir, "frame-%05d.png"),
    "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
    "-c:v", "libx264",
    "-pix_fmt", "yuv420p",
    outputPath,
  ];
}

function startBrowserFrameCapture(browser, artifactDir, fps) {
  const frameDir = path.join(artifactDir, "recording-frames");
  mkdirSync(frameDir, { recursive: true });
  const frameIntervalMs = Math.max(50, Math.round(1000 / Math.max(1, fps)));
  const startedAt = Date.now();
  let stopRequested = false;
  let frameCount = 0;
  let captureError = null;

  const captureLoop = (async () => {
    while (!stopRequested) {
      const startedAt = Date.now();
      try {
        const pngBase64 = await browser.takeScreenshot();
        writeFileSync(frameFilePath(frameDir, frameCount), Buffer.from(pngBase64, "base64"));
        frameCount += 1;
      } catch (error) {
        if (!stopRequested) {
          captureError = error;
        }
        break;
      }
      const elapsedMs = Date.now() - startedAt;
      if (elapsedMs < frameIntervalMs) {
        await sleep(frameIntervalMs - elapsedMs);
      }
    }
  })();

  return {
    async stop() {
      stopRequested = true;
      await captureLoop;
      if (captureError) {
        throw captureError;
      }
      return {
        frameDir,
        frameCount,
        fps,
        capture_duration_ms: Math.max(1, Date.now() - startedAt),
      };
    },
  };
}

async function ensureNewTaskVisible(browser, artifactDir) {
  try {
    await browser.waitUntil(
      async () =>
        await browser.execute(
          () => Boolean(document.querySelector("textarea.wb-new-composer-textarea") && document.querySelector(".wb-switcher-harness")),
        ),
      {
        timeout: 30_000,
        timeoutMsg: "new task composer did not become visible",
      },
    );
  } catch (error) {
    writeJson(path.join(artifactDir, "new-task-visible-failure.json"), { error: String(error?.stack || error) });
    throw error;
  }
}

async function focusNewTask(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(
        () => Boolean(globalThis.__ctxE2E && typeof globalThis.__ctxE2E.focusNewTask === "function"),
      ),
    {
      timeout: 30_000,
      timeoutMsg: "ctxE2E focusNewTask bridge did not become available",
    },
  );
  await browser.execute(() => {
    const focusNewTaskBridge = globalThis.__ctxE2E?.focusNewTask;
    if (typeof focusNewTaskBridge !== "function") {
      throw new Error("ctxE2E focusNewTask bridge unavailable");
    }
    focusNewTaskBridge();
  });
}

async function clearDraftHarness(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(
        () => Boolean(globalThis.__ctxE2E && typeof globalThis.__ctxE2E.clearDraftHarness === "function"),
      ),
    {
      timeout: 30_000,
      timeoutMsg: "ctxE2E clearDraftHarness bridge did not become available",
    },
  );
  await browser.execute(() => {
    const clearDraftHarnessBridge = globalThis.__ctxE2E?.clearDraftHarness;
    if (typeof clearDraftHarnessBridge !== "function") {
      throw new Error("ctxE2E clearDraftHarness bridge unavailable");
    }
    clearDraftHarnessBridge();
  });
  await browser.waitUntil(
    async () =>
      await browser.execute(() => {
        const trigger = document.querySelector(".wb-switcher-harness");
        return trigger?.textContent?.includes("Select agent") ?? false;
      }),
    {
      timeout: 30_000,
      timeoutMsg: "new task harness selector did not reset to Select agent",
    },
  );
}

async function selectHarness(browser, harnessLabel) {
  await browser.execute(() => {
    const trigger = document.querySelector(".wb-switcher-harness");
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("harness trigger not found");
    }
    trigger.click();
  });
  await browser.waitUntil(
    async () =>
      await browser.execute(
        (label) => Array.from(document.querySelectorAll(".wb-harness-row-main")).some((element) => element.textContent?.includes(label)),
        harnessLabel,
      ),
    {
      timeout: 10_000,
      timeoutMsg: `harness menu item ${harnessLabel} did not appear`,
    },
  );
  await sleep(HARNESS_MENU_DWELL_MS);
  await browser.execute((label) => {
    const target = Array.from(document.querySelectorAll(".wb-harness-row-main")).find((element) =>
      element.textContent?.includes(label));
    if (!(target instanceof HTMLButtonElement)) {
      throw new Error(`harness row not found for ${label}`);
    }
    target.click();
  }, harnessLabel);
  await sleep(HARNESS_SELECTED_DWELL_MS);
}

async function setComposerValue(browser, value) {
  return browser.execute((nextValue) => {
    const textarea = document.querySelector("textarea.wb-new-composer-textarea");
    if (!(textarea instanceof HTMLTextAreaElement)) {
      throw new Error("new task composer textarea not found");
    }
    const setter = Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype, "value")?.set;
    if (typeof setter !== "function") {
      throw new Error("textarea value setter unavailable");
    }
    setter.call(textarea, nextValue);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    textarea.dispatchEvent(new Event("change", { bubbles: true }));
    return textarea.value;
  }, value);
}

async function typePromptCharacterByCharacter(browser, promptText, options = {}) {
  const characters = buildPromptCharacters(promptText);
  let currentValue = "";
  for (const character of characters) {
    currentValue += character;
    await setComposerValue(browser, currentValue);
    await sleep(options.characterDelayMs ?? PROMPT_CHARACTER_DELAY_MS);
  }
  return currentValue;
}

async function openDiffFile(browser, filePath) {
  await browser.waitUntil(
    async () =>
      await browser.execute((targetPath) => {
        const diffFile = Array.from(document.querySelectorAll(".cursor-diff-file")).find((element) => {
          const filePathElement = element.querySelector(".cursor-diff-file-path");
          return filePathElement?.textContent?.trim() === targetPath;
        });
        if (!diffFile) {
          return false;
        }
        const body = diffFile.querySelector(".cursor-diff-file-body");
        if (body instanceof HTMLElement) {
          body.scrollIntoView({ block: "center", inline: "nearest" });
          return true;
        }
        const trigger = diffFile.querySelector(".cursor-diff-chevron");
        if (!(trigger instanceof HTMLButtonElement)) {
          return false;
        }
        trigger.click();
        return false;
      }, filePath),
    {
      timeout: 30_000,
      timeoutMsg: `diff file ${filePath} did not open`,
    },
  );
}

async function captureOpenedDiffFileState(browser, filePath) {
  return browser.execute((targetPath) => {
    const diffFile = Array.from(document.querySelectorAll(".cursor-diff-file")).find((element) => {
      const filePathElement = element.querySelector(".cursor-diff-file-path");
      return filePathElement?.textContent?.trim() === targetPath;
    });
    const body = diffFile?.querySelector(".cursor-diff-file-body");
    return {
      target_path: targetPath,
      has_file_row: Boolean(diffFile),
      has_open_body: body instanceof HTMLElement,
    };
  }, filePath);
}

export function findNewTaskRecord(tasks, knownTaskIds = []) {
  if (!Array.isArray(tasks)) {
    return null;
  }
  const known = knownTaskIds instanceof Set ? knownTaskIds : new Set(knownTaskIds);
  return tasks.find((task) => task && typeof task.id === "string" && !known.has(task.id)) || null;
}

async function waitForCreatedTask(baseUrl, token, workspaceId, knownTaskIds = []) {
  return waitFor(async () => {
    const tasks = await api(baseUrl, token, "GET", `/api/workspaces/${workspaceId}/tasks`);
    return findNewTaskRecord(tasks, knownTaskIds);
  }, {
    timeoutMs: 60_000,
    intervalMs: 1_000,
    label: `task creation in workspace ${workspaceId}`,
  });
}

async function waitForCreatedSession(baseUrl, token, taskId) {
  return waitFor(async () => {
    const sessions = await api(baseUrl, token, "GET", `/api/tasks/${taskId}/sessions`);
    if (Array.isArray(sessions) && sessions.length > 0) {
      return sessions[0];
    }
    return null;
  }, {
    timeoutMs: 60_000,
    intervalMs: 1_000,
    label: `session creation for task ${taskId}`,
  });
}

export function resolveTaskWorktreeRoot(daemonDataDir, workspaceId, taskRecord) {
  const primaryWorktreeId = typeof taskRecord?.primary_worktree_id === "string"
    ? taskRecord.primary_worktree_id.trim()
    : "";
  if (!primaryWorktreeId) {
    return null;
  }
  return path.join(daemonDataDir, "worktrees", workspaceId, primaryWorktreeId);
}

async function waitForTaskWorktreeRoot(baseUrl, token, daemonDataDir, workspaceId, taskId) {
  return waitFor(async () => {
    const tasks = await api(baseUrl, token, "GET", `/api/workspaces/${workspaceId}/tasks`);
    if (!Array.isArray(tasks)) {
      return null;
    }
    const taskRecord = tasks.find((entry) => entry?.id === taskId);
    return resolveTaskWorktreeRoot(daemonDataDir, workspaceId, taskRecord);
  }, {
    timeoutMs: 60_000,
    intervalMs: 1_000,
    label: `task ${taskId} worktree root`,
  });
}

async function waitForWorkspaceDiff(worktreeRoot) {
  return waitFor(async () => {
    const result = spawnSync("git", ["status", "--short"], {
      cwd: worktreeRoot,
      encoding: "utf8",
    });
    if (result.status !== 0) {
      return null;
    }
    const output = String(result.stdout || "").trim();
    return output ? output : null;
  }, {
    timeoutMs: 60_000,
    intervalMs: 1_000,
    label: `workspace diff in ${worktreeRoot}`,
  });
}

function seedExpectedWorkspaceDiff(worktreeRoot, expectedFinalAppPath) {
  if (!existsSync(expectedFinalAppPath)) {
    throw new Error(`expected final app file does not exist: ${expectedFinalAppPath}`);
  }
  copyFileSync(expectedFinalAppPath, path.join(worktreeRoot, "src/app.js"));
}

async function waitForArtifactVideoPlayback(browser) {
  await browser.waitUntil(async () => {
    const state = await captureArtifactsPaneState(browser);
    return state.playingVideoCount > 0 && state.firstVideoCurrentTime > 0;
  }, {
    timeout: 30_000,
    timeoutMsg: "artifact video preview did not start playing in time",
  });
}

async function attachSessionArtifact(baseUrl, token, sessionId, artifactPath) {
  if (!artifactPath) return null;
  if (!existsSync(artifactPath)) {
    throw new Error(`session artifact does not exist: ${artifactPath}`);
  }
  return api(baseUrl, token, "POST", `/api/sessions/${sessionId}/artifacts`, {
    artifacts: [
      {
        absolute_file_path: artifactPath,
        name: path.basename(artifactPath),
        mime_type: "video/mp4",
      },
    ],
  });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const fixture = JSON.parse(readFileSync(options.fixturePath, "utf8"));
  options.promptText = resolvePlaybackPrompt(fixture, options.promptText);
  options.sessionArtifactPath = resolvePlaybackSessionArtifactPath(options.fixturePath, fixture, options.sessionArtifactPath);
  mkdirSync(options.artifactDir, { recursive: true });
  mkdirSync(options.workspaceRoot, { recursive: true });

  let browser = null;
  let backendProc = null;
  let driverProc = null;
  let relayServer = null;
  let daemonProc = null;
  let videoCapture = null;
  let recordingStartedAt = 0;

  try {
    const setup = await startSetupProcess(options);
    relayServer = setup.relay.server;
    daemonProc = setup.daemon.proc;
    const setupManifest = setup.manifest;
    const authToken = readTokenFromManifest(setupManifest);
    const baseUrl = setupManifest.daemon.url;
    const workspaceId = setupManifest.fixture.workspace_id;
    await api(baseUrl, authToken, "POST", `/api/workspaces/${workspaceId}/providers/codex/verify`, {});

    buildAutomationAppIfNeeded(options.appPath, options.skipBuild, options.tauriTargetDir);
    const launchAppPath = prepareAutomationAppForLaunch(options.appPath, options.artifactDir);
    await terminateAutomationAppProcesses(launchAppPath);

    const cn = await startCrabNebulaStack({
      artifactDir: options.artifactDir,
      backendPort: options.backendPort,
      driverPort: options.driverPort,
      appEnv: {
        CTX_DESKTOP_DAEMON_URL: setupManifest.daemon.url,
        CTX_DESKTOP_DAEMON_TOKEN: authToken,
        CTX_DESKTOP_ALLOW_DEMO_COMMANDS: "1",
        CTX_DESKTOP_START_PATH: `/workspaces/${encodeURIComponent(workspaceId)}?ctxE2E=1&ctxDemoManualHarness=1`,
        CTX_AUTOMATION_WORKSPACE_PATH: options.workspaceRoot,
        CTX_BUNDLE_DIR: path.join(options.tauriTargetDir, "debug", "bundles"),
        CTX_DESKTOP_DEV_BIN_DIR: path.join(options.tauriTargetDir, "debug"),
      },
    });
    backendProc = cn.backendProc;
    driverProc = cn.driverProc;
    browser = await connectBrowser({ driverPort: options.driverPort, appPath: launchAppPath });

    await primeDemoDesktopConnectionToWorkspace(browser, baseUrl, authToken, workspaceId);
    await focusNewTask(browser);
    await ensureNewTaskVisible(browser, options.artifactDir);
    await clearDraftHarness(browser);
    if (options.recordVideoOut) {
      recordingStartedAt = Date.now();
      videoCapture = startBrowserFrameCapture(browser, options.artifactDir, options.recordFps);
      if (options.captureMilestones) {
        await captureMilestone(browser, options.artifactDir, recordingStartedAt, "new-task-ready");
      }
    }
    await sleep(450);

    await selectHarness(browser, options.harnessLabel);
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "harness-selected");
    }
    await sleep(POST_HARNESS_DWELL_MS);

    const promptScenarioPath = path.join(options.artifactDir, "prompt-scenario.json");
    writeJson(promptScenarioPath, {
      mode: "browser-character-input",
      prompt: options.promptText,
      characters: buildPromptCharacters(options.promptText),
      character_delay_ms: PROMPT_CHARACTER_DELAY_MS,
    });
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "prompt-typing-start");
    }
    const typedPrompt = await typePromptCharacterByCharacter(browser, options.promptText);
    writeJson(path.join(options.artifactDir, "prompt-typed-state.json"), { value: typedPrompt });
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "prompt-typed");
    }
    await sleep(POST_PROMPT_DWELL_MS);
    const existingTasks = await api(baseUrl, authToken, "GET", `/api/workspaces/${workspaceId}/tasks`);
    const knownTaskIds = Array.isArray(existingTasks)
      ? new Set(existingTasks.map((task) => String(task?.id || "")).filter(Boolean))
      : new Set();
    await submitComposerPrompt(browser, options.promptText);

    const task = await waitForCreatedTask(baseUrl, authToken, workspaceId, knownTaskIds);
    const session = await waitForCreatedSession(baseUrl, authToken, task.id);
    const worktreeRoot = await waitForTaskWorktreeRoot(
      baseUrl,
      authToken,
      setupManifest.daemon.data_dir,
      workspaceId,
      task.id,
    );
    await sleep(RESPONSE_PRE_DIFF_DWELL_MS);
    seedExpectedWorkspaceDiff(worktreeRoot, options.expectedFinalAppPath);
    const workspaceDiff = await waitForWorkspaceDiff(worktreeRoot);
    writeJson(path.join(options.artifactDir, "workspace-diff-detected.json"), {
      worktree_root: worktreeRoot,
      status: workspaceDiff,
    });
    await sleep(350);

    const diffToggleResult = await openDiffPane(browser);
    writeJson(path.join(options.artifactDir, "diff-toggle-result.json"), diffToggleResult);
    await waitForDiffPane(browser);
    await openDiffFile(browser, "src/app.js");
    writeJson(path.join(options.artifactDir, "diff-pane-state.json"), await captureDiffPaneState(browser));
    writeJson(path.join(options.artifactDir, "opened-diff-file-state.json"), await captureOpenedDiffFileState(browser, "src/app.js"));
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "diff-file-open");
    }
    await sleep(DIFF_PANE_DWELL_MS);

    const attachedArtifacts = await attachSessionArtifact(baseUrl, authToken, session.id, options.sessionArtifactPath);
    writeJson(path.join(options.artifactDir, "attached-artifacts.json"), attachedArtifacts);
    await sleep(POST_ARTIFACT_ATTACH_DWELL_MS);

    const artifactsToggleResult = await openArtifactsPane(browser);
    writeJson(path.join(options.artifactDir, "artifacts-toggle-result.json"), artifactsToggleResult);
    await waitForArtifactsPane(browser);
    await waitForArtifactVideoPlayback(browser);
    writeJson(path.join(options.artifactDir, "artifacts-pane-state.json"), await captureArtifactsPaneState(browser));
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "artifact-video-playing");
    }
    if (options.recordVideoOut) {
      await sleep(ARTIFACT_PANE_DWELL_MS);
      const captureSummary = await videoCapture.stop();
      videoCapture = null;
      mkdirSync(path.dirname(options.recordVideoOut), { recursive: true });
      const effectiveFps = Math.max(
        1,
        Number((captureSummary.frameCount / (captureSummary.capture_duration_ms / 1000)).toFixed(3)),
      );
      runChecked("ffmpeg", buildFrameSequenceFfmpegArgs(captureSummary.frameDir, effectiveFps, options.recordVideoOut));
      writeJson(path.join(options.artifactDir, "recorded-video.json"), {
        output_path: options.recordVideoOut,
        frame_count: captureSummary.frameCount,
        requested_fps: captureSummary.fps,
        encoded_fps: effectiveFps,
        capture_duration_ms: captureSummary.capture_duration_ms,
      });
    }

    const result = {
      status: "ok",
      artifact_dir: options.artifactDir,
      workspace_root: options.workspaceRoot,
      workspace_id: workspaceId,
      task_id: task.id,
      session_id: session.id,
      daemon_url: baseUrl,
      relay_port: setupManifest.relay.port,
      app_path: options.appPath,
      prompt_scenario_path: promptScenarioPath,
      recorded_video_path: options.recordVideoOut,
    };
    writeJson(path.join(options.artifactDir, "playback-result.json"), result);
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);

    if (options.keepAlive) {
      await new Promise(() => {});
    }
  } finally {
    if (browser) {
      try {
        await browser.deleteSession();
      } catch {
        // ignore
      }
    }
    if (videoCapture) {
      try {
        await videoCapture.stop();
      } catch {
        // ignore
      }
    }
    if (driverProc) driverProc.kill("SIGTERM");
    if (backendProc) backendProc.kill("SIGTERM");
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
