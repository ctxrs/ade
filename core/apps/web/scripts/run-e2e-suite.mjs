#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  ensureLockedNodeInstall,
  ensurePlaywrightBrowserInstall,
  requireLocalNodeBin,
} from "./localTooling.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const webRoot = path.resolve(__dirname, "..");
const e2eRoot = path.join(webRoot, "e2e");
const suiteDir = path.join(e2eRoot, "suites");

const suite = process.argv[2];
const forwardedArgs = process.argv.slice(3);
if (forwardedArgs[0] === "--") {
  forwardedArgs.shift();
}

const suites = ["premerge_required", "release_required", "cross_platform", "visual", "soak", "load"];
const allSuites = new Set([...suites, "all"]);

if (!suite || !allSuites.has(suite)) {
  console.error(`usage: node scripts/run-e2e-suite.mjs <${[...allSuites].join("|")}> [playwright args...]`);
  process.exit(2);
}

const configBySuite = {
  all: "playwright.config.ts",
  premerge_required: "playwright.premerge.config.ts",
  release_required: "playwright.release.config.ts",
  cross_platform: "playwright.cross-platform.config.ts",
  visual: "playwright.visual.config.ts",
  soak: "playwright.soak.config.ts",
  load: "playwright.load.config.ts",
};

const toPosix = (value) => value.split(path.sep).join("/");

const normalizeSpec = (line) => {
  const cleaned = line.trim();
  if (!cleaned || cleaned.startsWith("#")) return null;
  const relative = cleaned.startsWith("e2e/") ? cleaned : `e2e/${cleaned}`;
  return toPosix(path.posix.normalize(relative));
};

const readSuite = (name) => {
  const manifestPath = path.join(suiteDir, `${name}.txt`);
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`missing suite manifest: ${manifestPath}`);
  }
  const parsed = fs
    .readFileSync(manifestPath, "utf8")
    .split(/\r?\n/)
    .map(normalizeSpec)
    .filter(Boolean);

  if (parsed.length === 0) {
    throw new Error(`suite '${name}' is empty: ${manifestPath}`);
  }

  const deduped = [...new Set(parsed)].sort();
  for (const spec of deduped) {
    const abs = path.join(webRoot, spec);
    if (!fs.existsSync(abs)) {
      throw new Error(`suite '${name}' references missing spec: ${spec}`);
    }
  }
  return deduped;
};

const specsBySuite = new Map();
for (const name of suites) {
  specsBySuite.set(name, readSuite(name));
}

const owners = new Map();
for (const name of suites) {
  for (const spec of specsBySuite.get(name)) {
    const curr = owners.get(spec) ?? [];
    curr.push(name);
    owners.set(spec, curr);
  }
}

const duplicateOwners = [...owners.entries()].filter(([, groups]) => groups.length > 1);
if (duplicateOwners.length > 0) {
  const detail = duplicateOwners
    .map(([spec, groups]) => `  - ${spec}: ${groups.join(", ")}`)
    .join("\n");
  throw new Error(`spec(s) assigned to multiple suites:\n${detail}`);
}

const allSpecs = fs
  .readdirSync(e2eRoot)
  .filter((entry) => entry.endsWith(".spec.ts"))
  .map((entry) => `e2e/${entry}`)
  .sort();

const unassigned = allSpecs.filter((spec) => !owners.has(spec));
if (unassigned.length > 0) {
  const detail = unassigned.map((spec) => `  - ${spec}`).join("\n");
  throw new Error(`spec(s) missing suite assignment:\n${detail}`);
}

const files = suite === "all" ? allSpecs : specsBySuite.get(suite);
console.error(`running suite '${suite}' with ${files.length} spec(s)`);

const configPath = configBySuite[suite];
ensureLockedNodeInstall(webRoot);
ensurePlaywrightBrowserInstall(webRoot);
const cmd = requireLocalNodeBin(webRoot, "playwright");
const args = ["test", "-c", configPath, ...files, ...forwardedArgs];

const result = spawnSync(cmd, args, {
  cwd: webRoot,
  env: process.env,
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error);
  process.exit(1);
}

process.exit(result.status ?? 1);
