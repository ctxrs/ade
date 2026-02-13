#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const SCOPE_FILES = [
  "crates/ctx-mcp/src/main/02_main.rs",
  "crates/ctx-http/src/terminals.rs",
  "crates/ctx-http/src/perf_telemetry.rs",
  "crates/ctx-http/src/execution_setup.rs",
  "crates/ctx-worker-shim/src/main/02_terminal.rs",
  "crates/ctx-docs-mirror/src/crawler.rs",
  "crates/ctx-mcp/src/main/01_prelude.rs",
  "crates/ctx-http/src/fault_injection.rs",
  "crates/ctx-store/src/fault_injection.rs",
  "crates/ctx-worker-gateway/src/drivers/gcp.rs",
  "apps/desktop/src-tauri/src/main.rs",
  "apps/desktop/src-tauri/src/desktop_connection.rs",
  "apps/desktop/src-tauri/src/desktop_daemon.rs",
  "apps/desktop/src-tauri/src/desktop_deeplink.rs",
  "apps/desktop/src-tauri/src/desktop_editor.rs",
  "apps/desktop/src-tauri/src/desktop_ssh.rs",
  "apps/desktop/src-tauri/src/desktop_storage.rs",
  "apps/desktop/src-tauri/src/desktop_windows.rs",
  "crates/ctx-docs-mirror/src/plan.rs",
  "crates/ctx-docs-mirror/src/sitemap.rs",
  "crates/ctx-http/src/llm.rs",
  "crates/ctx-tunnel-relay/src/main.rs",
  "crates/ctx-store/src/store/messages.rs",
  "apps/tauri-mobile/src-tauri/src/lib.rs",
  "crates/ctx-http/src/api/tasks.rs",
  "crates/ctx-http/src/attachments.rs",
  "crates/ctx-http/src/provider_restart.rs",
  "crates/ctx-http/src/provider_usage.rs",
  "crates/ctx-store/src/active_snapshot_observer.rs",
  "crates/ctx-worker-gateway/src/drivers/azure.rs",
];

const PANIC_CALL_PATTERN = /\b(?:unwrap|expect)\s*\(/;
const CFG_TEST_PATTERN = /^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/;
const ATTRIBUTE_PATTERN = /^\s*#\s*\[/;

function braceDelta(line) {
  let delta = 0;
  for (const ch of line) {
    if (ch === "{") {
      delta += 1;
    } else if (ch === "}") {
      delta -= 1;
    }
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
      if (skipDepth <= 0) {
        skipDepth = 0;
      }
      continue;
    }

    if (pendingCfgTestItem) {
      if (trimmed === "") {
        continue;
      }
      if (ATTRIBUTE_PATTERN.test(trimmed) && !trimmed.includes("{")) {
        continue;
      }
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
      violations.push({
        line: index + 1,
        snippet: trimmed,
      });
    }
  }

  return violations;
}

const root = process.cwd();
const failures = [];

for (const relPath of SCOPE_FILES) {
  const absPath = path.join(root, relPath);
  if (!fs.existsSync(absPath)) {
    failures.push({
      file: relPath,
      line: 0,
      snippet: "missing file in panic-trap scope",
    });
    continue;
  }
  const source = fs.readFileSync(absPath, "utf8");
  const violations = findRuntimeViolations(source);
  for (const violation of violations) {
    failures.push({
      file: relPath,
      line: violation.line,
      snippet: violation.snippet,
    });
  }
}

if (failures.length > 0) {
  console.error("Rust runtime panic trap check failed:");
  for (const failure of failures) {
    if (failure.line > 0) {
      console.error(`  - ${failure.file}:${failure.line} ${failure.snippet}`);
    } else {
      console.error(`  - ${failure.file}: ${failure.snippet}`);
    }
  }
  process.exit(1);
}

console.log(`Rust runtime panic trap check passed (${SCOPE_FILES.length} files).`);
