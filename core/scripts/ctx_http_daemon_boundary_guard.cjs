#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const ctxHttpSrcRoot = path.join(coreRoot, "crates", "ctx-http", "src");
const ctxHttpTestsRoot = path.join(coreRoot, "crates", "ctx-http", "tests");
const ctxHttpTestSupportSrcRoot = path.join(coreRoot, "crates", "ctx-http-test-support", "src");
const apiRoot = path.join(coreRoot, "crates", "ctx-http", "src", "api");
const legacyHttpDaemonRoot = path.join(coreRoot, "crates", "ctx-http", "src", "daemon");
const legacyHttpDaemonRootPath = path.join(coreRoot, "crates", "ctx-http", "src", "daemon.rs");
const daemonRoot = path.join(coreRoot, "crates", "ctx-daemon", "src", "daemon");
const daemonRootPath = path.join(coreRoot, "crates", "ctx-daemon", "src", "daemon.rs");
const daemonHandlePath = path.join(coreRoot, "crates", "ctx-daemon", "src", "daemon", "handle.rs");
const rawStoreBlindApiRoots = [
  "core/crates/ctx-http/src/api/sessions/",
  "core/crates/ctx-http/src/api/tasks/",
  "core/crates/ctx-http/src/api/workspaces/",
];
const migratedRawDaemonTestRoots = [
  "core/crates/ctx-http/src/api/settings.rs",
  "core/crates/ctx-http/src/api/sessions/tests.rs",
  "core/crates/ctx-http/src/api/sessions/tests/",
  "core/crates/ctx-http/src/api/providers/login/codex/tests.rs",
  "core/crates/ctx-http/src/api/providers/tests/install_statuses.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/auth_change/",
  "core/crates/ctx-http/src/api/providers/tests/restarts/harness_source.rs",
  "core/crates/ctx-http/src/api/workspaces/tests.rs",
  "core/crates/ctx-http/src/api/tasks.rs",
  "core/crates/ctx-http/src/api/tasks/cleanup_lifecycle_tests.rs",
  "core/crates/ctx-http/src/api/tasks/lifecycle_tests/",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests.rs",
  "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/",
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/",
  "core/crates/ctx-http/src/lib_tests/cors.rs",
  "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
  "core/crates/ctx-http/src/lib_tests/daemon_smoke/",
  "core/crates/ctx-http/src/lib_tests/execution_launch/",
  "core/crates/ctx-http/src/lib_tests/health_diagnostics/",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries/",
  "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/",
  "core/crates/ctx-http/src/lib_tests/org_policy_routes.rs",
  "core/crates/ctx-http/src/lib_tests/provider_routes/",
  "core/crates/ctx-http/src/lib_tests/run_archive_routes.rs",
  "core/crates/ctx-http/src/lib_tests/session_artifacts.rs",
  "core/crates/ctx-http/src/lib_tests/session_artifacts/",
  "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http.rs",
  "core/crates/ctx-http/src/lib_tests/session_head_ctx_ui_sized_http/",
  "core/crates/ctx-http/src/lib_tests/session_head_large_http.rs",
  "core/crates/ctx-http/src/lib_tests/telemetry_export_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/update_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/web_session_routes/fixtures.rs",
  "core/crates/ctx-http/src/lib_tests/workspace_active_routes.rs",
  "core/crates/ctx-http/tests/acp_target_scoped_status.rs",
  "core/crates/ctx-http/tests/assistant_chunk_stream_only.rs",
  "core/crates/ctx-http/tests/assistant_message_persistence_faults.rs",
  "core/crates/ctx-http/tests/codex_host_import_api.rs",
  "core/crates/ctx-http/tests/codex_login_callback_api.rs",
  "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
  "core/crates/ctx-http/tests/fault_matrix.rs",
  "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
  "core/crates/ctx-http/tests/global_id_routing_http.rs",
  "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
  "core/crates/ctx-http/tests/install_start_contract.rs",
  "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
  "core/crates/ctx-http/tests/message_idempotency.rs",
  "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
  "core/crates/ctx-http/tests/provider_current_ctx_version_regressions.rs",
  "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
  "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
  "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
  "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
  "core/crates/ctx-http/tests/repo_validate_destination.rs",
  "core/crates/ctx-http/tests/session_model_api.rs",
  "core/crates/ctx-http/tests/subagent_mcp_http.rs",
  "core/crates/ctx-http/tests/system_prompt_append_http.rs",
  "core/crates/ctx-http/tests/task_default_session_http.rs",
  "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
  "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
  "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
  "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
  "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
  "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
  "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
  "core/crates/ctx-http/tests/worktree_archive_http.rs",
];

