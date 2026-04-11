#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import {
  ensureLockedNodeInstall,
  ensurePlaywrightBrowserInstall,
  requireLocalNodeBin,
} from "./localTooling.mjs";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const require = createRequire(import.meta.url);
const { buildCtxCacheEnv } = require("../../../scripts/lib/cache_roots.cjs");
const coreRoot = path.resolve(__dirname, "../../..");
const webRoot = path.resolve(__dirname, "..");
const e2eRoot = path.join(webRoot, "e2e");
const suiteDir = path.join(e2eRoot, "suites");
export const suites = ["premerge_required", "release_required", "cross_platform", "visual", "soak", "load"];
const allSuites = new Set([...suites, "all"]);

const configBySuite = {
  all: "playwright.config.ts",
  premerge_required: "playwright.premerge.config.ts",
  release_required: "playwright.release.config.ts",
  cross_platform: "playwright.cross-platform.config.ts",
  visual: "playwright.visual.config.ts",
  soak: "playwright.soak.config.ts",
  load: "playwright.load.config.ts",
};

export const browsersBySuite = {
  all: ["webkit", "chromium"],
  premerge_required: ["webkit"],
  release_required: ["webkit"],
  cross_platform: ["webkit", "chromium"],
  visual: ["webkit"],
  soak: ["webkit", "chromium"],
  load: ["webkit", "chromium"],
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

export const usage = () =>
  `usage: node scripts/run-e2e-suite.mjs <${[...allSuites].join("|")}> [playwright args...]`;

export const normalizeForwardedArgs = (args) => {
  const forwardedArgs = [...args];
  if (forwardedArgs[0] === "--") {
    forwardedArgs.shift();
  }
  return forwardedArgs;
};

export const buildSuiteRunnerEnv = (env = process.env) =>
  buildCtxCacheEnv({
    cwd: coreRoot,
    env,
    mode: "workspace",
    mkdir: true,
  }).env;

export const runSuite = (
  suite,
  forwardedArgs = [],
  {
    env = process.env,
    ensureLockedNodeInstallImpl = ensureLockedNodeInstall,
    ensurePlaywrightBrowserInstallImpl = ensurePlaywrightBrowserInstall,
    requireLocalNodeBinImpl = requireLocalNodeBin,
    spawnSyncImpl = spawnSync,
    stderr = console.error,
  } = {},
) => {
  if (!suite || !allSuites.has(suite)) {
    stderr(usage());
    return 2;
  }

  const files = suite === "all" ? allSpecs : specsBySuite.get(suite);
  stderr(`running suite '${suite}' with ${files.length} spec(s)`);

  const suiteEnv = buildSuiteRunnerEnv(env);
  const configPath = configBySuite[suite];
  ensureLockedNodeInstallImpl(webRoot, { env: suiteEnv, requiredBins: ["playwright"] });
  ensurePlaywrightBrowserInstallImpl(webRoot, { browsers: browsersBySuite[suite], env: suiteEnv });
  const cmd = requireLocalNodeBinImpl(webRoot, "playwright");
  const args = ["test", "-c", configPath, ...files, ...normalizeForwardedArgs(forwardedArgs)];

  const result = spawnSyncImpl(cmd, args, {
    cwd: webRoot,
    env: suiteEnv,
    stdio: "inherit",
  });

  if (result.error) {
    stderr(result.error);
    return 1;
  }

  return result.status ?? 1;
};

const main = () => {
  const suite = process.argv[2];
  const forwardedArgs = process.argv.slice(3);
  process.exit(runSuite(suite, forwardedArgs));
};

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  main();
}
