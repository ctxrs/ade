import fs from "node:fs";
import path from "node:path";

const TS_FILE_RE = /\.(ts|tsx)$/;
const TEST_FILE_RE = /\.(test|spec)\.(ts|tsx)$/;
const ANY_TOKEN_RE = /\bany\b/g;
const ANY_LINE_RE = /\bany\b/;
const AS_ANY_RE = /\bas any\b/g;
const TYPE_ANY_RE = /:\s*any\b/g;
const CATCH_ANY_RE = /catch\s*\(\s*\w+\s*:\s*any\s*\)/g;
const RECORD_ANY_RE = /Record\s*<\s*string\s*,\s*any\s*>/g;
const PROMISE_ANY_RE = /Promise\s*<\s*any\s*>/g;
const ARRAY_ANY_RE = /\bany\s*\[\s*\]/g;

function parseArgs(argv) {
  const out = {
    format: "table",
    enforce: false,
    writeBaseline: false,
    baseline: null,
    enforceMode: "zero",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--format") {
      out.format = String(argv[i + 1] || "table");
      i += 1;
      continue;
    }
    if (arg === "--enforce") {
      out.enforce = true;
      continue;
    }
    if (arg === "--write-baseline") {
      out.writeBaseline = true;
      continue;
    }
    if (arg === "--baseline") {
      out.baseline = String(argv[i + 1] || "");
      i += 1;
      continue;
    }
    if (arg === "--enforce-mode") {
      out.enforceMode = String(argv[i + 1] || "zero");
      i += 1;
      continue;
    }
  }
  if (out.format !== "table" && out.format !== "json") {
    throw new Error(`Unsupported --format value: ${out.format}`);
  }
  if (out.enforceMode !== "zero" && out.enforceMode !== "baseline") {
    throw new Error(`Unsupported --enforce-mode value: ${out.enforceMode}`);
  }
  return out;
}

