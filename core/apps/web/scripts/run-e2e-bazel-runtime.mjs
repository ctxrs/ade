#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const supportedRuntimeProfiles = new Set(["workbench-lite", "agent-full", "web-artifact"]);
const suiteConfig = {
  premerge_required: "playwright.premerge.config.ts",
  release_required: "playwright.release.config.ts",
  cross_platform: "playwright.cross-platform.config.ts",
  visual: "playwright.visual.config.ts",
  soak: "playwright.soak.config.ts",
  load: "playwright.load.config.ts",
};

const usage = () => [
  "usage: node apps/web/scripts/run-e2e-bazel-runtime.mjs \\",
  "  --config <playwright.config.ts> \\",
  "  --runtime-profile <workbench-lite|agent-full|web-artifact> \\",
  "  --ctx-http-bin <path> \\",
  "  (--suite <suite>|--spec <e2e/spec.ts>) [--ctx-mcp-bin <path>] [-- <playwright args...>]",
].join("\n");

export const normalizeSpec = (value) => {
  const raw = String(value || "").trim();
  if (!raw) {
    throw new Error("empty E2E spec path");
  }
  const prefixed = raw.startsWith("e2e/") ? raw : `e2e/${raw}`;
  const normalized = path.posix.normalize(prefixed.replace(/\\/gu, "/"));
  if (!normalized.startsWith("e2e/") || !normalized.endsWith(".spec.ts")) {
    throw new Error(`invalid E2E spec path: ${value}`);
  }
  return normalized;
};

export const parseArgs = (argv) => {
  const parsed = {
    config: "",
    ctxHttpBin: "",
    ctxMcpBin: "",
    forwardedArgs: [],
    runtimeProfile: "",
    specs: [],
    suite: "",
  };
  const args = [...argv];
  while (args.length > 0) {
    const arg = args.shift();
    if (arg === "--") {
      parsed.forwardedArgs = args;
      break;
    }
    switch (arg) {
      case "--config":
        parsed.config = String(args.shift() || "").trim();
        break;
      case "--ctx-http-bin":
        parsed.ctxHttpBin = String(args.shift() || "").trim();
        break;
      case "--ctx-mcp-bin":
        parsed.ctxMcpBin = String(args.shift() || "").trim();
        break;
      case "--runtime-profile":
        parsed.runtimeProfile = String(args.shift() || "").trim();
        break;
      case "--spec":
        parsed.specs.push(normalizeSpec(args.shift()));
        break;
      case "--suite":
        parsed.suite = String(args.shift() || "").trim();
        break;
      default:
        throw new Error(`unsupported argument: ${arg}\n${usage()}`);
    }
  }
  if (!parsed.config && parsed.suite && suiteConfig[parsed.suite]) {
    parsed.config = suiteConfig[parsed.suite];
  }
  if (!parsed.config) {
    throw new Error(`missing --config\n${usage()}`);
  }
  if (!supportedRuntimeProfiles.has(parsed.runtimeProfile)) {
    throw new Error(`unsupported --runtime-profile: ${parsed.runtimeProfile}`);
  }
  if (!parsed.ctxHttpBin) {
    throw new Error("missing --ctx-http-bin");
  }
  if (parsed.runtimeProfile === "agent-full" && !parsed.ctxMcpBin) {
    throw new Error("agent-full web E2E runtime requires --ctx-mcp-bin");
  }
  if (!parsed.suite && parsed.specs.length === 0) {
    throw new Error("missing --suite or --spec");
  }
  return parsed;
};

const isRepoRoot = (candidate) =>
  fs.existsSync(path.join(candidate, "core", "package.json"))
  && fs.existsSync(path.join(candidate, "core", "apps", "web", "package.json"));

const normalizeRepoRootCandidate = (candidate) => {
  if (!candidate) return "";
  const resolved = path.resolve(candidate);
  if (isRepoRoot(resolved)) return resolved;
  if (path.basename(resolved) === "core" && isRepoRoot(path.dirname(resolved))) {
    return path.dirname(resolved);
  }
  return "";
};

