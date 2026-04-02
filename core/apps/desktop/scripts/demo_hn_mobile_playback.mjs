#!/usr/bin/env node
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";

import {
  buildAutomationAppIfNeeded,
  captureArtifactsPaneState,
  captureDiffPaneState,
  ensureWorkbenchVisible,
  installVisibleHarnessProvidersAndWait,
  measureTargets,
  prepareAutomationAppForLaunch,
  primeDemoDesktopConnectionToWorkspace,
  readTokenFromManifest,
  runConductor,
  setArtifactsPanePlaybackHidden,
  setDiffPanePlaybackHidden,
  startCrabNebulaStack,
  startSetupProcess,
  terminateAutomationAppProcesses,
  waitForArtifactsPane,
  waitForDiffPane,
  waitForDiffPaneReady,
  connectBrowser,
} from "./demo_ping_pong_playback.mjs";
import { api, installProviderAndWait, sleep, waitFor } from "./demo_lib.mjs";
import { inferVideoArtifactMimeType } from "./demo_video_artifacts.mjs";
import { buildClickScenario, buildPromptScenario } from "./demo_desktop_probe.mjs";

const REPO_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "../../../..");
const DEMO_APP_PRODUCT_NAME = "ctx-demo";
const DEFAULT_PROMPT = "Build a Hacker News iOS app as a Tauri wrapper. Add a new muted domains feature in Settings, then record a short demo video.";
const DEFAULT_SESSION_ARTIFACT_PATH = path.resolve(
  REPO_ROOT,
  "core/apps/desktop/automation/fixtures/demo-artifacts/hn-muted-domains.mp4",
);
const DEFAULT_EXPECTED_FINAL_WORKSPACE_PATH = path.resolve(
  REPO_ROOT,
  "core/apps/desktop/automation/fixtures/demo-workspaces/hn-mobile-muted-domains",
);
const DEFAULT_DIFF_FILE_PATH = "src/hn_enhancer.js";
const DEFAULT_SECONDARY_DIFF_FILE_PATH = null;
const EXPECTED_FINAL_WORKSPACE_DIFF_FILES = Object.freeze([
  "src/app.js",
  "src/style.css",
  "src/hn_enhancer.js",
  "src/viewport.js",
  "vite.config.js",
  "hn_proxy.js",
]);
const HARNESS_LABEL_TO_PROVIDER_ID = Object.freeze({
  "Claude Code": "claude-crp",
  Codex: "codex",
  "Qwen Code": "qwen",
  Cursor: "cursor",
  Pi: "pi",
  Amp: "amp",
  Droid: "droid",
  Gemini: "gemini",
  Goose: "goose",
  Copilot: "copilot",
  OpenCode: "opencode",
  OpenHands: "openhands",
  Cline: "cline",
  "Mistral Vibe": "mistral",
  Auggie: "auggie",
  Kimi: "kimi",
});
const PROMPT_CHARACTER_DELAY_MS = 10;
const DEFAULT_HARNESS_MENU_DWELL_MS = 3000;
const HARNESS_SELECTED_DWELL_MS = 280;
const POST_HARNESS_DWELL_MS = 120;
const POST_PROMPT_DWELL_MS = 100;
const RESPONSE_PRE_DIFF_DWELL_MS = 900;
const DIFF_FILE_LIST_DWELL_MS = 900;
const PRIMARY_DIFF_PANE_DWELL_MS = 1700;
const POST_ARTIFACT_ATTACH_DWELL_MS = 250;
const APP_FRONTMOST_SETTLE_MS = 250;
const SIDEBAR_READY_STABLE_POLLS = 3;
const DEFAULT_RECORD_START_DELAY_MS = 0;
const DEFAULT_MOUSE_TAKEOVER_LEAD_MS = 2000;
const POST_ARTIFACT_VIDEO_COMPLETION_DWELL_MS = 1000;
let logicalCursorPoint = null;
const DEFAULT_MOUSE_TIMINGS = Object.freeze({
  neutralMoveMs: 240,
  neutralWaitMs: 120,
  harnessTriggerMoveMs: 420,
  harnessOptionMoveMs: 360,
  composerMoveMs: 420,
  promptTypingCps: 14,
  promptWaitMs: 260,
  sendButtonMoveMs: 320,
  sendButtonWaitMs: 140,
  diffToggleMoveMs: 340,
  diffToggleWaitMs: 120,
  diffFileMoveMs: 420,
  diffFileWaitMs: 80,
  artifactsToggleMoveMs: 360,
  artifactsToggleWaitMs: 140,
  artifactPlayMoveMs: 520,
  artifactPlayWaitMs: 160,
  artifactExitMoveMs: 360,
  artifactExitWaitMs: 140,
});