function findRepoRoot(startDir) {
  let current = path.resolve(startDir);
  while (true) {
    const ctxDir = path.join(current, ".ctx");
    const corePackageJson = path.join(current, "core", "package.json");
    const moduleBazel = path.join(current, "MODULE.bazel");
    if (
      (fs.existsSync(ctxDir) && fs.statSync(ctxDir).isDirectory()) ||
      (fs.existsSync(corePackageJson) && fs.statSync(corePackageJson).isFile() && fs.existsSync(moduleBazel))
    ) {
      return current;
    }
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
  throw new Error(`Could not locate repo root from ${startDir}`);
}

function listTsFiles(dir) {
  const out = [];
  if (!fs.existsSync(dir)) return out;
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...listTsFiles(full));
      continue;
    }
    if (entry.isFile() && TS_FILE_RE.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

function countMatches(input, re) {
  re.lastIndex = 0;
  let count = 0;
  while (re.exec(input)) count += 1;
  return count;
}

function scanFiles(baseDir, files, { productionOnly = false } = {}) {
  const data = {
    fileCount: 0,
    anyTokenMatches: 0,
    linesContainingAny: 0,
    explicitPatternMatches: 0,
    asAny: 0,
    typeAny: 0,
    catchAny: 0,
    recordStringAny: 0,
    promiseAny: 0,
    arrayAny: 0,
    topFilesByAnyLines: [],
  };
  const top = [];
  for (const file of files) {
    const rel = path.relative(baseDir, file);
    if (productionOnly && TEST_FILE_RE.test(rel)) continue;
    const content = fs.readFileSync(file, "utf8");
    const anyTokenMatches = countMatches(content, ANY_TOKEN_RE);
    const lines = content.split(/\r?\n/);
    const linesContainingAny = lines.reduce((sum, line) => sum + (ANY_LINE_RE.test(line) ? 1 : 0), 0);
    const asAny = countMatches(content, AS_ANY_RE);
    const typeAny = countMatches(content, TYPE_ANY_RE);
    const catchAny = countMatches(content, CATCH_ANY_RE);
    const recordStringAny = countMatches(content, RECORD_ANY_RE);
    const promiseAny = countMatches(content, PROMISE_ANY_RE);
    const arrayAny = countMatches(content, ARRAY_ANY_RE);
    const explicitPatternMatches = asAny + typeAny + recordStringAny + promiseAny + arrayAny;

    data.fileCount += 1;
    data.anyTokenMatches += anyTokenMatches;
    data.linesContainingAny += linesContainingAny;
    data.explicitPatternMatches += explicitPatternMatches;
    data.asAny += asAny;
    data.typeAny += typeAny;
    data.catchAny += catchAny;
    data.recordStringAny += recordStringAny;
    data.promiseAny += promiseAny;
    data.arrayAny += arrayAny;

    if (linesContainingAny > 0) {
      top.push({
        file: rel,
        anyLines: linesContainingAny,
      });
    }
  }
  data.topFilesByAnyLines = top.sort((a, b) => b.anyLines - a.anyLines).slice(0, 30);
  return data;
}

function renderTable(report) {
  const lines = [];
  lines.push("=== explicit-any report ===");
  lines.push(`generatedAt: ${report.generatedAt}`);
  lines.push("");
  lines.push("scope                               anyTokens  anyLines  explicitPatterns  asAny  typeAny  catchAny");
  lines.push(
    `src/all                             ${String(report.src.all.anyTokenMatches).padStart(8)}  ${String(report.src.all.linesContainingAny).padStart(8)}  ${String(report.src.all.explicitPatternMatches).padStart(16)}  ${String(report.src.all.asAny).padStart(5)}  ${String(report.src.all.typeAny).padStart(7)}  ${String(report.src.all.catchAny).padStart(8)}`,
  );
  lines.push(
    `src/production                      ${String(report.src.production.anyTokenMatches).padStart(8)}  ${String(report.src.production.linesContainingAny).padStart(8)}  ${String(report.src.production.explicitPatternMatches).padStart(16)}  ${String(report.src.production.asAny).padStart(5)}  ${String(report.src.production.typeAny).padStart(7)}  ${String(report.src.production.catchAny).padStart(8)}`,
  );
  lines.push(
    `src/tests                           ${String(report.src.tests.anyTokenMatches).padStart(8)}  ${String(report.src.tests.linesContainingAny).padStart(8)}  ${String(report.src.tests.explicitPatternMatches).padStart(16)}  ${String(report.src.tests.asAny).padStart(5)}  ${String(report.src.tests.typeAny).padStart(7)}  ${String(report.src.tests.catchAny).padStart(8)}`,
  );
  lines.push(
    `e2e/all                             ${String(report.e2e.anyTokenMatches).padStart(8)}  ${String(report.e2e.linesContainingAny).padStart(8)}  ${String(report.e2e.explicitPatternMatches).padStart(16)}  ${String(report.e2e.asAny).padStart(5)}  ${String(report.e2e.typeAny).padStart(7)}  ${String(report.e2e.catchAny).padStart(8)}`,
  );
  lines.push("");
  lines.push("top src/production files by any-lines:");
  for (const row of report.src.production.topFilesByAnyLines.slice(0, 15)) {
    lines.push(`- ${String(row.anyLines).padStart(3)}  ${row.file}`);
  }
  return lines.join("\n");
}

function buildReport(webRoot) {
  const srcDir = path.join(webRoot, "src");
  const e2eDir = path.join(webRoot, "e2e");
  const srcFiles = listTsFiles(srcDir);
  const e2eFiles = listTsFiles(e2eDir);

  const srcAll = scanFiles(webRoot, srcFiles, { productionOnly: false });
  const srcProduction = scanFiles(webRoot, srcFiles, { productionOnly: true });
  const srcTests = {
    fileCount: srcAll.fileCount - srcProduction.fileCount,
    anyTokenMatches: srcAll.anyTokenMatches - srcProduction.anyTokenMatches,
    linesContainingAny: srcAll.linesContainingAny - srcProduction.linesContainingAny,
    explicitPatternMatches: srcAll.explicitPatternMatches - srcProduction.explicitPatternMatches,
    asAny: srcAll.asAny - srcProduction.asAny,
    typeAny: srcAll.typeAny - srcProduction.typeAny,
    catchAny: srcAll.catchAny - srcProduction.catchAny,
    recordStringAny: srcAll.recordStringAny - srcProduction.recordStringAny,
    promiseAny: srcAll.promiseAny - srcProduction.promiseAny,
    arrayAny: srcAll.arrayAny - srcProduction.arrayAny,
    topFilesByAnyLines: [],
  };
  const e2eAll = scanFiles(webRoot, e2eFiles, { productionOnly: false });

  return {
    generatedAt: new Date().toISOString(),
    src: {
      all: srcAll,
      production: srcProduction,
      tests: srcTests,
    },
    e2e: e2eAll,
  };
}

function compareAgainstBaseline(report, baseline) {
  const violations = [];
  const checks = [
    ["src.production.anyTokenMatches", report.src.production.anyTokenMatches, baseline.src?.production?.anyTokenMatches],
    [
      "src.production.explicitPatternMatches",
      report.src.production.explicitPatternMatches,
      baseline.src?.production?.explicitPatternMatches,
    ],
    ["src.production.asAny", report.src.production.asAny, baseline.src?.production?.asAny],
    ["src.production.typeAny", report.src.production.typeAny, baseline.src?.production?.typeAny],
    ["src.production.catchAny", report.src.production.catchAny, baseline.src?.production?.catchAny],
  ];
  for (const [name, current, limit] of checks) {
    if (typeof limit !== "number") {
      violations.push(`${name}: baseline missing numeric value`);
      continue;
    }
    if (current > limit) {
      violations.push(`${name}: current=${current} exceeds baseline=${limit}`);
    }
  }
  return violations;
}

function compareAgainstZero(report) {
  const violations = [];
  const checks = [
    ["src.production.explicitPatternMatches", report.src.production.explicitPatternMatches],
    ["src.production.asAny", report.src.production.asAny],
    ["src.production.typeAny", report.src.production.typeAny],
    ["src.production.catchAny", report.src.production.catchAny],
    ["src.production.recordStringAny", report.src.production.recordStringAny],
    ["src.production.promiseAny", report.src.production.promiseAny],
    ["src.production.arrayAny", report.src.production.arrayAny],
  ];
  for (const [name, current] of checks) {
    if (current !== 0) {
      violations.push(`${name}: current=${current} expected=0`);
    }
  }
  return violations;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const webRoot = process.cwd();
  const repoRoot = findRepoRoot(webRoot);
  const baselinePath =
    args.baseline == null ? null : path.resolve(webRoot, args.baseline);
  const report = buildReport(webRoot);

  if (args.writeBaseline && baselinePath == null) {
    throw new Error("--write-baseline requires --baseline <path>");
  }

  if (args.writeBaseline) {
    fs.mkdirSync(path.dirname(baselinePath), { recursive: true });
    fs.writeFileSync(baselinePath, `${JSON.stringify(report, null, 2)}\n`);
  }

  if (args.format === "json") {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
  } else {
    process.stdout.write(`${renderTable(report)}\n`);
  }

  if (args.enforce) {
    let violations = [];
    if (args.enforceMode === "baseline") {
      if (baselinePath == null) {
        throw new Error("baseline enforcement requires --baseline <path>");
      }
      if (!fs.existsSync(baselinePath)) {
        throw new Error(`Baseline file not found: ${baselinePath}`);
      }
      const baseline = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
      violations = compareAgainstBaseline(report, baseline);
    } else {
      violations = compareAgainstZero(report);
    }
    if (violations.length > 0) {
      process.stderr.write(`\nexplicit-any enforcement failed (${violations.length}):\n`);
      for (const v of violations) process.stderr.write(`- ${v}\n`);
      process.exitCode = 1;
    }
  }
}

main();
