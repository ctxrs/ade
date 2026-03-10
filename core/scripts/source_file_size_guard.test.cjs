const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  GUIDANCE_MESSAGE,
  HARD_MAX_LINES,
  SOFT_MAX_LINES,
  collectSourceFileStats,
  countLines,
  evaluateSourceFileSizes,
  isTrackedSourceFile,
  run,
} = require("./source_file_size_guard.cjs");

const writeFile = (rootDir, relativePath, lineCount) => {
  const absolutePath = path.join(rootDir, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  const lines = Array.from({ length: lineCount }, (_, index) => `line ${index + 1}`);
  fs.writeFileSync(absolutePath, `${lines.join("\n")}\n`, "utf8");
};

const writeExceptions = (rootDir, entries) => {
  const exceptionPath = path.join(rootDir, "exceptions.json");
  fs.writeFileSync(exceptionPath, `${JSON.stringify(entries, null, 2)}\n`, "utf8");
  return exceptionPath;
};

test("isTrackedSourceFile includes src production files and excludes tests", () => {
  assert.equal(isTrackedSourceFile("apps/web/src/pages/SessionPage.view.tsx"), true);
  assert.equal(isTrackedSourceFile("crates/ctx-http/src/api/mod.rs"), true);
  assert.equal(isTrackedSourceFile("apps/web/src/pages/SessionPage.view.test.tsx"), false);
  assert.equal(isTrackedSourceFile("crates/ctx-http/src/provider_accounts/tests.rs"), false);
  assert.equal(isTrackedSourceFile("apps/web/scripts/replay-loadtest.mjs"), false);
  assert.equal(isTrackedSourceFile("external-harnesses/codex/codex-rs/core/src/codex.rs"), false);
});

test("evaluateSourceFileSizes reports missing exceptions and oversized exceptions", () => {
  const files = [
    { path: "apps/web/src/Small.ts", lineCount: SOFT_MAX_LINES + 10 },
    { path: "apps/web/src/TooBig.ts", lineCount: HARD_MAX_LINES + 1 },
    { path: "crates/ctx-http/src/Huge.rs", lineCount: HARD_MAX_LINES + 50 },
  ];
  const exceptions = new Map([
    [
      "crates/ctx-http/src/Huge.rs",
      { path: "crates/ctx-http/src/Huge.rs", maxLines: HARD_MAX_LINES + 20, reason: "temporary" },
    ],
  ]);

  const result = evaluateSourceFileSizes({ files, exceptions });

  assert.equal(result.warnings.length, 2);
  assert.deepEqual(
    result.violations.map((entry) => ({ path: entry.path, type: entry.type })),
    [
      { path: "apps/web/src/TooBig.ts", type: "missing_exception" },
      { path: "crates/ctx-http/src/Huge.rs", type: "exception_exceeded" },
    ],
  );
});

test("countLines matches newline-terminated and unterminated files", () => {
  assert.equal(countLines("a\nb\n"), 2);
  assert.equal(countLines("a\nb"), 2);
  assert.equal(countLines(""), 0);
});

test("run emits the architectural guidance and exits nonzero when enforcement fails", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "source-file-size-"));
  writeFile(rootDir, "apps/web/src/pages/HugePage.tsx", HARD_MAX_LINES + 5);
  const exceptionsFilePath = writeExceptions(rootDir, []);
  let output = "";
  const stream = {
    write(chunk) {
      output += String(chunk);
    },
  };

  const exitCode = run({
    rootDir,
    exceptionsFilePath,
    enforce: true,
    stdout: stream,
    stderr: stream,
  });

  assert.equal(exitCode, 1);
  assert.match(output, /HugePage\.tsx is \d+ lines/);
  assert.match(output, new RegExp(GUIDANCE_MESSAGE.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")));
});

test("run accepts files that are large but explicitly capped by exception", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "source-file-size-ok-"));
  writeFile(rootDir, "apps/web/src/pages/AllowedPage.tsx", HARD_MAX_LINES + 5);
  const exceptionsFilePath = writeExceptions(rootDir, [
    {
      path: "apps/web/src/pages/AllowedPage.tsx",
      maxLines: HARD_MAX_LINES + 5,
      reason: "existing large file",
    },
  ]);
  const files = collectSourceFileStats(rootDir);
  assert.equal(files.length, 1);
  const exitCode = run({
    rootDir,
    exceptionsFilePath,
    enforce: true,
    stdout: { write() {} },
    stderr: { write() {} },
  });
  assert.equal(exitCode, 0);
});
