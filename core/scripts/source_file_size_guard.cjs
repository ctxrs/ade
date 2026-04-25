#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");

const MAX_LINES = 600;
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".rs"]);
const EXCLUDED_PARTS = new Set([
  ".ctx",
  ".git",
  "node_modules",
  "target",
  "dist",
  "build",
  ".next",
  "coverage",
  ".turbo",
  ".cache",
]);
const TEST_PATH_SEGMENTS = new Set(["tests", "__tests__", "e2e", "automation"]);
const REPO_ROOTS = new Set(["core"]);
const TEST_FILE_PATTERNS = [
  /\.test\.[^.]+$/u,
  /\.spec\.[^.]+$/u,
  /_test\.rs$/u,
  /(^|\/)(test|tests)\.[^.]+$/u,
];
const AUTOMATION_PATH_PATTERNS = [/^core\/scripts\//u, /^core\/apps\/[^/]+\/scripts\//u];
const PRODUCTION_ROOT_PATTERNS = [
  /^core\/apps\/web\/src\//u,
  /^core\/apps\/desktop\/src-tauri\/src\//u,
  /^core\/crates\/[^/]+\/src\//u,
  /^core\/packages\/[^/]+\/src\//u,
  /^core\/tools\/[^/]+\/src\//u,
];

const GUIDANCE_MESSAGE =
  "If you are receiving this error message, do not try to make small tweaks just to barely slip below the 600-line cap. Take the opportunity to pause, think through an architecturally sound split that will age well, and use that to bring the file back under the limit. This file is getting too big, which is usually a code smell pointing to a module with too many concerns. Break it up along clean responsibility boundaries instead of sharding it arbitrarily. We do not allow production-source exceptions to this hard cap.";

const toPosix = (value) => value.split(path.sep).join("/");

const isTestOrAutomationFile = (relativePath) => {
  const normalized = toPosix(relativePath);
  const parts = normalized.split("/");
  if (TEST_FILE_PATTERNS.some((pattern) => pattern.test(normalized))) return true;
  if (parts.some((part) => TEST_PATH_SEGMENTS.has(part))) return true;
  return AUTOMATION_PATH_PATTERNS.some((pattern) => pattern.test(normalized));
};

const classifySourceFile = (relativePath) => {
  const normalized = toPosix(relativePath);
  const parts = normalized.split("/");
  const [top] = parts;

  if (parts.some((part) => EXCLUDED_PARTS.has(part))) return "ignore";
  if (top === "external-harnesses") return "ignore";
  if (!SOURCE_EXTENSIONS.has(path.extname(normalized))) return "ignore";
  if (isTestOrAutomationFile(normalized)) return "test_or_automation";
  if (!REPO_ROOTS.has(top)) return "ignore";
  if (PRODUCTION_ROOT_PATTERNS.some((pattern) => pattern.test(normalized))) return "production";
  return "ignore";
};

const isTrackedSourceFile = (relativePath) => {
  return classifySourceFile(relativePath) === "production";
};

const countLines = (raw) => {
  if (raw.length === 0) return 0;
  const parts = raw.split(/\r?\n/u);
  return raw.endsWith("\n") ? parts.length - 1 : parts.length;
};

const walkFiles = (dirPath, output = []) => {
  for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
    if (EXCLUDED_PARTS.has(entry.name)) continue;
    const absolutePath = path.join(dirPath, entry.name);
    if (entry.isDirectory()) {
      walkFiles(absolutePath, output);
      continue;
    }
    output.push(absolutePath);
  }
  return output;
};

const listTrackedFiles = (rootDir) => {
  const raw = childProcess.execFileSync("git", ["-C", rootDir, "ls-files", "-z"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return raw
    .split("\0")
    .filter(Boolean)
    .map((relativePath) => path.join(rootDir, relativePath));
};

const collectSourceFileStats = (rootDir = repoRoot) => {
  const files = rootDir === repoRoot ? listTrackedFiles(rootDir) : walkFiles(rootDir);
  const stats = [];
  for (const absolutePath of files) {
    const relativePath = toPosix(path.relative(rootDir, absolutePath));
    if (!isTrackedSourceFile(relativePath)) continue;
    if (!fs.existsSync(absolutePath)) continue;
    const raw = fs.readFileSync(absolutePath, "utf8");
    stats.push({
      path: relativePath,
      absolutePath,
      lineCount: countLines(raw),
    });
  }
  stats.sort((lhs, rhs) => rhs.lineCount - lhs.lineCount || lhs.path.localeCompare(rhs.path));
  return stats;
};

const evaluateSourceFileSizes = ({
  files,
  maxLines = MAX_LINES,
}) => {
  const violations = [];

  for (const file of files) {
    if (file.lineCount <= maxLines) {
      continue;
    }

    violations.push({
      path: file.path,
      lineCount: file.lineCount,
      limit: maxLines,
    });
  }

  return { violations };
};

const formatViolation = (violation) =>
  `${violation.path} is ${violation.lineCount} lines (limit ${violation.limit}).`;

const printReport = ({
  violations,
  stream = process.stderr,
}) => {
  if (violations.length === 0) return;

  stream.write("source-file-size violations:\n");
  for (const violation of violations) {
    stream.write(`  - ${formatViolation(violation)}\n`);
  }
  stream.write("\n");
  stream.write(`${GUIDANCE_MESSAGE}\n\n`);
  stream.write(`No production-source exceptions are allowed. Refactor the file below ${MAX_LINES} lines.\n`);
};

const run = ({
  rootDir = repoRoot,
  enforce = false,
  stdout = process.stdout,
  stderr = process.stderr,
} = {}) => {
  const files = collectSourceFileStats(rootDir);
  const result = evaluateSourceFileSizes({ files });
  printReport({ ...result, stream: result.violations.length > 0 ? stderr : stdout });

  if (!enforce) return 0;
  return result.violations.length > 0 ? 1 : 0;
};

if (require.main === module) {
  const enforce = process.argv.includes("--enforce");
  process.exitCode = run({ enforce });
}

module.exports = {
  MAX_LINES,
  GUIDANCE_MESSAGE,
  collectSourceFileStats,
  countLines,
  evaluateSourceFileSizes,
  classifySourceFile,
  isTrackedSourceFile,
  isTestOrAutomationFile,
  run,
};
