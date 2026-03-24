#!/usr/bin/env node

/**
 * Rust runtime panic-trap enforcement.
 *
 * Scans all production Rust source files for `unwrap()` and `expect()` calls
 * outside of `#[cfg(test)]` blocks.
 *
 * Usage:
 *   node scripts/check_rust_runtime_panic_traps.mjs            # report only
 *   node scripts/check_rust_runtime_panic_traps.mjs --enforce  # fail on any hit
 *
 * The scan covers every tracked .rs file under the production source roots
 * (crates/, apps/desktop/src-tauri/src/, apps/tauri-mobile/src-tauri/src/).
 * Test files and files outside those runtime source roots are excluded.
 */

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

function parseArgs(argv) {
  const out = {
    format: "table",
    enforce: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--format") {
      out.format = String(argv[i + 1] || "table");
      i += 1;
    } else if (arg === "--enforce") {
      out.enforce = true;
    }
  }
  if (out.format !== "table" && out.format !== "json") {
    throw new Error(`Unsupported --format value: ${out.format}`);
  }
  return out;
}

const PRODUCTION_ROOT_PATTERNS = [
  /^crates\/[^/]+\/src\//u,
  /^apps\/desktop\/src-tauri\/src\//u,
  /^apps\/tauri-mobile\/src-tauri\/src\//u,
];

const TEST_PATH_SEGMENTS = new Set(["tests", "test", "test_support"]);

const TEST_FILE_PATTERNS = [
  /_tests?\.rs$/u,
  /^(test|tests)\.rs$/u,
  /test[_-]support\.rs$/u,
  /test[_-]server\.rs$/u,
];

function isProductionRustFile(relPath) {
  if (!relPath.endsWith(".rs")) return false;
  if (!PRODUCTION_ROOT_PATTERNS.some((re) => re.test(relPath))) return false;

  const parts = relPath.split("/");
  if (parts.some((part) => TEST_PATH_SEGMENTS.has(part))) return false;
  const basename = parts[parts.length - 1];
  if (TEST_FILE_PATTERNS.some((re) => re.test(basename))) return false;

  return true;
}

