#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const exceptionsPath = path.join(__dirname, "source_file_size_exceptions.json");

const SOFT_MAX_LINES = 800;
const HARD_MAX_LINES = 1200;
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx", ".rs"]);
const EXCLUDED_PARTS = new Set(["node_modules", "target", "dist", "build", ".next", "coverage", ".turbo"]);
const TEST_FILE_PATTERNS = [/\.test\.tsx?$/u, /_test\.rs$/u, /(^|\/)tests\.rs$/u];

const GUIDANCE_MESSAGE =
  'This file is getting too big. Consider if that is a code smell pointing to a deeper architectural issue with a module having too many concerns. If you absolutely must keep it, you can add it as an exception. But consider if you can break it up. We do not want to break things up just to break them up though. We should have clean responsibilities, good architecture, and "just works" mentality.';

const toPosix = (value) => value.split(path.sep).join("/");

const isTestFile = (relativePath) => TEST_FILE_PATTERNS.some((pattern) => pattern.test(relativePath));

const isTrackedSourceFile = (relativePath) => {
  const normalized = toPosix(relativePath);
  const parts = normalized.split("/");
  if (parts.some((part) => EXCLUDED_PARTS.has(part))) return false;
  if (isTestFile(normalized)) return false;
  if (parts.length < 3) return false;
  if (!SOURCE_EXTENSIONS.has(path.extname(normalized))) return false;
  const [top] = parts;
  if (!new Set(["apps", "crates", "packages", "tools"]).has(top)) return false;
  return parts.includes("src");
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

const loadExceptions = (filePath = exceptionsPath) => {
  const raw = fs.readFileSync(filePath, "utf8");
  const parsed = JSON.parse(raw);
  const byPath = new Map();
  for (const entry of parsed) {
    const normalizedPath = toPosix(String(entry.path ?? "").trim());
    if (!normalizedPath) {
      throw new Error(`source file size exception entry is missing path in ${filePath}`);
    }
    const maxLines = Number(entry.maxLines);
    if (!Number.isInteger(maxLines) || maxLines <= 0) {
      throw new Error(`source file size exception entry for ${normalizedPath} has invalid maxLines`);
    }
    byPath.set(normalizedPath, {
      path: normalizedPath,
      maxLines,
      reason: String(entry.reason ?? "").trim(),
    });
  }
  return byPath;
};

const collectSourceFileStats = (rootDir = coreRoot) => {
  const files = walkFiles(rootDir);
  const stats = [];
  for (const absolutePath of files) {
    const relativePath = toPosix(path.relative(rootDir, absolutePath));
    if (!isTrackedSourceFile(relativePath)) continue;
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
  exceptions,
  softMaxLines = SOFT_MAX_LINES,
  hardMaxLines = HARD_MAX_LINES,
}) => {
  const warnings = [];
  const violations = [];

  for (const file of files) {
    const exception = exceptions.get(file.path);

    if (file.lineCount > softMaxLines && !exception) {
      warnings.push({
        type: "soft_limit",
        path: file.path,
        lineCount: file.lineCount,
        limit: softMaxLines,
      });
    }

    if (file.lineCount <= hardMaxLines) {
      continue;
    }

    if (!exception) {
      violations.push({
        type: "missing_exception",
        path: file.path,
        lineCount: file.lineCount,
        limit: hardMaxLines,
      });
      continue;
    }

    if (file.lineCount > exception.maxLines) {
      violations.push({
        type: "exception_exceeded",
        path: file.path,
        lineCount: file.lineCount,
        limit: exception.maxLines,
        reason: exception.reason,
      });
    }
  }

  const staleExceptions = [];
  for (const [exceptionPath, exception] of exceptions.entries()) {
    if (!files.some((file) => file.path === exceptionPath)) {
      staleExceptions.push({
        path: exceptionPath,
        limit: exception.maxLines,
        reason: exception.reason,
      });
    }
  }

  return { warnings, violations, staleExceptions };
};

const formatWarning = (warning) =>
  `warning: ${warning.path} is ${warning.lineCount} lines (soft limit ${warning.limit})`;

const formatViolation = (violation) => {
  if (violation.type === "missing_exception") {
    return `${violation.path} is ${violation.lineCount} lines (hard limit ${violation.limit}) and has no exception entry.`;
  }
  return `${violation.path} is ${violation.lineCount} lines but its exception cap is ${violation.limit}.`;
};

const printReport = ({
  warnings,
  violations,
  staleExceptions,
  stream = process.stderr,
}) => {
  if (warnings.length > 0) {
    stream.write("source-file-size warnings:\n");
    for (const warning of warnings) {
      stream.write(`  - ${formatWarning(warning)}\n`);
    }
    stream.write("\n");
  }

  if (staleExceptions.length > 0) {
    stream.write("source-file-size stale exceptions:\n");
    for (const stale of staleExceptions) {
      stream.write(`  - ${stale.path} no longer matches a tracked source file\n`);
    }
    stream.write("\n");
  }

  if (violations.length === 0) return;

  stream.write("source-file-size violations:\n");
  for (const violation of violations) {
    stream.write(`  - ${formatViolation(violation)}\n`);
  }
  stream.write("\n");
  stream.write(`${GUIDANCE_MESSAGE}\n\n`);
  stream.write(`If you need an exception, update ${toPosix(path.relative(coreRoot, exceptionsPath))} with a deliberate maxLines cap and rationale.\n`);
};

const run = ({
  rootDir = coreRoot,
  exceptionsFilePath = exceptionsPath,
  enforce = false,
  stdout = process.stdout,
  stderr = process.stderr,
} = {}) => {
  let exceptions;
  try {
    exceptions = loadExceptions(exceptionsFilePath);
  } catch (error) {
    stderr.write(`source-file-size guard failed to load exceptions: ${error.message}\n`);
    return 1;
  }

  const files = collectSourceFileStats(rootDir);
  const result = evaluateSourceFileSizes({ files, exceptions });
  printReport({ ...result, stream: result.violations.length > 0 ? stderr : stdout });

  if (!enforce) return 0;
  return result.violations.length > 0 ? 1 : 0;
};

if (require.main === module) {
  const enforce = process.argv.includes("--enforce");
  process.exitCode = run({ enforce });
}

module.exports = {
  SOFT_MAX_LINES,
  HARD_MAX_LINES,
  GUIDANCE_MESSAGE,
  collectSourceFileStats,
  countLines,
  evaluateSourceFileSizes,
  isTrackedSourceFile,
  loadExceptions,
  run,
};
