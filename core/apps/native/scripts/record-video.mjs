#!/usr/bin/env node
"use strict";

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const DEFAULT_ADDR = "http://127.0.0.1:6160";
const DEFAULT_FPS = 30;
const DEFAULT_READY_TIMEOUT_MS = 30000;
const DEFAULT_REQUEST_TIMEOUT_MS = 15000;
const DEFAULT_TMP_DIR = "/tmp/ctx-native-video";

let activeRecording = null;

function printHelp() {
  console.log(`Usage: node core/apps/native/scripts/record-video.mjs [options]

Options:
  --out <path>               Output mp4 path (required)
  --fps <fps>                Frames per second (default: ${DEFAULT_FPS})
  --tmp-dir <dir>            Directory for intermediate frames (default: ${DEFAULT_TMP_DIR})
  --keep-frames              Do not delete intermediate frames
  --addr <host:port|url>     Automation server address (default: ${DEFAULT_ADDR})
  --ready-timeout-ms <ms>    Timeout for /ready (default: ${DEFAULT_READY_TIMEOUT_MS})
  --request-timeout-ms <ms>  Timeout for /screenshot requests (default: ${DEFAULT_REQUEST_TIMEOUT_MS})
  --duration-ms <ms>         Stop after the given duration
  --duration-s <seconds>     Stop after the given duration
  -h, --help                 Show this help
`);
}

function normalizeAddr(value) {
  const trimmed = String(value || "").trim();
  if (!trimmed) {
    throw new Error("--addr must not be empty");
  }
  if (/^[a-z]+:\/\//i.test(trimmed)) {
    return trimmed;
  }
  return `http://${trimmed}`;
}

function toPositiveInt(value, flag) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) {
    throw new Error(`${flag} must be a positive number`);
  }
  return Math.trunc(number);
}

function toNonNegativeInt(value, flag) {
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0) {
    throw new Error(`${flag} must be a non-negative number`);
  }
  return Math.trunc(number);
}

function parseArgs(argv) {
  const args = {
    outPath: null,
    fps: DEFAULT_FPS,
    tmpDir: DEFAULT_TMP_DIR,
    addr: DEFAULT_ADDR,
    readyTimeoutMs: DEFAULT_READY_TIMEOUT_MS,
    requestTimeoutMs: DEFAULT_REQUEST_TIMEOUT_MS,
    keepFrames: false,
    durationMs: null,
    showHelp: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      args.showHelp = true;
      continue;
    }
    if (arg === "--keep-frames") {
      args.keepFrames = true;
      continue;
    }
    if (arg === "--out") {
      if (i + 1 >= argv.length) {
        throw new Error("--out requires a value");
      }
      args.outPath = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--out=")) {
      args.outPath = arg.slice("--out=".length);
      continue;
    }
    if (arg === "--fps") {
      if (i + 1 >= argv.length) {
        throw new Error("--fps requires a value");
      }
      args.fps = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--fps=")) {
      args.fps = arg.slice("--fps=".length);
      continue;
    }
    if (arg === "--tmp-dir") {
      if (i + 1 >= argv.length) {
        throw new Error("--tmp-dir requires a value");
      }
      args.tmpDir = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--tmp-dir=")) {
      args.tmpDir = arg.slice("--tmp-dir=".length);
      continue;
    }
    if (arg === "--addr") {
      if (i + 1 >= argv.length) {
        throw new Error("--addr requires a value");
      }
      args.addr = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--addr=")) {
      args.addr = arg.slice("--addr=".length);
      continue;
    }
    if (arg === "--ready-timeout-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--ready-timeout-ms requires a value");
      }
      args.readyTimeoutMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--ready-timeout-ms=")) {
      args.readyTimeoutMs = arg.slice("--ready-timeout-ms=".length);
      continue;
    }
    if (arg === "--request-timeout-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--request-timeout-ms requires a value");
      }
      args.requestTimeoutMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--request-timeout-ms=")) {
      args.requestTimeoutMs = arg.slice("--request-timeout-ms=".length);
      continue;
    }
    if (arg === "--duration-ms") {
      if (i + 1 >= argv.length) {
        throw new Error("--duration-ms requires a value");
      }
      args.durationMs = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg.startsWith("--duration-ms=")) {
      args.durationMs = arg.slice("--duration-ms=".length);
      continue;
    }
    if (arg === "--duration-s") {
      if (i + 1 >= argv.length) {
        throw new Error("--duration-s requires a value");
      }
      args.durationMs = Number(argv[i + 1]) * 1000;
      i += 1;
      continue;
    }
    if (arg.startsWith("--duration-s=")) {
      args.durationMs = Number(arg.slice("--duration-s=".length)) * 1000;
      continue;
    }

    throw new Error(`Unknown argument: ${arg}`);
  }

  if (args.durationMs !== null) {
    args.durationMs = toNonNegativeInt(args.durationMs, "--duration-ms");
  }

  if (args.outPath) {
    args.outPath = String(args.outPath);
  }
  args.fps = toPositiveInt(args.fps, "--fps");
  args.tmpDir = String(args.tmpDir);
  args.addr = normalizeAddr(args.addr);
  args.readyTimeoutMs = toPositiveInt(args.readyTimeoutMs, "--ready-timeout-ms");
  args.requestTimeoutMs = toPositiveInt(args.requestTimeoutMs, "--request-timeout-ms");

  return args;
}