const findRepoRootFrom = (startDir) => {
  let current = path.resolve(startDir);
  while (true) {
    const normalized = normalizeRepoRootCandidate(current);
    if (normalized) return normalized;
    const parent = path.dirname(current);
    if (parent === current) return "";
    current = parent;
  }
};

export const resolveRepoRoot = (env = process.env, cwd = process.cwd()) => {
  const candidates = [
    env.BUILD_WORKSPACE_DIRECTORY,
    env.CTX_REAL_WORKSPACE_ROOT,
    env.INIT_CWD,
    cwd,
    path.resolve(__dirname, "../../../.."),
    env.TEST_SRCDIR && env.TEST_WORKSPACE ? path.join(env.TEST_SRCDIR, env.TEST_WORKSPACE) : "",
    env.RUNFILES_DIR && env.TEST_WORKSPACE ? path.join(env.RUNFILES_DIR, env.TEST_WORKSPACE) : "",
    env.RUNFILES_DIR ? path.join(env.RUNFILES_DIR, "_main") : "",
  ];
  for (const candidate of candidates) {
    const normalized = normalizeRepoRootCandidate(candidate);
    if (normalized) return normalized;
  }
  for (const candidate of candidates.filter(Boolean)) {
    const found = findRepoRootFrom(candidate);
    if (found) return found;
  }
  throw new Error("failed to locate ctx repo root for web E2E runtime");
};

const binName = (tool) => (process.platform === "win32" ? `${tool}.cmd` : tool);

