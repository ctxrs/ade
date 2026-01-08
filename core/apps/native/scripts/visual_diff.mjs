#!/usr/bin/env node
"use strict";

import fs from "node:fs";
import path from "node:path";
import pixelmatch from "pixelmatch";
import pngjs from "pngjs";

const { PNG } = pngjs;

const args = process.argv.slice(2);
const getFlag = (flag) => {
  const idx = args.indexOf(flag);
  if (idx === -1) return undefined;
  return args[idx + 1];
};

const showUsage = () => {
  console.log(
    [
      "Usage: visual_diff.mjs --web <dir> --native <dir> --out <dir> [--threshold <ratio>] [--pixel-threshold <ratio>] [--report <path>]",
      "",
      "Defaults:",
      "  --threshold 0",
      "  --pixel-threshold 0.1",
      "  --report <out>/summary.json",
    ].join("\n"),
  );
};

if (args.includes("--help") || args.includes("-h")) {
  showUsage();
  process.exit(0);
}

const webDir = getFlag("--web");
const nativeDir = getFlag("--native");
const outDir = getFlag("--out");

if (!webDir || !nativeDir || !outDir) {
  showUsage();
  process.exit(1);
}

const threshold = Number(getFlag("--threshold") ?? "0");
const pixelThreshold = Number(getFlag("--pixel-threshold") ?? "0.1");
const reportPath = getFlag("--report") ?? path.join(outDir, "summary.json");

if (!Number.isFinite(threshold) || threshold < 0) {
  console.error("Invalid --threshold value.");
  process.exit(1);
}

if (!Number.isFinite(pixelThreshold) || pixelThreshold <= 0 || pixelThreshold > 1) {
  console.error("Invalid --pixel-threshold value.");
  process.exit(1);
}

const toPosix = (value) => value.split(path.sep).join("/");

const listPngFiles = async (rootDir) => {
  const files = new Map();
  const stack = [rootDir];

  while (stack.length > 0) {
    const current = stack.pop();
    const entries = await fs.promises.readdir(current, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(fullPath);
        continue;
      }
      if (!entry.isFile()) continue;
      if (entry.name.toLowerCase().endsWith(".png")) {
        const relPath = path.relative(rootDir, fullPath);
        files.set(toPosix(relPath), fullPath);
      }
    }
  }

  return files;
};

const padTo = (png, width, height) => {
  if (png.width === width && png.height === height) return png;
  const padded = new PNG({ width, height });
  PNG.bitblt(png, padded, 0, 0, png.width, png.height, 0, 0);
  return padded;
};

const ensureDir = async (dirPath) => {
  await fs.promises.mkdir(dirPath, { recursive: true });
};

const main = async () => {
  const resolvedWebDir = path.resolve(webDir);
  const resolvedNativeDir = path.resolve(nativeDir);
  const resolvedOutDir = path.resolve(outDir);

  const [webFiles, nativeFiles] = await Promise.all([
    listPngFiles(resolvedWebDir),
    listPngFiles(resolvedNativeDir),
  ]);

  const webKeys = new Set(webFiles.keys());
  const nativeKeys = new Set(nativeFiles.keys());

  const matched = [...webKeys].filter((key) => nativeKeys.has(key)).sort();
  const missingWeb = [...nativeKeys].filter((key) => !webKeys.has(key)).sort();
  const missingNative = [...webKeys].filter((key) => !nativeKeys.has(key)).sort();

  await ensureDir(resolvedOutDir);

  const results = [];
  let overallPass = missingWeb.length === 0 && missingNative.length === 0;

  for (const key of matched) {
    const webPath = webFiles.get(key);
    const nativePath = nativeFiles.get(key);
    const webPng = PNG.sync.read(fs.readFileSync(webPath));
    const nativePng = PNG.sync.read(fs.readFileSync(nativePath));

    const width = Math.max(webPng.width, nativePng.width);
    const height = Math.max(webPng.height, nativePng.height);
    const sizeMismatch = webPng.width !== nativePng.width || webPng.height !== nativePng.height;

    const webPadded = padTo(webPng, width, height);
    const nativePadded = padTo(nativePng, width, height);
    const diff = new PNG({ width, height });

    const diffPixels = pixelmatch(
      webPadded.data,
      nativePadded.data,
      diff.data,
      width,
      height,
      { threshold: pixelThreshold },
    );
    const diffRatio = diffPixels / (width * height);
    const pass = !sizeMismatch && diffRatio <= threshold;
    if (!pass) overallPass = false;

    const parsed = path.parse(key);
    const diffRelPath = path.join(parsed.dir, `${parsed.name}.diff${parsed.ext}`);
    const diffOutPath = path.join(resolvedOutDir, diffRelPath);
    await ensureDir(path.dirname(diffOutPath));
    fs.writeFileSync(diffOutPath, PNG.sync.write(diff));

    results.push({
      file: key,
      diffPixels,
      diffRatio,
      pass,
      sizeMismatch,
      webSize: { width: webPng.width, height: webPng.height },
      nativeSize: { width: nativePng.width, height: nativePng.height },
      diffPath: toPosix(path.relative(resolvedOutDir, diffOutPath)),
    });
  }

  const summary = {
    webDir: resolvedWebDir,
    nativeDir: resolvedNativeDir,
    outDir: resolvedOutDir,
    threshold,
    pixelThreshold,
    pass: overallPass,
    totals: {
      web: webFiles.size,
      native: nativeFiles.size,
      matched: matched.length,
      missingWeb: missingWeb.length,
      missingNative: missingNative.length,
    },
    missing: {
      web: missingWeb,
      native: missingNative,
    },
    files: results,
  };

  await ensureDir(path.dirname(reportPath));
  await fs.promises.writeFile(reportPath, `${JSON.stringify(summary, null, 2)}\n`);

  console.log(
    [
      `Matched: ${matched.length}`,
      `Missing web: ${missingWeb.length}`,
      `Missing native: ${missingNative.length}`,
      `Pass: ${overallPass}`,
      `Report: ${reportPath}`,
    ].join(" | "),
  );

  process.exitCode = overallPass ? 0 : 1;
};

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