function parsePositiveInt(value, fallback) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function parsePositiveNumber(value, fallback) {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

export function buildMouseTimings(overrides = {}) {
  return {
    neutralMoveMs: parsePositiveInt(overrides.neutralMoveMs, DEFAULT_MOUSE_TIMINGS.neutralMoveMs),
    neutralWaitMs: parsePositiveInt(overrides.neutralWaitMs, DEFAULT_MOUSE_TIMINGS.neutralWaitMs),
    harnessTriggerMoveMs: parsePositiveInt(overrides.harnessTriggerMoveMs, DEFAULT_MOUSE_TIMINGS.harnessTriggerMoveMs),
    harnessOptionMoveMs: parsePositiveInt(overrides.harnessOptionMoveMs, DEFAULT_MOUSE_TIMINGS.harnessOptionMoveMs),
    composerMoveMs: parsePositiveInt(overrides.composerMoveMs, DEFAULT_MOUSE_TIMINGS.composerMoveMs),
    promptTypingCps: parsePositiveNumber(overrides.promptTypingCps, DEFAULT_MOUSE_TIMINGS.promptTypingCps),
    promptWaitMs: parsePositiveInt(overrides.promptWaitMs, DEFAULT_MOUSE_TIMINGS.promptWaitMs),
    sendButtonMoveMs: parsePositiveInt(overrides.sendButtonMoveMs, DEFAULT_MOUSE_TIMINGS.sendButtonMoveMs),
    sendButtonWaitMs: parsePositiveInt(overrides.sendButtonWaitMs, DEFAULT_MOUSE_TIMINGS.sendButtonWaitMs),
    diffToggleMoveMs: parsePositiveInt(overrides.diffToggleMoveMs, DEFAULT_MOUSE_TIMINGS.diffToggleMoveMs),
    diffToggleWaitMs: parsePositiveInt(overrides.diffToggleWaitMs, DEFAULT_MOUSE_TIMINGS.diffToggleWaitMs),
    diffFileMoveMs: parsePositiveInt(overrides.diffFileMoveMs, DEFAULT_MOUSE_TIMINGS.diffFileMoveMs),
    diffFileWaitMs: parsePositiveInt(overrides.diffFileWaitMs, DEFAULT_MOUSE_TIMINGS.diffFileWaitMs),
    artifactsToggleMoveMs: parsePositiveInt(overrides.artifactsToggleMoveMs, DEFAULT_MOUSE_TIMINGS.artifactsToggleMoveMs),
    artifactsToggleWaitMs: parsePositiveInt(overrides.artifactsToggleWaitMs, DEFAULT_MOUSE_TIMINGS.artifactsToggleWaitMs),
    artifactPlayMoveMs: parsePositiveInt(overrides.artifactPlayMoveMs, DEFAULT_MOUSE_TIMINGS.artifactPlayMoveMs),
    artifactPlayWaitMs: parsePositiveInt(overrides.artifactPlayWaitMs, DEFAULT_MOUSE_TIMINGS.artifactPlayWaitMs),
    artifactExitMoveMs: parsePositiveInt(overrides.artifactExitMoveMs, DEFAULT_MOUSE_TIMINGS.artifactExitMoveMs),
    artifactExitWaitMs: parsePositiveInt(overrides.artifactExitWaitMs, DEFAULT_MOUSE_TIMINGS.artifactExitWaitMs),
  };
}

export function buildRecordStartDelayPlan(recordStartDelayMs, mouseTakeoverLeadMs) {
  const totalDelayMs = parsePositiveInt(recordStartDelayMs, 0);
  const takeoverLeadMs = parsePositiveInt(mouseTakeoverLeadMs, DEFAULT_MOUSE_TAKEOVER_LEAD_MS);
  if (totalDelayMs <= 0) {
    return {
      idleDelayMs: 0,
      preSequenceTakeoverDelayMs: 0,
    };
  }
  return {
    idleDelayMs: Math.max(0, totalDelayMs - takeoverLeadMs),
    preSequenceTakeoverDelayMs: Math.min(totalDelayMs, takeoverLeadMs),
  };
}

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

function appleScriptStringLiteral(value) {
  return `"${String(value).replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
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

export function parseArgs(argv) {
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
    expectedFinalWorkspacePath: DEFAULT_EXPECTED_FINAL_WORKSPACE_PATH,
    diffFilePath: DEFAULT_DIFF_FILE_PATH,
    secondaryDiffFilePath: DEFAULT_SECONDARY_DIFF_FILE_PATH,
    harnessMenuDwellMs: DEFAULT_HARNESS_MENU_DWELL_MS,
    readyPrerollMs: 0,
    recordStartDelayMs: DEFAULT_RECORD_START_DELAY_MS,
    mouseTakeoverLeadMs: DEFAULT_MOUSE_TAKEOVER_LEAD_MS,
    recordVideoOut: null,
    recordFps: 12,
    captureMilestones: false,
    installVisibleHarnesses: true,
    mouseTimings: buildMouseTimings(),
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
    } else if (arg === "--expected-final-workspace") {
      options.expectedFinalWorkspacePath = path.resolve(next);
      index += 1;
    } else if (arg === "--diff-file") {
      options.diffFilePath = next;
      index += 1;
    } else if (arg === "--secondary-diff-file") {
      options.secondaryDiffFilePath = next;
      index += 1;
    } else if (arg === "--harness-label") {
      options.harnessLabel = next;
      index += 1;
    } else if (arg === "--harness-menu-dwell-ms") {
      options.harnessMenuDwellMs = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--ready-preroll-ms") {
      const parsedReadyPrerollMs = Number.parseInt(next, 10);
      options.readyPrerollMs = Number.isFinite(parsedReadyPrerollMs) && parsedReadyPrerollMs > 0 ? parsedReadyPrerollMs : 0;
      index += 1;
    } else if (arg === "--record-start-delay-ms") {
      const parsedRecordStartDelayMs = Number.parseInt(next, 10);
      options.recordStartDelayMs = Number.isFinite(parsedRecordStartDelayMs) && parsedRecordStartDelayMs > 0
        ? parsedRecordStartDelayMs
        : DEFAULT_RECORD_START_DELAY_MS;
      index += 1;
    } else if (arg === "--mouse-takeover-lead-ms") {
      options.mouseTakeoverLeadMs = parsePositiveInt(next, DEFAULT_MOUSE_TAKEOVER_LEAD_MS);
      index += 1;
    } else if (arg === "--record-video-out") {
      options.recordVideoOut = path.resolve(next);
      index += 1;
    } else if (arg === "--record-fps") {
      options.recordFps = Number.parseInt(next, 10);
      index += 1;
    } else if (arg === "--mouse-neutral-move-ms") {
      options.mouseTimings.neutralMoveMs = parsePositiveInt(next, options.mouseTimings.neutralMoveMs);
      index += 1;
    } else if (arg === "--mouse-neutral-wait-ms") {
      options.mouseTimings.neutralWaitMs = parsePositiveInt(next, options.mouseTimings.neutralWaitMs);
      index += 1;
    } else if (arg === "--mouse-harness-trigger-move-ms") {
      options.mouseTimings.harnessTriggerMoveMs = parsePositiveInt(next, options.mouseTimings.harnessTriggerMoveMs);
      index += 1;
    } else if (arg === "--mouse-harness-option-move-ms") {
      options.mouseTimings.harnessOptionMoveMs = parsePositiveInt(next, options.mouseTimings.harnessOptionMoveMs);
      index += 1;
    } else if (arg === "--mouse-composer-move-ms") {
      options.mouseTimings.composerMoveMs = parsePositiveInt(next, options.mouseTimings.composerMoveMs);
      index += 1;
    } else if (arg === "--mouse-prompt-cps") {
      options.mouseTimings.promptTypingCps = parsePositiveNumber(next, options.mouseTimings.promptTypingCps);
      index += 1;
    } else if (arg === "--mouse-prompt-wait-ms") {
      options.mouseTimings.promptWaitMs = parsePositiveInt(next, options.mouseTimings.promptWaitMs);
      index += 1;
    } else if (arg === "--mouse-send-button-move-ms") {
      options.mouseTimings.sendButtonMoveMs = parsePositiveInt(next, options.mouseTimings.sendButtonMoveMs);
      index += 1;
    } else if (arg === "--mouse-send-button-wait-ms") {
      options.mouseTimings.sendButtonWaitMs = parsePositiveInt(next, options.mouseTimings.sendButtonWaitMs);
      index += 1;
    } else if (arg === "--mouse-diff-toggle-move-ms") {
      options.mouseTimings.diffToggleMoveMs = parsePositiveInt(next, options.mouseTimings.diffToggleMoveMs);
      index += 1;
    } else if (arg === "--mouse-diff-toggle-wait-ms") {
      options.mouseTimings.diffToggleWaitMs = parsePositiveInt(next, options.mouseTimings.diffToggleWaitMs);
      index += 1;
    } else if (arg === "--mouse-diff-file-move-ms") {
      options.mouseTimings.diffFileMoveMs = parsePositiveInt(next, options.mouseTimings.diffFileMoveMs);
      index += 1;
    } else if (arg === "--mouse-diff-file-wait-ms") {
      options.mouseTimings.diffFileWaitMs = parsePositiveInt(next, options.mouseTimings.diffFileWaitMs);
      index += 1;
    } else if (arg === "--mouse-artifacts-toggle-move-ms") {
      options.mouseTimings.artifactsToggleMoveMs = parsePositiveInt(next, options.mouseTimings.artifactsToggleMoveMs);
      index += 1;
    } else if (arg === "--mouse-artifacts-toggle-wait-ms") {
      options.mouseTimings.artifactsToggleWaitMs = parsePositiveInt(next, options.mouseTimings.artifactsToggleWaitMs);
      index += 1;
    } else if (arg === "--mouse-artifact-play-move-ms") {
      options.mouseTimings.artifactPlayMoveMs = parsePositiveInt(next, options.mouseTimings.artifactPlayMoveMs);
      index += 1;
    } else if (arg === "--mouse-artifact-play-wait-ms") {
      options.mouseTimings.artifactPlayWaitMs = parsePositiveInt(next, options.mouseTimings.artifactPlayWaitMs);
      index += 1;
    } else if (arg === "--mouse-artifact-exit-move-ms") {
      options.mouseTimings.artifactExitMoveMs = parsePositiveInt(next, options.mouseTimings.artifactExitMoveMs);
      index += 1;
    } else if (arg === "--mouse-artifact-exit-wait-ms") {
      options.mouseTimings.artifactExitWaitMs = parsePositiveInt(next, options.mouseTimings.artifactExitWaitMs);
      index += 1;
    } else if (arg === "--capture-milestones") {
      options.captureMilestones = true;
    }
  }
  return options;
}

async function waitForAutomationWindowRect(browser) {
  return waitFor(async () => {
    try {
      const rect = await browser.getWindowRect();
      if (!rect || !Number.isFinite(rect.x) || !Number.isFinite(rect.y) || !Number.isFinite(rect.width) || !Number.isFinite(rect.height)) {
        return null;
      }
      return {
        x: Number(rect.x),
        y: Number(rect.y),
        width: Number(rect.width),
        height: Number(rect.height),
      };
    } catch {
      return null;
    }
  }, {
    timeoutMs: 10_000,
    intervalMs: 200,
    label: "automation window rect",
  });
}

export function pointFromWindowRectRect(windowRect, metrics, rect, xFraction, yFraction) {
  const frame = windowRect && typeof windowRect === "object" ? windowRect : null;
  if (!frame || !metrics || !rect) {
    throw new Error("window rect, metrics, and rect are required");
  }
  const frameX = Number(metrics.windowInnerPosition?.x ?? frame.x);
  const frameY = Number(metrics.windowInnerPosition?.y ?? frame.y);
  const rectLeft = Number(rect.left);
  const rectTop = Number(rect.top);
  const rectWidth = Number(rect.width);
  const rectHeight = Number(rect.height);
  if (
    !Number.isFinite(frameX) ||
    !Number.isFinite(frameY) ||
    !Number.isFinite(rectLeft) ||
    !Number.isFinite(rectTop) ||
    !Number.isFinite(rectWidth) ||
    !Number.isFinite(rectHeight)
  ) {
    throw new Error("window rect point conversion requires finite numeric metrics");
  }
  const globalXTopDown = frameX + rectLeft + rectWidth * xFraction;
  const globalYTopDown = frameY + rectTop + rectHeight * yFraction;
  return {
    x: Number(globalXTopDown.toFixed(2)),
    y: Number(globalYTopDown.toFixed(2)),
  };
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

export function buildExpectedSidebarTaskTitles(fixture) {
  const activeTasks = Array.isArray(fixture?.active_tasks) ? fixture.active_tasks : [];
  return [...activeTasks]
    .map((task) => ({
      title: String(task?.title || "").trim(),
      minutesAgo: Number(task?.minutes_ago ?? Number.POSITIVE_INFINITY),
    }))
    .filter((task) => task.title)
    .sort((left, right) => left.minutesAgo - right.minutesAgo || left.title.localeCompare(right.title))
    .map((task) => task.title);
}

async function captureSidebarTaskState(browser) {
  return browser.execute(() => {
    const sidebar = document.querySelector(".wb-sidebar");
    const rows = Array.from(document.querySelectorAll(".wb-task-row")).map((element) => {
      const row = element instanceof HTMLElement ? element : null;
      if (!row) return null;
      const title = row.querySelector(".wb-task-title")?.textContent?.trim() || row.getAttribute("aria-label") || "";
      const age = row.querySelector(".wb-task-age")?.textContent?.trim() || "";
      return {
        title,
        age,
        selected: row.classList.contains("wb-task-row-active"),
        hasWorkingSpinner: Boolean(row.querySelector(".wb-task-spinner")),
        hasUnreadDot: Boolean(row.querySelector(".wb-task-status-dot-unread")),
      };
    }).filter(Boolean);
    return {
      sidebarHidden: sidebar?.getAttribute("aria-hidden") === "true",
      hasTaskList: Boolean(document.querySelector(".wb-task-list")),
      roleListItemCount: document.querySelectorAll('[role="listitem"]').length,
      sidebarText: (sidebar?.textContent || "").replace(/\s+/g, " ").trim().slice(0, 240),
      rowCount: rows.length,
      rows,
    };
  });
}

export async function ensureSidebarOpen(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => {
        const sidebar = document.querySelector(".wb-sidebar");
        if (sidebar?.getAttribute("aria-hidden") !== "true") {
          return true;
        }
        const showSidebarButton = document.querySelector(".wb-sidebar-tab-collapsed");
        if (showSidebarButton instanceof HTMLButtonElement) {
          showSidebarButton.click();
        }
        return false;
      }),
    {
      timeout: 10_000,
      timeoutMsg: "sidebar did not open in time",
    },
  );
}

export function sidebarMatchesExpectedState(state, expectedTitles, requiredWorkingTitle) {
  if (!state || !Array.isArray(state.rows)) {
    return false;
  }
  const matchedTitles = state.rows
    .map((row) => String(row?.title || "").trim())
    .filter((title) => expectedTitles.includes(title));
  if (matchedTitles.length !== expectedTitles.length) {
    return false;
  }
  if (matchedTitles.some((title, index) => title !== expectedTitles[index])) {
    return false;
  }
  const workingRows = state.rows.filter((row) => row?.hasWorkingSpinner);
  return workingRows.length === 1 && String(workingRows[0]?.title || "") === requiredWorkingTitle;
}

async function waitForSidebarTaskState(browser, expectedTitles, requiredWorkingTitle, options = {}) {
  const minRowCount = Math.max(expectedTitles.length, Number(options.minRowCount ?? expectedTitles.length));
  const requiredStablePolls = Math.max(1, Number(options.requiredStablePolls ?? SIDEBAR_READY_STABLE_POLLS));
  let stablePolls = 0;
  let lastSignature = null;
  let latestState = null;
  try {
    await browser.waitUntil(async () => {
      const state = await captureSidebarTaskState(browser);
      latestState = state;
      const isReady = state.rowCount >= minRowCount && sidebarMatchesExpectedState(state, expectedTitles, requiredWorkingTitle);
      if (!isReady) {
        stablePolls = 0;
        lastSignature = null;
        return false;
      }
      const signature = JSON.stringify(
        state.rows.map((row) => ({
          title: row.title,
          age: row.age,
          selected: row.selected,
          working: row.hasWorkingSpinner,
          unread: row.hasUnreadDot,
        })),
      );
      if (signature === lastSignature) {
        stablePolls += 1;
      } else {
        lastSignature = signature;
        stablePolls = 1;
      }
      return stablePolls >= requiredStablePolls;
    }, {
      timeout: 30_000,
      timeoutMsg: "sidebar task rows did not reach the expected stable state",
    });
  } catch (error) {
    if (error && typeof error === "object") {
      error.sidebarState = latestState;
    }
    throw error;
  }
  return latestState;
}

function buildMeasuredPointDetail(description, payload, point, windowRect = null, xFraction = 0.5, yFraction = 0.5) {
  return {
    description,
    selector: payload?.selector ?? null,
    label: payload?.label ?? null,
    text: payload?.text ?? null,
    rect: payload?.rect ?? null,
    metrics: payload?.metrics ?? null,
    window_rect: windowRect,
    target_client_point: payload?.rect
      ? {
        x: Number((Number(payload.rect.left) + Number(payload.rect.width) * xFraction).toFixed(2)),
        y: Number((Number(payload.rect.top) + Number(payload.rect.height) * yFraction).toFixed(2)),
      }
      : null,
    point,
  };
}

function copyPoint(point) {
  if (!point) {
    return null;
  }
  return {
    x: Number(point.x),
    y: Number(point.y),
  };
}

function deriveScenarioFinalCursorPoint(scenario, startingPoint = null) {
  let currentPoint = copyPoint(startingPoint);
  for (const action of Array.isArray(scenario?.actions) ? scenario.actions : []) {
    if (!action || typeof action !== "object") {
      continue;
    }
    if ((action.kind === "move" || action.kind === "drag") && Number.isFinite(action.x) && Number.isFinite(action.y)) {
      currentPoint = {
        x: Number(action.x),
        y: Number(action.y),
      };
    }
  }
  return currentPoint;
}

function measuredPointFromPayload(payload, description, windowRect, xFraction = 0.5, yFraction = 0.5) {
  if (!payload?.metrics || !payload?.rect) {
    throw new Error(`unable to measure ${description}`);
  }
  const point = pointFromWindowRectRect(windowRect, payload.metrics, payload.rect, xFraction, yFraction);
  return {
    point,
    detail: buildMeasuredPointDetail(description, payload, point, windowRect, xFraction, yFraction),
  };
}

async function measureSelectorPoint(browser, windowRect, selector, options = {}) {
  const measurement = await measureTargets(browser, {
    target: selector,
  });
  const target = measurement?.elements?.target;
  return measuredPointFromPayload(
    {
      metrics: measurement?.metrics,
      rect: target?.rect,
      selector,
      text: target?.text ?? null,
    },
    options.description ?? selector,
    windowRect,
    options.xFraction ?? 0.5,
    options.yFraction ?? 0.5,
  );
}

async function measureHarnessOptionPoint(browser, windowRect, harnessLabel, options = {}) {
  const payload = await browser.execute(async (label) => {
    const bridge = globalThis.__ctxE2E;
    if (!bridge || typeof bridge.measureHarnessOption !== "function") {
      throw new Error("ctxE2E measureHarnessOption bridge unavailable");
    }
    return bridge.measureHarnessOption(label);
  }, harnessLabel);
  return measuredPointFromPayload(
    payload,
    `harness option ${harnessLabel}`,
    windowRect,
    options.xFraction ?? 0.5,
    options.yFraction ?? 0.5,
  );
}

async function measureDiffFilePoint(browser, windowRect, filePath, options = {}) {
  const payload = await browser.execute(async (targetPath) => {
    const bridge = globalThis.__ctxE2E;
    if (!bridge || typeof bridge.measureDiffFile !== "function") {
      throw new Error("ctxE2E measureDiffFile bridge unavailable");
    }
    return bridge.measureDiffFile(targetPath);
  }, filePath);
  return measuredPointFromPayload(
    payload,
    `diff file ${filePath}`,
    windowRect,
    options.xFraction ?? 0.5,
    options.yFraction ?? 0.5,
  );
}

async function writeAndRunScenario(artifactDir, basename, scenario, detail = null) {
  const scenarioPath = path.join(artifactDir, `${basename}.json`);
  const scenarioWithCursorState = {
    ...(logicalCursorPoint ? { initial_cursor_point: logicalCursorPoint } : {}),
    ...scenario,
  };
  writeJson(scenarioPath, scenarioWithCursorState);
  if (detail) {
    writeJson(path.join(artifactDir, `${basename.replace(/-scenario$/, "")}-target.json`), detail);
  }
  await runConductor(scenarioPath);
  logicalCursorPoint = deriveScenarioFinalCursorPoint(scenarioWithCursorState, logicalCursorPoint);
  return scenarioPath;
}

async function clickMeasuredPoint(artifactDir, basename, measuredTarget, options = {}) {
  return writeAndRunScenario(
    artifactDir,
    `${basename}-scenario`,
    buildClickScenario(measuredTarget.point, {
      moveDurationMs: options.moveDurationMs,
      waitDurationMs: options.waitDurationMs,
    }),
    measuredTarget.detail,
  );
}

async function moveCursorToNeutralWorkbenchPoint(browser, artifactDir, mouseTimings, windowRect) {
  const measuredTarget = await measureSelectorPoint(browser, windowRect, ".wb-main", {
    description: "neutral workbench point",
    xFraction: 0.12,
    yFraction: 0.06,
  });
  await writeAndRunScenario(
    artifactDir,
    "neutral-cursor-scenario",
    {
      actions: [
        { kind: "move", x: measuredTarget.point.x, y: measuredTarget.point.y, duration_ms: mouseTimings.neutralMoveMs },
        { kind: "wait", duration_ms: mouseTimings.neutralWaitMs },
      ],
    },
    measuredTarget.detail,
  );
  return measuredTarget;
}

function activateAutomationApp(appPath) {
  const result = spawnSync(
    "osascript",
    [
      "-e",
      `tell application (POSIX file ${appleScriptStringLiteral(appPath)} as text) to activate`,
    ],
    {
      stdio: "ignore",
    },
  );
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`failed to activate automation app at ${appPath}`);
  }
}

async function waitForBrowserFocus(browser) {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => document.hasFocus() && document.visibilityState === "visible"),
    {
      timeout: 10_000,
      timeoutMsg: "automation app did not become frontmost after activate",
    },
  );
}

async function clickNeutralWorkbenchPoint(artifactDir, neutralTarget, mouseTimings) {
  await writeAndRunScenario(
    artifactDir,
    "neutral-focus-scenario",
    {
      actions: [
        { kind: "click", button: "left" },
        { kind: "wait", duration_ms: mouseTimings.neutralWaitMs },
      ],
    },
    neutralTarget.detail,
  );
}

async function captureHarnessMenuState(browser, harnessLabel) {
  return browser.execute((label) => {
    const trigger = document.querySelector(".wb-switcher-harness");
    const rows = Array.from(document.querySelectorAll(".wb-harness-row-main")).map((element) => element.textContent?.trim() ?? "");
    return {
      document_has_focus: document.hasFocus(),
      visibility_state: document.visibilityState,
      trigger_text: trigger?.textContent?.trim() ?? null,
      harness_menu_present: Boolean(document.querySelector(".wb-harness-menu")),
      matching_row_present: rows.some((text) => text.includes(label)),
      visible_rows: rows,
      mouse_probe: globalThis.__ctxDemoMouseProbe ?? null,
      active_element_tag: document.activeElement?.tagName ?? null,
      active_element_text: document.activeElement?.textContent?.trim()?.slice(0, 120) ?? null,
    };
  }, harnessLabel);
}

async function captureHarnessMenuFailureArtifacts(browser, artifactDir, harnessLabel) {
  writeJson(
    path.join(artifactDir, "harness-menu-failure-state.json"),
    await captureHarnessMenuState(browser, harnessLabel),
  );
  const screenshotPath = path.join(artifactDir, "harness-menu-failure.png");
  const pngBase64 = await browser.takeScreenshot();
  writeFileSync(screenshotPath, Buffer.from(pngBase64, "base64"));
}

async function installMouseProbe(browser) {
  await browser.execute(() => {
    if (globalThis.__ctxDemoMouseProbeInstalled) {
      return;
    }
    const probe = {
      lastTarget: null,
      harnessTrigger: { mousedown: 0, mouseup: 0, click: 0 },
      composerTextarea: { mousedown: 0, mouseup: 0, click: 0 },
      body: { mousedown: 0, mouseup: 0, click: 0 },
    };
    const attachCounters = (element, bucketName) => {
      if (!(element instanceof HTMLElement)) {
        return;
      }
      for (const type of ["mousedown", "mouseup", "click"]) {
        element.addEventListener(
          type,
          (event) => {
            probe[bucketName][type] += 1;
            probe.lastTarget = {
              bucket: bucketName,
              type,
              clientX: event.clientX,
              clientY: event.clientY,
              text: event.currentTarget instanceof HTMLElement ? event.currentTarget.textContent?.trim()?.slice(0, 120) ?? "" : "",
            };
          },
          true,
        );
      }
    };
    attachCounters(document.body, "body");
    attachCounters(document.querySelector(".wb-switcher-harness"), "harnessTrigger");
    attachCounters(document.querySelector("textarea.wb-new-composer-textarea"), "composerTextarea");
    globalThis.__ctxDemoMouseProbe = probe;
    globalThis.__ctxDemoMouseProbeInstalled = true;
  });
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

async function selectHarness(browser, artifactDir, harnessLabel, menuDwellMs, mouseTimings, windowRect) {
  await clickMeasuredPoint(
    artifactDir,
    "harness-trigger",
    await measureSelectorPoint(browser, windowRect, ".wb-switcher-harness", {
      description: "harness trigger",
      xFraction: 0.52,
      yFraction: 0.5,
    }),
    {
      moveDurationMs: mouseTimings.harnessTriggerMoveMs,
      waitDurationMs: mouseTimings.diffToggleWaitMs,
    },
  );
  try {
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
  } catch (error) {
    await captureHarnessMenuFailureArtifacts(browser, artifactDir, harnessLabel);
    throw error;
  }
  await sleep(menuDwellMs);
  await clickMeasuredPoint(
    artifactDir,
    "harness-option",
    await measureHarnessOptionPoint(browser, windowRect, harnessLabel),
    {
      moveDurationMs: mouseTimings.harnessOptionMoveMs,
      waitDurationMs: mouseTimings.diffToggleWaitMs,
    },
  );
  await browser.waitUntil(
    async () =>
      await browser.execute(
        (label) => document.querySelector(".wb-switcher-harness")?.textContent?.includes(label) ?? false,
        harnessLabel,
      ),
    {
      timeout: 10_000,
      timeoutMsg: `selected harness ${harnessLabel} did not appear on the trigger`,
    },
  );
  await sleep(HARNESS_SELECTED_DWELL_MS);
}

async function openHarnessMenu(browser) {
  await browser.execute(() => {
    const trigger = document.querySelector(".wb-switcher-harness");
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("harness trigger not found");
    }
    trigger.click();
  });
  await browser.waitUntil(
    async () =>
      await browser.execute(() => Boolean(document.querySelector(".wb-harness-menu .wb-harness-row"))),
    {
      timeout: 10_000,
      timeoutMsg: "harness menu did not open",
    },
  );
}

async function closeHarnessMenu(browser) {
  const menuOpen = await browser.execute(() => Boolean(document.querySelector(".wb-harness-menu")));
  if (!menuOpen) {
    return;
  }
  await browser.execute(() => {
    const trigger = document.querySelector(".wb-switcher-harness");
    if (!(trigger instanceof HTMLButtonElement)) {
      throw new Error("harness trigger not found");
    }
    trigger.click();
  });
  await browser.waitUntil(
    async () =>
      !(await browser.execute(() => Boolean(document.querySelector(".wb-harness-menu")))),
    {
      timeout: 10_000,
      timeoutMsg: "harness menu did not close",
    },
  );
}

async function captureVisibleHarnessRows(browser) {
  return browser.execute(() => {
    const list = document.querySelector(".wb-harness-list");
    if (!(list instanceof HTMLElement)) {
      return [];
    }
    const listRect = list.getBoundingClientRect();
    return Array.from(list.querySelectorAll(".wb-harness-row"))
      .map((row) => {
        if (!(row instanceof HTMLElement)) {
          return null;
        }
        const rect = row.getBoundingClientRect();
        const label = row.querySelector(".wb-harness-name")?.textContent?.trim() ?? "";
        const installButton = row.querySelector(".wb-harness-install");
        const isVisible =
          label.length > 0
          && rect.height > 0
          && rect.bottom > listRect.top
          && rect.top < listRect.bottom;
        if (!isVisible) {
          return null;
        }
        return {
          label,
          hasInstallButton: installButton instanceof HTMLButtonElement,
          installDisabled: installButton instanceof HTMLButtonElement ? installButton.disabled : false,
        };
      })
      .filter(Boolean);
  });
}

export function visibleHarnessRowsNeedInstall(rows) {
  return Array.isArray(rows) && rows.some((row) => row?.hasInstallButton);
}

function resolveHarnessProviderId(label) {
  const normalizedLabel = String(label || "").trim();
  return HARNESS_LABEL_TO_PROVIDER_ID[normalizedLabel] || null;
}

async function waitForVisibleHarnessRowsInstalled(browser, timeout = 30_000) {
  await browser.waitUntil(async () => {
    const rows = await captureVisibleHarnessRows(browser);
    return rows.length > 0 && !visibleHarnessRowsNeedInstall(rows);
  }, {
    timeout,
    timeoutMsg: "visible harness rows still showed Install after preinstall",
  });
}

async function preinstallVisibleHarnesses(browser, baseUrl, authToken, artifactDir) {
  await openHarnessMenu(browser);
  const beforeRows = await captureVisibleHarnessRows(browser);
  writeJson(path.join(artifactDir, "visible-harness-rows-before-install.json"), beforeRows);

  const labelsNeedingInstall = [...new Set(
    beforeRows
      .filter((row) => row.hasInstallButton)
      .map((row) => row.label),
  )];
  if (labelsNeedingInstall.length === 0) {
    await closeHarnessMenu(browser);
    return [];
  }

  const unresolvedLabels = labelsNeedingInstall.filter((label) => !resolveHarnessProviderId(label));
  if (unresolvedLabels.length > 0) {
    throw new Error(`unable to map visible harness labels to provider ids: ${unresolvedLabels.join(", ")}`);
  }

  await closeHarnessMenu(browser);

  const installs = [];
  for (const label of labelsNeedingInstall) {
    const providerId = resolveHarnessProviderId(label);
    installs.push({
      label,
      provider_id: providerId,
      install: await installProviderAndWait(baseUrl, authToken, providerId, {
        target: "host",
        timeoutMs: 20 * 60_000,
        pollMs: 2_000,
      }),
    });
  }
  writeJson(path.join(artifactDir, "visible-harness-installs.json"), installs);

  await sleep(1_500);
  await openHarnessMenu(browser);
  try {
    await waitForVisibleHarnessRowsInstalled(browser);
  } catch (error) {
    await closeHarnessMenu(browser);
    await browser.refresh();
    await focusNewTask(browser);
    await ensureNewTaskVisible(browser, artifactDir);
    await sleep(1_000);
    await openHarnessMenu(browser);
    await waitForVisibleHarnessRowsInstalled(browser);
  }
  const afterRows = await captureVisibleHarnessRows(browser);
  writeJson(path.join(artifactDir, "visible-harness-rows-after-install.json"), afterRows);
  await closeHarnessMenu(browser);
  return installs;
}

async function typePromptWithMouse(browser, artifactDir, promptText, mouseTimings, windowRect) {
  const measuredComposer = await measureSelectorPoint(browser, windowRect, "textarea.wb-new-composer-textarea", {
    description: "new task composer",
  });
  const promptScenarioPath = await writeAndRunScenario(
    artifactDir,
    "prompt-scenario",
    buildPromptScenario(measuredComposer.point, promptText, {
      moveDurationMs: mouseTimings.composerMoveMs,
      cps: mouseTimings.promptTypingCps,
      submitViaKey: false,
      waitDurationMs: mouseTimings.promptWaitMs,
    }),
    measuredComposer.detail,
  );
  await browser.waitUntil(
    async () =>
      await browser.execute(
        (expectedValue) => {
          const textarea = document.querySelector("textarea.wb-new-composer-textarea");
          return textarea instanceof HTMLTextAreaElement && textarea.value === expectedValue;
        },
        promptText,
      ),
    {
      timeout: 30_000,
      timeoutMsg: "prompt text did not finish typing into the new task composer",
    },
  );
  return {
    promptScenarioPath,
    typedPrompt: await browser.execute(() => {
      const textarea = document.querySelector("textarea.wb-new-composer-textarea");
      return textarea instanceof HTMLTextAreaElement ? textarea.value : "";
    }),
  };
}

async function clickComposerSendButton(browser, artifactDir, mouseTimings, windowRect) {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => {
        const sendButton = document.querySelector("button.wb-send[aria-label=\"Send\"]");
        return sendButton instanceof HTMLButtonElement && !sendButton.disabled;
      }),
    {
      timeout: 30_000,
      timeoutMsg: "new task send button did not become enabled",
    },
  );
  await clickMeasuredPoint(
    artifactDir,
    "composer-send",
    await measureSelectorPoint(browser, windowRect, "button.wb-send[aria-label=\"Send\"]", {
      description: "new task send button",
    }),
    {
      moveDurationMs: mouseTimings.sendButtonMoveMs,
      waitDurationMs: mouseTimings.sendButtonWaitMs,
    },
  );
}

async function openDiffPaneWithMouse(browser, artifactDir, mouseTimings, windowRect) {
  await clickMeasuredPoint(
    artifactDir,
    "diff-toggle",
    await measureSelectorPoint(browser, windowRect, "button[aria-label=\"Toggle diff view\"]", {
      description: "diff toggle button",
    }),
    {
      moveDurationMs: mouseTimings.diffToggleMoveMs,
      waitDurationMs: mouseTimings.diffToggleWaitMs,
    },
  );
}

async function openDiffFileWithMouse(browser, artifactDir, filePath, mouseTimings, windowRect) {
  await clickMeasuredPoint(
    artifactDir,
    "diff-file",
    await measureDiffFilePoint(browser, windowRect, filePath),
    {
      moveDurationMs: mouseTimings.diffFileMoveMs,
      waitDurationMs: mouseTimings.diffFileWaitMs,
    },
  );
}

async function openArtifactsPaneWithMouse(browser, artifactDir, mouseTimings, windowRect) {
  await clickMeasuredPoint(
    artifactDir,
    "artifacts-toggle",
    await measureSelectorPoint(browser, windowRect, "button[aria-label=\"Toggle artifacts\"]", {
      description: "artifacts toggle button",
    }),
    {
      moveDurationMs: mouseTimings.artifactsToggleMoveMs,
      waitDurationMs: mouseTimings.artifactsToggleWaitMs,
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
    const loadingLabel = Array.from(body?.querySelectorAll(".muted") ?? [])
      .map((element) => element.textContent?.trim() ?? "")
      .find((text) => text.length > 0) ?? null;
    return {
      target_path: targetPath,
      has_file_row: Boolean(diffFile),
      has_open_body: body instanceof HTMLElement,
      has_editor_shell: Boolean(body?.querySelector(".cursor-diff-editor-shell")),
      has_monaco_editor: Boolean(body?.querySelector(".monaco-editor")),
      loading_label: loadingLabel,
    };
  }, filePath);
}

async function waitForOpenedDiffFileReady(browser, filePath) {
  await browser.waitUntil(async () => {
    const state = await captureOpenedDiffFileState(browser, filePath);
    return state.has_open_body && state.has_editor_shell && state.has_monaco_editor && !state.loading_label;
  }, {
    timeout: 30_000,
    interval: 150,
    timeoutMsg: `diff file ${filePath} did not finish rendering`,
  });
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

function seedExpectedWorkspaceDiff(
  worktreeRoot,
  expectedFinalWorkspacePath,
  relativePaths = EXPECTED_FINAL_WORKSPACE_DIFF_FILES,
) {
  if (!existsSync(expectedFinalWorkspacePath)) {
    throw new Error(`expected final workspace does not exist: ${expectedFinalWorkspacePath}`);
  }

  for (const relativePath of relativePaths) {
    const sourcePath = path.join(expectedFinalWorkspacePath, relativePath);
    if (!existsSync(sourcePath)) {
      throw new Error(`expected final workspace file does not exist: ${sourcePath}`);
    }

    const destinationPath = path.join(worktreeRoot, relativePath);
    mkdirSync(path.dirname(destinationPath), { recursive: true });
    copyFileSync(sourcePath, destinationPath);
  }
}

export function computeArtifactPlaybackTimeoutMs(durationSeconds) {
  if (Number.isFinite(durationSeconds) && durationSeconds > 0) {
    return Math.max(12_000, Math.ceil((durationSeconds + 1.5) * 1000));
  }
  return 20_000;
}

export function buildArtifactPlaybackCompletionPlan(durationSeconds) {
  return {
    timeoutMs: computeArtifactPlaybackTimeoutMs(durationSeconds),
    tailDwellMs: POST_ARTIFACT_VIDEO_COMPLETION_DWELL_MS,
  };
}

async function readArtifactVideoState(browser) {
  return browser.execute(() => {
    const video = document.querySelector(".wb-artifact-video");
    if (!(video instanceof HTMLVideoElement)) {
      return null;
    }
    return {
      currentTime: video.currentTime,
      duration: Number.isFinite(video.duration) ? video.duration : null,
      paused: video.paused,
      ended: video.ended,
      controls: video.controls,
      loop: video.loop,
    };
  });
}

async function prepareArtifactVideoForManualPlayback(browser) {
  return browser.execute(async () => {
    const video = document.querySelector(".wb-artifact-video");
    if (!(video instanceof HTMLVideoElement)) {
      return null;
    }
    if (video.readyState < HTMLMediaElement.HAVE_METADATA) {
      await new Promise((resolve) => {
        const onLoadedMetadata = () => {
          video.removeEventListener("loadedmetadata", onLoadedMetadata);
          resolve();
        };
        video.addEventListener("loadedmetadata", onLoadedMetadata, { once: true });
      });
    }
    video.pause();
    video.currentTime = 0;
    video.loop = false;
    video.controls = true;
    video.setAttribute("controls", "");
    video.style.pointerEvents = "auto";
    video.muted = true;
    return {
      currentTime: video.currentTime,
      duration: Number.isFinite(video.duration) ? video.duration : null,
      paused: video.paused,
      ended: video.ended,
      controls: video.controls,
      loop: video.loop,
    };
  });
}

async function hideArtifactVideoChrome(browser) {
  return browser.execute(() => {
    const video = document.querySelector(".wb-artifact-video");
    if (!(video instanceof HTMLVideoElement)) {
      return null;
    }
    video.controls = false;
    video.removeAttribute("controls");
    video.style.pointerEvents = "none";
    return {
      currentTime: video.currentTime,
      duration: Number.isFinite(video.duration) ? video.duration : null,
      paused: video.paused,
      ended: video.ended,
      controls: video.controls,
      loop: video.loop,
    };
  });
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

async function openArtifactsPaneAndWait(browser, artifactDir, mouseTimings, windowRect, attempts = 2) {
  let lastError = null;
  let lastToggleResult = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    lastToggleResult = {
      opened_with_mouse: true,
      attempt: attempt + 1,
    };
    await openArtifactsPaneWithMouse(browser, artifactDir, mouseTimings, windowRect);
    try {
      await waitForArtifactsPane(browser);
      return lastToggleResult;
    } catch (error) {
      lastError = error;
      await sleep(300);
    }
  }
  throw lastError instanceof Error ? lastError : new Error("artifacts pane did not open in time");
}

async function clickArtifactVideoPlay(browser, artifactDir, neutralPoint, mouseTimings, windowRect) {
  const measuredVideo = await measureSelectorPoint(browser, windowRect, ".wb-artifact-video", {
    description: "artifact video",
  });
  const scenario = {
    actions: [
      { kind: "move", x: measuredVideo.point.x, y: measuredVideo.point.y, duration_ms: mouseTimings.artifactPlayMoveMs },
      { kind: "click", button: "left" },
      { kind: "wait", duration_ms: mouseTimings.artifactPlayWaitMs },
    ],
  };
  if (neutralPoint) {
    scenario.actions.push({
      kind: "move",
      x: neutralPoint.x,
      y: neutralPoint.y,
      duration_ms: mouseTimings.artifactExitMoveMs,
    });
    scenario.actions.push({
      kind: "wait",
      duration_ms: mouseTimings.artifactExitWaitMs,
    });
  }
  await writeAndRunScenario(
    artifactDir,
    "artifact-play-scenario",
    scenario,
    {
      artifact_video: measuredVideo.detail,
      neutral_point: neutralPoint ?? null,
    },
  );
}

async function waitForArtifactVideoCompletion(browser) {
  const initialState = await readArtifactVideoState(browser);
  const completionPlan = buildArtifactPlaybackCompletionPlan(initialState?.duration ?? null);
  await browser.waitUntil(async () => {
    const state = await readArtifactVideoState(browser);
    if (!state) return false;
    if (state.ended) return true;
    return (
      Number.isFinite(state.duration) &&
      state.duration > 0 &&
      state.currentTime >= state.duration - 0.05 &&
      state.paused
    );
  }, {
    timeout: completionPlan.timeoutMs,
    timeoutMsg: "artifact video preview did not finish in time",
  });
  await sleep(completionPlan.tailDwellMs);
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
        mime_type: inferVideoArtifactMimeType(artifactPath),
      },
    ],
  });
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  logicalCursorPoint = null;
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
  let promptScenarioPath = null;

  try {
    const setup = await startSetupProcess({
      ...options,
      installVisibleHarnesses: false,
    });
    relayServer = setup.relay.server;
    daemonProc = setup.daemon.proc;
    const setupManifest = setup.manifest;
    const authToken = readTokenFromManifest(setupManifest);
    const baseUrl = setupManifest.daemon.url;
    const workspaceId = setupManifest.fixture.workspace_id;
    await api(baseUrl, authToken, "POST", `/api/workspaces/${workspaceId}/providers/codex/verify`, {});
    if (options.installVisibleHarnesses) {
      writeJson(
        path.join(options.artifactDir, "visible-harness-install-all.json"),
        await installVisibleHarnessProvidersAndWait(baseUrl, authToken, "host"),
      );
      options.installVisibleHarnesses = false;
    }

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
    await ensureWorkbenchVisible(browser, options.artifactDir);
    await focusNewTask(browser);
    await ensureNewTaskVisible(browser, options.artifactDir);
    await ensureSidebarOpen(browser);
    await clearDraftHarness(browser);
    if (options.installVisibleHarnesses) {
      await preinstallVisibleHarnesses(browser, baseUrl, authToken, options.artifactDir);
      await focusNewTask(browser);
      await ensureNewTaskVisible(browser, options.artifactDir);
      await ensureSidebarOpen(browser);
      await clearDraftHarness(browser);
    }
    const expectedSidebarTitles = buildExpectedSidebarTaskTitles(fixture);
    await sleep(2_200);
    const sidebarState = await captureSidebarTaskState(browser);
    if (!sidebarMatchesExpectedState(sidebarState, expectedSidebarTitles, "Tune muted-domain toast timing")) {
      writeJson(path.join(options.artifactDir, "sidebar-task-state-failure.json"), {
        expected_titles: expectedSidebarTitles,
        observed: sidebarState,
        error: "sidebar task rows were not fully ready before capture start",
      });
    }
    const recordStartDelayPlan = buildRecordStartDelayPlan(options.recordStartDelayMs, options.mouseTakeoverLeadMs);
    writeJson(path.join(options.artifactDir, "sidebar-task-state.json"), sidebarState);
    if (recordStartDelayPlan.idleDelayMs > 0) {
      await sleep(recordStartDelayPlan.idleDelayMs);
    }
    activateAutomationApp(launchAppPath);
    await waitForBrowserFocus(browser);
    await sleep(APP_FRONTMOST_SETTLE_MS);
    const automationWindowRect = await waitForAutomationWindowRect(browser);
    writeJson(path.join(options.artifactDir, "automation-window-rect.json"), automationWindowRect);
    await installMouseProbe(browser);
    const neutralTarget = await moveCursorToNeutralWorkbenchPoint(
      browser,
      options.artifactDir,
      options.mouseTimings,
      automationWindowRect,
    );
    await clickNeutralWorkbenchPoint(options.artifactDir, neutralTarget, options.mouseTimings);
    await waitForBrowserFocus(browser);
    const neutralPoint = neutralTarget.point;
    if (recordStartDelayPlan.preSequenceTakeoverDelayMs > 0) {
      await sleep(recordStartDelayPlan.preSequenceTakeoverDelayMs);
    }
    if (options.recordVideoOut) {
      recordingStartedAt = Date.now();
      videoCapture = startBrowserFrameCapture(browser, options.artifactDir, options.recordFps);
      if (options.captureMilestones) {
        await captureMilestone(browser, options.artifactDir, recordingStartedAt, "new-task-ready");
      }
    }
    if (options.readyPrerollMs > 0) {
      await sleep(options.readyPrerollMs);
    }
    await sleep(450);

    await selectHarness(
      browser,
      options.artifactDir,
      options.harnessLabel,
      options.harnessMenuDwellMs,
      options.mouseTimings,
      automationWindowRect,
    );
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "harness-selected");
    }
    await sleep(POST_HARNESS_DWELL_MS);

    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "prompt-typing-start");
    }
    const promptResult = await typePromptWithMouse(
      browser,
      options.artifactDir,
      options.promptText,
      options.mouseTimings,
      automationWindowRect,
    );
    promptScenarioPath = promptResult.promptScenarioPath;
    const typedPrompt = promptResult.typedPrompt;
    writeJson(path.join(options.artifactDir, "prompt-typed-state.json"), { value: typedPrompt });
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "prompt-typed");
    }
    await sleep(POST_PROMPT_DWELL_MS);
    const existingTasks = await api(baseUrl, authToken, "GET", `/api/workspaces/${workspaceId}/tasks`);
    const knownTaskIds = Array.isArray(existingTasks)
      ? new Set(existingTasks.map((task) => String(task?.id || "")).filter(Boolean))
      : new Set();
    await clickComposerSendButton(browser, options.artifactDir, options.mouseTimings, automationWindowRect);

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
    seedExpectedWorkspaceDiff(worktreeRoot, options.expectedFinalWorkspacePath);
    const workspaceDiff = await waitForWorkspaceDiff(worktreeRoot);
    writeJson(path.join(options.artifactDir, "workspace-diff-detected.json"), {
      worktree_root: worktreeRoot,
      status: workspaceDiff,
    });
    await sleep(350);

    await setDiffPanePlaybackHidden(browser, true);
    await openDiffPaneWithMouse(browser, options.artifactDir, options.mouseTimings, automationWindowRect);
    const diffToggleResult = {
      opened_with_mouse: true,
    };
    writeJson(path.join(options.artifactDir, "diff-toggle-result.json"), diffToggleResult);
    await waitForDiffPane(browser);
    await waitForDiffPaneReady(browser);
    await setDiffPanePlaybackHidden(browser, false);
    writeJson(path.join(options.artifactDir, "diff-pane-state.json"), await captureDiffPaneState(browser));
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "diff-file-list-ready");
    }
    await sleep(DIFF_FILE_LIST_DWELL_MS);

    await openDiffFileWithMouse(
      browser,
      options.artifactDir,
        options.diffFilePath,
        options.mouseTimings,
        automationWindowRect,
      );
    await waitForOpenedDiffFileReady(browser, options.diffFilePath);
    writeJson(
      path.join(options.artifactDir, "opened-diff-file-state.json"),
      await captureOpenedDiffFileState(browser, options.diffFilePath),
    );
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "diff-file-open");
    }
    await sleep(PRIMARY_DIFF_PANE_DWELL_MS);

    if (options.secondaryDiffFilePath) {
      await openDiffFileWithMouse(
        browser,
        options.artifactDir,
        options.secondaryDiffFilePath,
        options.mouseTimings,
        automationWindowRect,
      );
      await waitForOpenedDiffFileReady(browser, options.secondaryDiffFilePath);
      writeJson(
        path.join(options.artifactDir, "opened-secondary-diff-file-state.json"),
        await captureOpenedDiffFileState(browser, options.secondaryDiffFilePath),
      );
      if (videoCapture && options.captureMilestones) {
        await captureMilestone(browser, options.artifactDir, recordingStartedAt, "secondary-diff-file-open");
      }
      await sleep(PRIMARY_DIFF_PANE_DWELL_MS);
    }

    const attachedArtifacts = await attachSessionArtifact(baseUrl, authToken, session.id, options.sessionArtifactPath);
    writeJson(path.join(options.artifactDir, "attached-artifacts.json"), attachedArtifacts);
    await sleep(POST_ARTIFACT_ATTACH_DWELL_MS);

    await setArtifactsPanePlaybackHidden(browser, true);
    const artifactsToggleResult = await openArtifactsPaneAndWait(
      browser,
      options.artifactDir,
      options.mouseTimings,
      automationWindowRect,
    );
    writeJson(path.join(options.artifactDir, "artifacts-toggle-result.json"), artifactsToggleResult);
    writeJson(
      path.join(options.artifactDir, "artifact-video-prepared.json"),
      await prepareArtifactVideoForManualPlayback(browser),
    );
    await setArtifactsPanePlaybackHidden(browser, false);
    await clickArtifactVideoPlay(
      browser,
      options.artifactDir,
      neutralPoint,
      options.mouseTimings,
      automationWindowRect,
    );
    await waitForArtifactVideoPlayback(browser);
    writeJson(
      path.join(options.artifactDir, "artifact-video-controls-hidden.json"),
      await hideArtifactVideoChrome(browser),
    );
    writeJson(path.join(options.artifactDir, "artifacts-pane-state.json"), await captureArtifactsPaneState(browser));
    if (videoCapture && options.captureMilestones) {
      await captureMilestone(browser, options.artifactDir, recordingStartedAt, "artifact-video-playing");
    }
    await waitForArtifactVideoCompletion(browser);
    if (options.recordVideoOut) {
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