function sleep(ms) {
  if (ms <= 0) {
    return Promise.resolve();
  }
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function callJson(baseUrl, endpoint, options = {}) {
  const { method = "GET", body, timeoutMs = DEFAULT_REQUEST_TIMEOUT_MS } = options;
  const url = new URL(endpoint, baseUrl);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
  const headers = { accept: "application/json" };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
  }

  try {
    const response = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
    });
    const text = await response.text();
    let payload;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch (err) {
        throw new Error(`invalid JSON response: ${err.message}`);
      }
    }

    if (!response.ok) {
      throw new Error(`HTTP ${response.status} ${response.statusText}`);
    }
    if (!payload || payload.ok !== true) {
      throw new Error(payload?.error || "unknown error");
    }
    return payload.result;
  } catch (err) {
    throw new Error(`Request ${method} ${url} failed: ${err.message}`);
  } finally {
    clearTimeout(timeoutId);
  }
}

function assertNoActiveRecording() {
  if (activeRecording) {
    throw new Error("Video recording already in progress");
  }
}

async function captureFrames(state) {
  let nextFrameAt = Date.now();

  while (!state.stopRequested) {
    const now = Date.now();
    if (now < nextFrameAt) {
      await sleep(nextFrameAt - now);
    }
    if (state.stopRequested) {
      break;
    }

    const frameName = `frame-${String(state.frameCount).padStart(6, "0")}.png`;
    const framePath = path.join(state.framesDir, frameName);
    await callJson(state.addr, "/screenshot", {
      method: "POST",
      body: { path: framePath },
      timeoutMs: state.requestTimeoutMs,
    });

    state.frameCount += 1;
    nextFrameAt += state.intervalMs;
    if (Date.now() - nextFrameAt > state.intervalMs) {
      nextFrameAt = Date.now();
    }
  }
}

