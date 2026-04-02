#!/usr/bin/env node
import fs from "node:fs/promises";
import { existsSync, mkdirSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

import {
  ensurePlaywrightCoreRuntime,
  GOOGLE_CHROME_EXECUTABLE,
} from "./demo_hn_mobile_artifact_lib.mjs";

const DEFAULT_EDITOR_URL = "https://app.rotato.app/app/edit/iphone-15-still?mode=video";
const DEFAULT_RESOLUTION = "4K";
const DEFAULT_QUALITY = "Very High quality - big file";
const DEFAULT_THEME = "Midnight";
const DEFAULT_CANVAS_PRESET = "Tall Portrait";

function parseArgs(argv) {
  const options = {
    canvasPreset: DEFAULT_CANVAS_PRESET,
    chromeExecutable: GOOGLE_CHROME_EXECUTABLE,
    cookiesPath: "",
    editorUrl: DEFAULT_EDITOR_URL,
    outputPath: "",
    quality: DEFAULT_QUALITY,
    rawPath: "",
    resolution: DEFAULT_RESOLUTION,
    theme: DEFAULT_THEME,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--cookies") {
      options.cookiesPath = path.resolve(next);
      index += 1;
    } else if (arg === "--editor-url") {
      options.editorUrl = next;
      index += 1;
    } else if (arg === "--chrome-executable") {
      options.chromeExecutable = next;
      index += 1;
    } else if (arg === "--canvas-preset") {
      options.canvasPreset = next;
      index += 1;
    } else if (arg === "--theme") {
      options.theme = next;
      index += 1;
    } else if (arg === "--resolution") {
      options.resolution = next;
      index += 1;
    } else if (arg === "--quality") {
      options.quality = next;
      index += 1;
    } else if (!options.rawPath) {
      options.rawPath = path.resolve(arg);
    } else if (!options.outputPath) {
      options.outputPath = path.resolve(arg);
    } else {
      throw new Error(`Unexpected argument: ${arg}`);
    }
  }

  if (!options.rawPath) {
    throw new Error("usage: node demo_hn_mobile_rotato_export.mjs --cookies <cookies.json> [--editor-url <url>] <input.mov> <output.mp4>");
  }
  if (!options.outputPath) {
    throw new Error("output path is required");
  }
  if (!options.cookiesPath) {
    throw new Error("--cookies is required");
  }
  if (!existsSync(options.cookiesPath)) {
    throw new Error(`missing cookies file: ${options.cookiesPath}`);
  }
  if (!existsSync(options.rawPath)) {
    throw new Error(`missing source clip: ${options.rawPath}`);
  }
  if (!existsSync(options.chromeExecutable)) {
    throw new Error(`missing Chrome executable: ${options.chromeExecutable}`);
  }

  return options;
}

function getDurationSeconds(videoPath) {
  const raw = execFileSync(
    "ffprobe",
    [
      "-v",
      "error",
      "-show_entries",
      "format=duration",
      "-of",
      "default=noprint_wrappers=1:nokey=1",
      videoPath,
    ],
    { encoding: "utf8" },
  ).trim();
  const duration = Number(raw);
  if (!Number.isFinite(duration) || duration <= 0) {
    throw new Error(`Could not read duration for ${videoPath}`);
  }
  return duration;
}

async function dismissOnboarding(page) {
  for (let index = 0; index < 8; index += 1) {
    const nextButton = page.getByRole("button", { name: "Next" });
    if (!(await nextButton.count())) {
      return;
    }

    for (const label of [
      /Internal presentations|Client presentations|My portfolio|Social media/,
      /Weekly|Rarely|Daily|Monthly/,
    ]) {
      const button = page.getByRole("button", { name: label }).first();
      if (await button.count()) {
        await button.click().catch(() => {});
        break;
      }
    }

    await nextButton.click().catch(() => {});
    await page.waitForTimeout(800);
  }
}

async function setCanvasPreset(page, presetName) {
  await page.getByRole("combobox").first().click({ force: true });
  await page.waitForTimeout(400);
  await page.getByText(presetName, { exact: true }).click({ timeout: 10_000 });
  await page.waitForTimeout(1200);
}

async function setTotalDuration(page, seconds) {
  const totalInput = page.locator("input[type=\"text\"]").nth(1);
  await totalInput.click({ force: true });
  await totalInput.fill(seconds.toFixed(2));
  await totalInput.press("Enter");
  await page.waitForTimeout(1200);
}

