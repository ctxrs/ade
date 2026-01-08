#!/usr/bin/env node
import fs from "node:fs/promises";
import path from "node:path";
import pixelmatch from "pixelmatch";
import { PNG } from "pngjs";

const DEFAULTS = {
  outDir: "diffs",
  threshold: 0.01,
  pixelThreshold: 0.1,
};

const USAGE = `
Usage:
  node core/apps/native/scripts/visual-diff.mjs <webDir> <nativeDir> [options]

Options:
  --out <dir>             Output directory (default: ${DEFAULTS.outDir})
  --threshold <ratio>     Max diff ratio to pass (default: ${DEFAULTS.threshold})
  --pixel-threshold <n>   Pixelmatch threshold (default: ${DEFAULTS.pixelThreshold})
  -h, --help              Show this help text
`;

function parseArgs(argv) {
  const opts = { ...DEFAULTS };
  const positional = [];

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    if (arg === "--out") {
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) {
        throw new Error("--out requires a value");
      }
      opts.outDir = next;
      i += 1;
      continue;
    }
    if (arg === "--threshold") {
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) {
        throw new Error("--threshold requires a value");
      }
      opts.threshold = Number.parseFloat(next);
      i += 1;
      continue;
    }
    if (arg === "--pixel-threshold") {
      const next = argv[i + 1];
      if (!next || next.startsWith("--")) {
        throw new Error("--pixel-threshold requires a value");
      }
      opts.pixelThreshold = Number.parseFloat(next);
      i += 1;
      continue;
    }
    if (arg.startsWith("--")) {
      throw new Error(`Unknown option: ${arg}`);
    }
    positional.push(arg);
  }

  return { ...opts, positional };
}

function ensureNumber(value, label) {
  if (!Number.isFinite(value) || value < 0 || value > 1) {
    throw new Error(`${label} must be a finite number between 0 and 1`);
  }
}

async function assertDirectory(dirPath, label) {
  const stat = await fs.stat(dirPath).catch(() => null);
  if (!stat || !stat.isDirectory()) {
    throw new Error(`${label} directory not found: ${dirPath}`);
  }
}

async function collectPngFiles(rootDir) {
  const files = [];

  async function walk(currentDir) {
    const entries = await fs.readdir(currentDir, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(currentDir, entry.name);
      if (entry.isDirectory()) {
        await walk(fullPath);
        continue;
      }
      if (entry.isFile() && path.extname(entry.name).toLowerCase() === ".png") {
        files.push({
          name: entry.name,
          absPath: fullPath,
          relPath: path.relative(rootDir, fullPath),
        });
      }
    }
  }

  await walk(rootDir);
  return files;
}

function mapByName(files) {
  const map = new Map();
  for (const file of files) {
    const list = map.get(file.name) ?? [];
    list.push(file);
    map.set(file.name, list);
  }
  return map;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || args.positional.length < 2) {
    console.log(USAGE.trim());
    process.exitCode = 1;
    return;
  }

  const [webDirInput, nativeDirInput] = args.positional;
  const webDir = path.resolve(webDirInput);
  const nativeDir = path.resolve(nativeDirInput);
  const outDir = path.resolve(args.outDir);
  const diffsDir = path.join(outDir, "diffs");

  ensureNumber(args.threshold, "threshold");
  ensureNumber(args.pixelThreshold, "pixel-threshold");

  await assertDirectory(webDir, "Web");
  await assertDirectory(nativeDir, "Native");
  await fs.mkdir(diffsDir, { recursive: true });

  const webFiles = await collectPngFiles(webDir);
  const nativeFiles = await collectPngFiles(nativeDir);
  const webMap = mapByName(webFiles);
  const nativeMap = mapByName(nativeFiles);
  const allNames = new Set([...webMap.keys(), ...nativeMap.keys()]);

  const comparisons = [];
  const missing = [];
  const ambiguous = [];
  const sizeMismatch = [];

  for (const name of Array.from(allNames).sort()) {
    const webList = webMap.get(name) ?? [];
    const nativeList = nativeMap.get(name) ?? [];

    if (webList.length === 0 || nativeList.length === 0) {
      missing.push({
        name,
        missing: webList.length === 0 ? "web" : "native",
        webPaths: webList.map((file) => file.relPath),
        nativePaths: nativeList.map((file) => file.relPath),
      });
      continue;
    }

    if (webList.length > 1 || nativeList.length > 1) {
      ambiguous.push({
        name,
        webPaths: webList.map((file) => file.relPath),
        nativePaths: nativeList.map((file) => file.relPath),
      });
      continue;
    }

    const webFile = webList[0];
    const nativeFile = nativeList[0];
    const webPng = PNG.sync.read(await fs.readFile(webFile.absPath));
    const nativePng = PNG.sync.read(await fs.readFile(nativeFile.absPath));

    if (webPng.width !== nativePng.width || webPng.height !== nativePng.height) {
      sizeMismatch.push({
        name,
        webPath: webFile.relPath,
        nativePath: nativeFile.relPath,
        webSize: { width: webPng.width, height: webPng.height },
        nativeSize: { width: nativePng.width, height: nativePng.height },
      });
      continue;
    }

    const diff = new PNG({ width: webPng.width, height: webPng.height });
    const diffPixels = pixelmatch(
      webPng.data,
      nativePng.data,
      diff.data,
      webPng.width,
      webPng.height,
      { threshold: args.pixelThreshold },
    );
    const totalPixels = webPng.width * webPng.height;
    const diffRatio = totalPixels === 0 ? 0 : diffPixels / totalPixels;
    const pass = diffRatio <= args.threshold;
    const diffFileName = `${path.parse(name).name}.diff.png`;
    const diffPath = path.join(diffsDir, diffFileName);

    await fs.writeFile(diffPath, PNG.sync.write(diff));

    comparisons.push({
      name,
      webPath: webFile.relPath,
      nativePath: nativeFile.relPath,
      diffPixels,
      diffRatio,
      pass,
      diffImage: path.relative(outDir, diffPath),
    });
  }

  const passed = comparisons.filter((entry) => entry.pass).length;
  const failed = comparisons.length - passed;
  const overallPass =
    comparisons.length > 0 &&
    failed === 0 &&
    missing.length === 0 &&
    ambiguous.length === 0 &&
    sizeMismatch.length === 0;

  const report = {
    generatedAt: new Date().toISOString(),
    webDir,
    nativeDir,
    outDir,
    threshold: args.threshold,
    pixelThreshold: args.pixelThreshold,
    totals: {
      webFiles: webFiles.length,
      nativeFiles: nativeFiles.length,
      compared: comparisons.length,
      passed,
      failed,
      missing: missing.length,
      ambiguous: ambiguous.length,
      sizeMismatch: sizeMismatch.length,
    },
    overallPass,
    comparisons,
    missing,
    ambiguous,
    sizeMismatch,
  };

  const reportPath = path.join(outDir, "summary.json");
  await fs.writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);

  console.log(
    `Compared ${comparisons.length} image(s). Passed: ${passed}. Failed: ${failed}. Overall pass: ${overallPass}.`,
  );
  console.log(`Report: ${reportPath}`);
  if (!overallPass) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