const API_RAW_DAEMON_PATTERNS = [
  {
    name: "raw DaemonState type",
    regex: /\bDaemonState\b/,
  },
  {
    name: "raw daemon state extractor",
    regex: /State\s*<\s*Arc\s*<\s*DaemonState\s*>\s*>/,
  },
  {
    name: "raw daemon state arc",
    regex: /Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "broad daemon handle extractor",
    regex: /State\s*<\s*DaemonHandle\s*>/,
  },
  {
    name: "daemon handle escalation call",
    regex: /\.daemon_handle\s*\(/,
  },
  {
    name: "global store accessor",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "daemon store accessor",
    regex: /\.(?:(?:route_)?(?:load_)?(?:store_for_(?:workspace|worktree|task|session)|existing_(?:workspace|session)_store(?:_allow_archived|_for_write)?)|(?:route_|load_)(?:workspace|worktree|task|session|existing_workspace|existing_session(?:_allow_archived|_for_write)?)_store|route_(?:existing_)?(?:workspace|session)_store(?:_allow_archived|_for_write)?)\s*\(/,
  },
  {
    name: "broad daemon handle field",
    regex: /^\s*\w+\s*:\s*DaemonHandle\b/,
  },
];

const API_DOMAIN_RAW_STORE_PATTERNS = [
  {
    name: "raw ctx_store Store in daemon-blind API family",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
  },
];

const HANDLE_BACKDOOR_PATTERNS = [
  {
    name: "raw daemon FromRef backdoor",
    regex: /FromRef\s*<\s*DaemonHandle\s*>\s*for\s*Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "raw daemon state accessor",
    regex: /\bfn\s+state\s*\(\s*&self\s*\)\s*->\s*&\s*Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "daemon handle escalation accessor",
    regex: /\bfn\s+daemon_handle\s*\(\s*&self\s*\)\s*->\s*DaemonHandle/,
  },
  {
    name: "secure proxy full-router backdoor",
    regex: /router\s*\(\s*handle\.clone\s*\(\s*\)\s*\)|Arc\s*<\s*axum::Router\s*>/,
  },
];

const DAEMON_EXTRACTION_BLOCKER_PATTERNS = [
  {
    name: "daemon depends on ctx-http",
    regex: /\bctx_http::/,
  },
  {
    name: "daemon imports API module",
    regex: /\bcrate::api\b|\bapi::router\b/,
    contentRegex: /\buse\s+crate::\s*\{[^;]*\bapi\b[^;]*\}\s*;/gm,
  },
  {
    name: "daemon depends on Axum",
    regex: /\buse\s+axum\b|\baxum::/,
  },
  {
    name: "daemon owns Axum extractor glue",
    regex: /\bFromRef\s*</,
  },
];

const TEST_RAW_DAEMON_BUCKET_PATTERNS = [
  {
    name: "raw daemon runtime bucket field access",
    regex: /\b(?:state|app_state|daemon_state)\s*\.\s*(?:core|sessions|workspaces|providers|telemetry|transport|execution)\s*\./,
  },
];

const MIGRATED_TEST_RAW_DAEMON_PATTERNS = [
  {
    name: "raw daemon state constructor in migrated test surface",
    regex: /\bDaemonState::new(?:_with_(?:public_base_url|runtime_flags))?\s*\(/,
  },
  {
    name: "raw daemon state arc in migrated test surface",
    regex: /Arc\s*<\s*DaemonState\s*>/,
  },
  {
    name: "raw daemon router wiring in migrated test surface",
    regex: /\bapi::router\s*\(\s*state(?:\.clone\s*\(\s*\))?\s*\)/,
  },
  {
    name: "raw common daemon state helper in migrated test surface",
    regex: /(?:\bcommon::|(?<![\w:.])\b)build_state\s*\(/,
  },
  {
    name: "raw common router helper in migrated test surface",
    regex: /(?:\bcommon::|(?<![\w:.])\b)router\s*\(/,
  },
  {
    name: "raw provider-session token helper in migrated test surface",
    regex: /(?:\bctx_daemon::daemon::|(?<!\.)\b)(?:issue_provider_session_mcp_token(?:_with_capabilities)?|revoke_provider_session_mcp_token)\s*\(/,
  },
];

function isRustFile(filePath) {
  return filePath.endsWith(".rs");
}

function isTestRustPath(filePath) {
  const normalized = filePath.split(path.sep).join("/");
  const base = path.basename(filePath);
  return normalized.includes("/tests/")
    || normalized.includes("/lib_tests/")
    || normalized.includes("/test_support/")
    || normalized.includes("/lifecycle_tests/")
    || normalized.includes("/storage_admission_http_tests/")
    || normalized.includes("/cleanup_lifecycle_tests")
    || base === "tests.rs"
    || base.endsWith("_tests.rs");
}

function testSurfaceRustFiles() {
  const files = [];
  if (fs.existsSync(ctxHttpSrcRoot)) {
    files.push(
      ...listRustFiles(ctxHttpSrcRoot).filter((filePath) => {
        const relativePath = repoRelative(filePath);
        return isTestRustPath(filePath) || migratedTestPatternsForPath(relativePath).length > 0;
      }),
    );
  }
  for (const root of [ctxHttpTestsRoot, ctxHttpTestSupportSrcRoot]) {
    if (fs.existsSync(root)) {
      files.push(...listRustFiles(root));
    }
  }
  return files;
}

function listRustFiles(root) {
  const out = [];
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const fullPath = path.join(root, entry.name);
    if (entry.isDirectory()) {
      out.push(...listRustFiles(fullPath));
    } else if (entry.isFile() && isRustFile(fullPath)) {
      out.push(fullPath);
    }
  }
  return out;
}

function stripCfgTestItems(contents) {
  const lines = contents.split(/\r?\n/);
  const kept = [];
  let skipCfgItem = false;
  let braceDepth = 0;
  let sawCfgItemBody = false;

  for (const line of lines) {
    if (!skipCfgItem && /^\s*#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]/.test(line)) {
      skipCfgItem = true;
      braceDepth = 0;
      sawCfgItemBody = false;
      continue;
    }

    if (skipCfgItem) {
      for (const char of line) {
        if (char === "{") {
          braceDepth += 1;
          sawCfgItemBody = true;
        }
        if (char === "}") braceDepth -= 1;
      }
      if (!sawCfgItemBody && line.trim().endsWith(";")) {
        skipCfgItem = false;
      } else if (sawCfgItemBody && braceDepth <= 0) {
        skipCfgItem = false;
      }
      continue;
    }

    kept.push(line);
  }

  return kept.join("\n");
}

function scanText({ filePath, contents, patterns }) {
  const violations = [];
  const lines = contents.split(/\r?\n/);
  for (const pattern of patterns) {
    for (let index = 0; index < lines.length; index += 1) {
      if (pattern.regex.test(lines[index])) {
        violations.push({
          filePath,
          line: index + 1,
          name: pattern.name,
          text: lines[index].trim(),
        });
      }
    }
    if (pattern.contentRegex) {
      pattern.contentRegex.lastIndex = 0;
      for (let match = pattern.contentRegex.exec(contents); match; match = pattern.contentRegex.exec(contents)) {
        const line = contents.slice(0, match.index).split(/\r?\n/).length;
        violations.push({
          filePath,
          line,
          name: pattern.name,
          text: match[0].trim().replace(/\s+/g, " "),
        });
      }
    }
  }
  return violations;
}

function repoRelative(filePath) {
  return path.relative(repoRoot, filePath).split(path.sep).join("/");
}

function apiPatternsForPath(relativePath) {
  const patterns = [...API_RAW_DAEMON_PATTERNS];
  if (relativePath === "core/crates/ctx-http/src/api/router.rs") {
    const index = patterns.findIndex((pattern) => pattern.name === "broad daemon handle field");
    if (index !== -1) {
      patterns.splice(index, 1);
    }
  }
  if (rawStoreBlindApiRoots.some((root) => relativePath.startsWith(root))) {
    patterns.push(...API_DOMAIN_RAW_STORE_PATTERNS);
  }
  return patterns;
}

function migratedTestPatternsForPath(relativePath) {
  if (migratedRawDaemonTestRoots.some((root) => relativePath.startsWith(root))) {
    return MIGRATED_TEST_RAW_DAEMON_PATTERNS;
  }
  return [];
}

function scanRepo() {
  const violations = [];
  if (fs.existsSync(legacyHttpDaemonRootPath)) {
    violations.push({
      filePath: repoRelative(legacyHttpDaemonRootPath),
      line: 1,
      name: "legacy ctx-http daemon root",
      text: "ctx-http/src/daemon.rs must stay physically extracted into ctx-daemon",
    });
  }
  if (fs.existsSync(legacyHttpDaemonRoot)) {
    violations.push({
      filePath: repoRelative(legacyHttpDaemonRoot),
      line: 1,
      name: "legacy ctx-http daemon directory",
      text: "ctx-http/src/daemon must stay physically extracted into ctx-daemon",
    });
  }

  for (const filePath of listRustFiles(apiRoot)) {
    if (isTestRustPath(filePath)) {
      continue;
    }
    const relativePath = repoRelative(filePath);
    const contents = stripCfgTestItems(fs.readFileSync(filePath, "utf8"));
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: apiPatternsForPath(relativePath),
      }),
    );
  }

  if (fs.existsSync(daemonHandlePath)) {
    violations.push(
      ...scanText({
        filePath: repoRelative(daemonHandlePath),
        contents: fs.readFileSync(daemonHandlePath, "utf8"),
        patterns: HANDLE_BACKDOOR_PATTERNS,
      }),
    );
  }

  const daemonFiles = [];
  if (fs.existsSync(daemonRootPath)) {
    daemonFiles.push(daemonRootPath);
  }
  if (fs.existsSync(daemonRoot)) {
    daemonFiles.push(...listRustFiles(daemonRoot));
  }
  for (const filePath of daemonFiles) {
    if (isTestRustPath(filePath)) {
      continue;
    }
    const contents = stripCfgTestItems(fs.readFileSync(filePath, "utf8"));
    violations.push(
      ...scanText({
        filePath: repoRelative(filePath),
        contents,
        patterns: DAEMON_EXTRACTION_BLOCKER_PATTERNS,
      }),
    );
  }

  for (const filePath of testSurfaceRustFiles()) {
    const relativePath = repoRelative(filePath);
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents: fs.readFileSync(filePath, "utf8"),
        patterns: TEST_RAW_DAEMON_BUCKET_PATTERNS,
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents: fs.readFileSync(filePath, "utf8"),
        patterns: migratedTestPatternsForPath(relativePath),
      }),
    );
  }

  return violations;
}

function main() {
  const violations = scanRepo();
  if (violations.length === 0) {
    console.log("ctx-http daemon boundary guard: OK");
    return;
  }

  for (const violation of violations) {
    console.error(
      `${violation.filePath}:${violation.line}: ${violation.name}: ${violation.text}`,
    );
  }
  process.exitCode = 1;
}

if (require.main === module) {
  main();
}

module.exports = {
  DAEMON_EXTRACTION_BLOCKER_PATTERNS,
  API_RAW_DAEMON_PATTERNS,
  API_DOMAIN_RAW_STORE_PATTERNS,
  HANDLE_BACKDOOR_PATTERNS,
  MIGRATED_TEST_RAW_DAEMON_PATTERNS,
  TEST_RAW_DAEMON_BUCKET_PATTERNS,
  apiPatternsForPath,
  isTestRustPath,
  migratedTestPatternsForPath,
  scanRepo,
  scanText,
  stripCfgTestItems,
};