async function setTheme(page, themeName) {
  await page.getByRole("button", { name: "Colors" }).click({ timeout: 10_000 });
  await page.waitForTimeout(300);
  await page.locator(`button[title="${themeName}"]`).click({ timeout: 10_000 });
  await page.waitForTimeout(800);
}

async function applyNewestLibraryAsset(page) {
  const tileCenter = await page.evaluate(() => {
    const label = [...document.querySelectorAll("span")]
      .find((node) => (node.textContent || "").trim() === "Asset Library");
    const root = label?.closest(".space-y-3");
    const tiles = root ? [...root.querySelectorAll('div[role="button"]')] : [];
    const newest = tiles.at(-1);
    if (!newest) {
      return null;
    }
    const rect = newest.getBoundingClientRect();
    return {
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2,
    };
  });

  if (!tileCenter) {
    throw new Error("Could not find uploaded asset tile in Rotato asset library");
  }

  await page.mouse.click(tileCenter.x, tileCenter.y);
  await page.waitForTimeout(1200);
}

async function exportMovie(page, resolution, quality) {
  await page.getByRole("button", { name: "Export Video" }).click({ timeout: 10_000 });
  await page.waitForTimeout(400);
  await page.getByText("Standard", { exact: true }).click({ timeout: 10_000 });
  await page.waitForTimeout(250);
  await page.getByText(resolution, { exact: true }).click({ timeout: 10_000 });
  await page.waitForTimeout(250);
  await page.getByText("High quality", { exact: true }).click({ timeout: 10_000 });
  await page.waitForTimeout(250);
  await page.getByText(quality, { exact: true }).click({ timeout: 10_000 });
  await page.waitForTimeout(250);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const runtimeDir = path.join(os.tmpdir(), "ctx-hn-mobile-rotato-runtime");
  const durationSeconds = getDurationSeconds(options.rawPath);
  const cookies = JSON.parse(await fs.readFile(options.cookiesPath, "utf8"));
  const debugDir = path.join(os.tmpdir(), "ctx-hn-mobile-rotato-debug");
  mkdirSync(debugDir, { recursive: true });
  mkdirSync(path.dirname(options.outputPath), { recursive: true });

  ensurePlaywrightCoreRuntime(runtimeDir);
  const playwrightEntry = path.join(runtimeDir, "node_modules", "playwright-core", "index.mjs");
  const { chromium } = await import(pathToFileURL(playwrightEntry).href);

  const browser = await chromium.launch({
    executablePath: options.chromeExecutable,
    headless: true,
    args: ["--no-sandbox"],
  });

  try {
    const context = await browser.newContext({
      viewport: { width: 1600, height: 1200 },
      acceptDownloads: true,
    });
    await context.addCookies(cookies);
    const page = await context.newPage();

    await page.goto(options.editorUrl, {
      waitUntil: "domcontentloaded",
      timeout: 60_000,
    });
    await page.waitForTimeout(2500);
    await dismissOnboarding(page);
    await page.locator("input[type=\"file\"]").first().setInputFiles(options.rawPath);
    await page.waitForTimeout(10_000);
    await dismissOnboarding(page);
    await applyNewestLibraryAsset(page);
    await setCanvasPreset(page, options.canvasPreset);
    await setTotalDuration(page, durationSeconds);
    await setTheme(page, options.theme);
    await exportMovie(page, options.resolution, options.quality);

    const downloadPromise = page.waitForEvent("download", { timeout: 600_000 });
    await page.getByRole("button", { name: "Export MP4" }).click({ timeout: 10_000 });
    await page.waitForTimeout(45_000);
    await page.screenshot({
      path: path.join(debugDir, "rotato-render-modal.png"),
      fullPage: true,
    });

    const modalDownload = page.getByRole("button", { name: "Download MP4" });
    await modalDownload.waitFor({ timeout: 600_000 });
    await page.waitForTimeout(500);
    await modalDownload.click({ timeout: 10_000 });
    const download = await downloadPromise;
    await download.saveAs(options.outputPath);
  } finally {
    await browser.close();
  }

  process.stdout.write(`${JSON.stringify({
    status: "ok",
    output: options.outputPath,
    editor_url: options.editorUrl,
    duration_seconds: durationSeconds,
  }, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