function listGitTrackedFiles(coreRoot) {
  const raw = execFileSync("git", ["-C", coreRoot, "ls-files", "-z"], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return raw.split("\0").filter(Boolean);
}

function discoverProductionRustFiles(coreRoot) {
  return listGitTrackedFiles(coreRoot).filter(isProductionRustFile).sort();
}

const PANIC_CALL_PATTERN = /\b(?:unwrap|expect)\s*\(/;
const CFG_TEST_PATTERN = /^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/;
const ATTRIBUTE_PATTERN = /^\s*#\s*\[/;
const EXCEPTION_COMMENT_PATTERN = /\/\/\s*EXCEPTION:\s*panic-trap\b/i;

function braceDelta(line) {
  let delta = 0;
  for (const ch of line) {
    if (ch === "{") delta += 1;
    else if (ch === "}") delta -= 1;
  }
  return delta;
}

function findRuntimeViolations(sourceText) {
  const violations = [];
  const lines = sourceText.split(/\r?\n/);
  let pendingCfgTestItem = false;
  let skipDepth = 0;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    const trimmed = line.trim();

    if (skipDepth > 0) {
      skipDepth += braceDelta(line);
      if (skipDepth <= 0) skipDepth = 0;
      continue;
    }

    if (pendingCfgTestItem) {
      if (trimmed === "") continue;
      if (ATTRIBUTE_PATTERN.test(trimmed) && !trimmed.includes("{")) continue;
      if (trimmed.includes("{")) {
        skipDepth = Math.max(braceDelta(line), 0);
        pendingCfgTestItem = false;
        continue;
      }
      if (trimmed.endsWith(";")) {
        pendingCfgTestItem = false;
        continue;
      }
      continue;
    }

    if (CFG_TEST_PATTERN.test(line)) {
      const tail = line.replace(CFG_TEST_PATTERN, "").trim();
      if (tail.length === 0) {
        pendingCfgTestItem = true;
      } else {
        skipDepth = Math.max(braceDelta(tail), 0);
      }
      continue;
    }

    if (PANIC_CALL_PATTERN.test(line)) {
      if (EXCEPTION_COMMENT_PATTERN.test(line)) continue;

      violations.push({
        line: index + 1,
        snippet: trimmed,
      });
    }
  }

  return violations;
}

function scanAllFiles(coreRoot, files) {
  const perFile = {};
  let totalUnwrap = 0;
  let totalExpect = 0;
  let totalViolations = 0;
  let cleanFiles = 0;

  for (const relPath of files) {
    const absPath = path.join(coreRoot, relPath);
    if (!fs.existsSync(absPath)) continue;

    const source = fs.readFileSync(absPath, "utf8");
    const violations = findRuntimeViolations(source);

    if (violations.length === 0) {
      cleanFiles += 1;
      continue;
    }

    let unwrapCount = 0;
    let expectCount = 0;
    for (const violation of violations) {
      if (/\bunwrap\s*\(/.test(violation.snippet)) unwrapCount += 1;
      if (/\bexpect\s*\(/.test(violation.snippet)) expectCount += 1;
    }

    perFile[relPath] = {
      violations: violations.length,
      unwrap: unwrapCount,
      expect: expectCount,
    };

    totalUnwrap += unwrapCount;
    totalExpect += expectCount;
    totalViolations += violations.length;
  }

  return {
    generatedAt: new Date().toISOString(),
    summary: {
      filesScanned: files.length,
      filesWithViolations: Object.keys(perFile).length,
      cleanFiles,
      totalViolations,
      totalUnwrap,
      totalExpect,
    },
    perFile,
  };
}

function renderTable(report) {
  const lines = [];
  lines.push("=== rust panic-trap report ===");
  lines.push(`generatedAt: ${report.generatedAt}`);
  lines.push("");
  lines.push(
    `files scanned: ${report.summary.filesScanned}  ` +
      `clean: ${report.summary.cleanFiles}  ` +
      `with violations: ${report.summary.filesWithViolations}`,
  );
  lines.push(
    `total violations: ${report.summary.totalViolations}  ` +
      `unwrap: ${report.summary.totalUnwrap}  ` +
      `expect: ${report.summary.totalExpect}`,
  );
  lines.push("");

  const sorted = Object.entries(report.perFile).sort(
    (a, b) => b[1].violations - a[1].violations,
  );
  lines.push("top files by violation count:");
  if (sorted.length === 0) {
    lines.push("  (none)");
    return lines.join("\n");
  }

  for (const [file, data] of sorted.slice(0, 30)) {
    lines.push(
      `  ${String(data.violations).padStart(4)}  ` +
        `(unwrap=${data.unwrap} expect=${data.expect})  ${file}`,
    );
  }

  if (sorted.length > 30) {
    lines.push(`  ... and ${sorted.length - 30} more files`);
  }

  return lines.join("\n");
}

function compareAgainstZero(report) {
  const violations = [];
  for (const [file, data] of Object.entries(report.perFile)) {
    violations.push(
      `${file}: ${data.violations} violations (unwrap=${data.unwrap} expect=${data.expect})`,
    );
  }
  return violations;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const coreRoot = process.cwd();
  const files = discoverProductionRustFiles(coreRoot);
  const report = scanAllFiles(coreRoot, files);

  if (args.format === "json") {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  } else {
    process.stdout.write(`${renderTable(report)}\n`);
  }

  if (!args.enforce) {
    return;
  }

  const enforceViolations = compareAgainstZero(report);
  if (enforceViolations.length > 0) {
    process.stderr.write(
      `\nrust panic-trap enforcement FAILED (${enforceViolations.length} violations):\n`,
    );
    for (const violation of enforceViolations) {
      process.stderr.write(`  - ${violation}\n`);
    }
    process.stderr.write(
      "\nDo not use unwrap() or expect() in production Rust code.\n" +
        "Use proper error propagation with `?` and Result types.\n" +
        "If truly unavoidable, add an inline `// EXCEPTION: panic-trap — <rationale>` comment.\n",
    );
    process.exitCode = 1;
    return;
  }

  process.stderr.write(
    `Rust panic-trap enforcement passed (${files.length} files).\n`,
  );
}

main();