export const resolveLocalNodeBin = (packageRoot, tool) => {
  const expected = binName(tool);
  let current = path.resolve(packageRoot);
  while (true) {
    const candidate = path.join(current, "node_modules", ".bin", expected);
    if (fs.existsSync(candidate)) return candidate;
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  const expectedPath = path.join(path.resolve(packageRoot), "node_modules", ".bin", expected);
  throw new Error(
    `Missing local ${tool} binary at ${expectedPath}. Run pnpm install in core before running Bazel web E2E.`,
  );
};

export const resolveExistingPath = (configured, { cwd = process.cwd(), env = process.env, repoRoot } = {}) => {
  const raw = String(configured || "").trim();
  if (!raw) {
    throw new Error("missing path");
  }
  const candidates = [];
  if (path.isAbsolute(raw)) {
    candidates.push(raw);
  } else {
    candidates.push(
      path.resolve(cwd, raw),
      repoRoot ? path.resolve(repoRoot, raw) : "",
      env.TEST_SRCDIR && env.TEST_WORKSPACE ? path.join(env.TEST_SRCDIR, env.TEST_WORKSPACE, raw) : "",
      env.RUNFILES_DIR && env.TEST_WORKSPACE ? path.join(env.RUNFILES_DIR, env.TEST_WORKSPACE, raw) : "",
      env.RUNFILES_DIR ? path.join(env.RUNFILES_DIR, "_main", raw) : "",
    );
  }
  for (const candidate of candidates.filter(Boolean)) {
    if (fs.existsSync(candidate)) return path.resolve(candidate);
  }
  throw new Error(`declared Bazel runtime input does not exist: ${raw}`);
};

const readSuiteSpecs = (webRoot, suite) => {
  if (!suiteConfig[suite]) {
    throw new Error(`unsupported E2E suite: ${suite}`);
  }
  const manifestPath = path.join(webRoot, "e2e", "suites", `${suite}.txt`);
  const specs = fs
    .readFileSync(manifestPath, "utf8")
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .map(normalizeSpec);
  if (specs.length === 0) {
    throw new Error(`E2E suite is empty: ${suite}`);
  }
  return [...new Set(specs)].sort();
};

const resolveSpecs = (webRoot, { suite, specs }) => {
  const resolved = [
    ...(suite ? readSuiteSpecs(webRoot, suite) : []),
    ...specs,
  ];
  const deduped = [...new Set(resolved)].sort();
  for (const spec of deduped) {
    const abs = path.join(webRoot, spec);
    if (!fs.existsSync(abs)) {
      throw new Error(`E2E spec does not exist: ${spec}`);
    }
  }
  return deduped;
};

const ensureTempRoot = (env) => {
  const root = path.resolve(env.TEST_TMPDIR || env.CTX_E2E_TMPDIR || os.tmpdir());
  fs.mkdirSync(root, { recursive: true });
  return root;
};

const run = (command, args, options) => {
  const result = spawnSync(command, args, { ...options, stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
};

const buildWebDist = ({ env, runtimeProfile, tempRoot, viteBin, webRoot }) => {
  const distDir = path.join(tempRoot, `ctx-web-e2e-dist-${runtimeProfile}-${process.pid}`);
  fs.rmSync(distDir, { recursive: true, force: true });
  run(viteBin, ["build", "--outDir", distDir, "--emptyOutDir"], {
    cwd: webRoot,
    env,
  });
  return distDir;
};

export const buildPlaywrightEnv = ({
  ctxHttpBin,
  ctxMcpBin = "",
  env = process.env,
  runtimeProfile,
  tempRoot,
  webDistDir,
}) => {
  const e2eTmpDir = path.join(tempRoot, `ctx-e2e-${runtimeProfile}-tmp-${process.pid}`);
  const e2eDataDir = path.join(tempRoot, `ctx-e2e-${runtimeProfile}-data-${process.pid}`);
  const nextEnv = {
    ...env,
    CI: env.CI ?? "1",
    CTX_E2E_CTX_HTTP_BIN: ctxHttpBin,
    CTX_E2E_DATA_DIR: e2eDataDir,
    CTX_E2E_RUNTIME_PROFILE: runtimeProfile,
    CTX_E2E_RUNTIME_SOURCE: "bazel-runfiles",
    CTX_E2E_SKIP_WEB_BUILD: "1",
    CTX_E2E_TMPDIR: e2eTmpDir,
    CTX_E2E_WEB_DIST: webDistDir,
    CTX_VOLATILE_TMPDIR: env.CTX_VOLATILE_TMPDIR || tempRoot,
    TMP: e2eTmpDir,
    TEMP: e2eTmpDir,
    TMPDIR: e2eTmpDir,
  };
  delete nextEnv.CTX_MCP_DISABLED;
  if (runtimeProfile === "agent-full") {
    nextEnv.CTX_E2E_CTX_MCP_BIN = ctxMcpBin;
  } else {
    delete nextEnv.CTX_E2E_CTX_MCP_BIN;
    delete nextEnv.CTX_MCP_COMMAND;
  }
  return nextEnv;
};

export const buildPlaywrightArgs = ({ config, forwardedArgs = [], specs }) => [
  "test",
  "-c",
  config,
  ...specs,
  ...forwardedArgs,
];

export const runBazelRuntimeE2E = (argv, env = process.env) => {
  const options = parseArgs(argv);
  const repoRoot = resolveRepoRoot(env);
  const coreRoot = path.join(repoRoot, "core");
  const webRoot = path.join(coreRoot, "apps", "web");
  const tempRoot = ensureTempRoot(env);
  const ctxHttpBin = resolveExistingPath(options.ctxHttpBin, { env, repoRoot });
  const ctxMcpBin = options.ctxMcpBin
    ? resolveExistingPath(options.ctxMcpBin, { env, repoRoot })
    : "";
  const specs = resolveSpecs(webRoot, options);
  const viteBin = resolveLocalNodeBin(webRoot, "vite");
  const playwrightBin = resolveLocalNodeBin(webRoot, "playwright");
  const buildEnv = {
    ...env,
    TMP: tempRoot,
    TEMP: tempRoot,
    TMPDIR: tempRoot,
  };
  const webDistDir = buildWebDist({
    env: buildEnv,
    runtimeProfile: options.runtimeProfile,
    tempRoot,
    viteBin,
    webRoot,
  });
  const playwrightEnv = buildPlaywrightEnv({
    ctxHttpBin,
    ctxMcpBin,
    env,
    runtimeProfile: options.runtimeProfile,
    tempRoot,
    webDistDir,
  });
  const playwrightArgs = buildPlaywrightArgs({
    config: options.config,
    forwardedArgs: options.forwardedArgs,
    specs,
  });
  run(playwrightBin, playwrightArgs, {
    cwd: webRoot,
    env: playwrightEnv,
  });
};

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  try {
    runBazelRuntimeE2E(process.argv.slice(2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
