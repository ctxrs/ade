const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  GUIDANCE_MESSAGE,
  HARD_MAX_LINES,
  SOFT_MAX_LINES,
  classifySourceFile,
  collectSourceFileStats,
  countLines,
  evaluateSourceFileSizes,
  isTrackedSourceFile,
  isTestOrAutomationFile,
  run,
} = require("./source_file_size_guard.cjs");

const writeFile = (rootDir, relativePath, lineCount) => {
  const absolutePath = path.join(rootDir, relativePath);
  fs.mkdirSync(path.dirname(absolutePath), { recursive: true });
  const lines = Array.from({ length: lineCount }, (_, index) => `line ${index + 1}`);
  fs.writeFileSync(absolutePath, `${lines.join("\n")}\n`, "utf8");
};

test("source file classification distinguishes production from test and ignored paths", () => {
  assert.equal(classifySourceFile("core/apps/web/src/pages/SessionPage.view.tsx"), "production");
  assert.equal(classifySourceFile("core/crates/ctx-http/src/api/mod.rs"), "production");
  assert.equal(classifySourceFile("site/src/main.js"), "production");
  assert.equal(classifySourceFile("core/apps/web/src/pages/SessionPage.view.test.tsx"), "test_or_automation");
  assert.equal(classifySourceFile("core/crates/ctx-http/src/provider_accounts/tests.rs"), "test_or_automation");
  assert.equal(classifySourceFile("core/apps/web/e2e/workbench-index.spec.ts"), "test_or_automation");
  assert.equal(classifySourceFile("core/scripts/bundled_dependency_updates.cjs"), "test_or_automation");
  assert.equal(classifySourceFile("core/crates/ctx-http/src/bin/ctx-http-lsp-test-server.rs"), "production");
  assert.equal(classifySourceFile("external-harnesses/codex/codex-rs/core/src/codex.rs"), "ignore");
});

test("isTrackedSourceFile includes production files and excludes tests and external harnesses", () => {
  assert.equal(isTrackedSourceFile("core/apps/web/src/pages/SessionPage.view.tsx"), true);
  assert.equal(isTrackedSourceFile("core/crates/ctx-http/src/api/mod.rs"), true);
  assert.equal(isTrackedSourceFile("site/src/main.js"), true);
  assert.equal(isTrackedSourceFile("core/apps/web/src/pages/SessionPage.view.test.tsx"), false);
  assert.equal(isTrackedSourceFile("core/crates/ctx-http/src/provider_accounts/tests.rs"), false);
  assert.equal(isTrackedSourceFile("core/apps/web/scripts/replay-loadtest.mjs"), false);
  assert.equal(isTrackedSourceFile("external-harnesses/codex/codex-rs/core/src/codex.rs"), false);
  assert.equal(isTestOrAutomationFile("core/apps/web/e2e/workbench-index.spec.ts"), true);
  assert.equal(isTestOrAutomationFile("core/apps/web/src/pages/SessionPage.view.tsx"), false);
});

test("evaluateSourceFileSizes reports soft-limit warnings and hard-limit violations", () => {
  const files = [
    { path: "apps/web/src/Small.ts", lineCount: SOFT_MAX_LINES + 10 },
    { path: "apps/web/src/TooBig.ts", lineCount: HARD_MAX_LINES + 1 },
    { path: "crates/ctx-http/src/Huge.rs", lineCount: HARD_MAX_LINES + 50 },
  ];

  const result = evaluateSourceFileSizes({ files });

  assert.equal(result.warnings.length, 3);
  assert.deepEqual(result.violations.map((entry) => entry.path), [
    "apps/web/src/TooBig.ts",
    "crates/ctx-http/src/Huge.rs",
  ]);
});

test("countLines matches newline-terminated and unterminated files", () => {
  assert.equal(countLines("a\nb\n"), 2);
  assert.equal(countLines("a\nb"), 2);
  assert.equal(countLines(""), 0);
});

test("run emits the architectural guidance and exits nonzero when enforcement fails", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "source-file-size-"));
  writeFile(rootDir, "core/apps/web/src/pages/HugePage.tsx", HARD_MAX_LINES + 5);
  let output = "";
  const stream = {
    write(chunk) {
      output += String(chunk);
    },
  };

  const exitCode = run({
    rootDir,
    enforce: true,
    stdout: stream,
    stderr: stream,
  });

  assert.equal(exitCode, 1);
  assert.match(output, /HugePage\.tsx is \d+ lines/);
  assert.match(output, new RegExp(GUIDANCE_MESSAGE.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")));
});

test("run ignores oversized test and automation files", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "source-file-size-ignore-"));
  writeFile(rootDir, "core/apps/web/src/pages/AllowedPage.test.tsx", HARD_MAX_LINES + 50);
  writeFile(rootDir, "core/apps/web/e2e/workbench-index.spec.ts", HARD_MAX_LINES + 50);
  writeFile(rootDir, "core/scripts/replay-loadtest.mjs", HARD_MAX_LINES + 50);
  const files = collectSourceFileStats(rootDir);
  assert.equal(files.length, 0);
  const exitCode = run({
    rootDir,
    enforce: true,
    stdout: { write() {} },
    stderr: { write() {} },
  });
  assert.equal(exitCode, 0);
});

test("collectSourceFileStats keeps site sources and ignores external harnesses", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "source-file-size-collect-"));
  writeFile(rootDir, "core/apps/web/src/pages/WorkbenchPage.shell.tsx", 10);
  writeFile(rootDir, "site/src/main.js", 11);
  writeFile(rootDir, "core/apps/web/e2e/workbench-index.spec.ts", 12);
  writeFile(rootDir, "external-harnesses/codex/codex-rs/core/src/codex.rs", 13);

  const files = collectSourceFileStats(rootDir);
  assert.deepEqual(
    files.map((entry) => entry.path),
    ["site/src/main.js", "core/apps/web/src/pages/WorkbenchPage.shell.tsx"],
  );
});