function runCommand(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: "inherit", ...options });
    child.on("error", (err) => {
      if (err.code === "ENOENT") {
        reject(new Error(`${command} not found. Install ffmpeg (brew install ffmpeg).`));
        return;
      }
      reject(err);
    });
    child.on("close", (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} exited with code ${code}`));
    });
  });
}

async function encodeVideo(state) {
  await fs.promises.mkdir(path.dirname(state.outPath), { recursive: true });
  const args = [
    "-y",
    "-framerate",
    String(state.fps),
    "-i",
    state.framePattern,
    "-c:v",
    "libx264",
    "-pix_fmt",
    "yuv420p",
    "-crf",
    "18",
    "-preset",
    "veryfast",
    "-movflags",
    "+faststart",
    state.outPath,
  ];
  await runCommand("ffmpeg", args);
}

export async function startVideo(options) {
  assertNoActiveRecording();
  const outPath = String(options?.outPath || "").trim();
  const tmpDir = String(options?.tmpDir || "").trim();
  if (!outPath) {
    throw new Error("startVideo requires outPath");
  }
  if (!tmpDir) {
    throw new Error("startVideo requires tmpDir");
  }

  const fps = toPositiveInt(options?.fps ?? DEFAULT_FPS, "fps");
  const addr = normalizeAddr(options?.addr ?? DEFAULT_ADDR);
  const readyTimeoutMs = toPositiveInt(
    options?.readyTimeoutMs ?? DEFAULT_READY_TIMEOUT_MS,
    "readyTimeoutMs",
  );
  const requestTimeoutMs = toPositiveInt(
    options?.requestTimeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS,
    "requestTimeoutMs",
  );
  const cleanupFrames = options?.cleanupFrames !== false;

  const resolvedTmpDir = path.resolve(tmpDir);
  const resolvedOutPath = path.resolve(outPath);
  const sessionId = crypto.randomUUID();
  const framesDir = path.join(resolvedTmpDir, `ctx-native-frames-${sessionId}`);
  const framePattern = path.join(framesDir, "frame-%06d.png");

  await fs.promises.mkdir(framesDir, { recursive: true });

  const readyTimeout = Math.max(readyTimeoutMs + 5000, requestTimeoutMs);
  await callJson(addr, `/ready?timeout_ms=${readyTimeoutMs}`, {
    timeoutMs: readyTimeout,
  });

  const state = {
    addr,
    fps,
    intervalMs: 1000 / fps,
    framesDir,
    framePattern,
    outPath: resolvedOutPath,
    cleanupFrames,
    readyTimeoutMs,
    requestTimeoutMs,
    stopRequested: false,
    frameCount: 0,
    capturePromise: null,
  };

  state.capturePromise = captureFrames(state);
  activeRecording = state;

  return {
    outPath: resolvedOutPath,
    framesDir,
    fps,
  };
}

export async function stopVideo() {
  if (!activeRecording) {
    throw new Error("No active video recording");
  }

  const state = activeRecording;
  activeRecording = null;
  state.stopRequested = true;

  try {
    await state.capturePromise;
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    throw new Error(`Video capture failed: ${message}`);
  }

  if (state.frameCount === 0) {
    throw new Error("No frames captured");
  }

  await encodeVideo(state);

  if (state.cleanupFrames) {
    await fs.promises.rm(state.framesDir, { recursive: true, force: true });
  }

  return { outPath: state.outPath };
}

const isMain =
  path.resolve(process.argv[1] || "") === path.resolve(fileURLToPath(import.meta.url));

if (isMain) {
  const run = async () => {
    const args = parseArgs(process.argv.slice(2));
    if (args.showHelp) {
      printHelp();
      return;
    }

    if (!args.outPath) {
      printHelp();
      process.exitCode = 1;
      return;
    }

    await startVideo({
      outPath: args.outPath,
      fps: args.fps,
      tmpDir: args.tmpDir,
      addr: args.addr,
      readyTimeoutMs: args.readyTimeoutMs,
      requestTimeoutMs: args.requestTimeoutMs,
      cleanupFrames: !args.keepFrames,
    });

    if (args.durationMs !== null) {
      await sleep(args.durationMs);
      await stopVideo();
      return;
    }

    console.log("Recording... press Ctrl+C to stop.");
    await new Promise((resolve) => {
      const finish = async () => {
        try {
          await stopVideo();
        } catch (err) {
          console.error(err instanceof Error ? err.message : String(err));
          process.exitCode = 1;
        }
        resolve();
      };
      process.once("SIGINT", finish);
      process.once("SIGTERM", finish);
    });
  };

  run().catch((err) => {
    console.error(err instanceof Error ? err.message : String(err));
    process.exit(1);
  });
}
