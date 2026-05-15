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
  "core/crates/ctx-http/tests/attachments_demo_react.rs",
  "core/crates/ctx-http/tests/codex_host_import_api.rs",
  "core/crates/ctx-http/tests/codex_login_callback_api.rs",
  "core/crates/ctx-http/tests/cache_rehydration.rs",
  "core/crates/ctx-http/tests/demo_seed_transcript_http.rs",
  "core/crates/ctx-http/tests/common/updates_failure_safety.rs",
  "core/crates/ctx-http/tests/disk_isolated_sandbox_smoke.rs",
  "core/crates/ctx-http/tests/disk_isolated_vcs_integrity.rs",
  "core/crates/ctx-http/tests/fault_matrix.rs",
  "core/crates/ctx-http/tests/gemini_live_model_catalog.rs",
  "core/crates/ctx-http/tests/global_id_routing_http.rs",
  "core/crates/ctx-http/tests/harness_container_sandbox_e2e.rs",
  "core/crates/ctx-http/tests/hot_endpoints_no_db.rs",
  "core/crates/ctx-http/tests/image_attachments_http_e2e.rs",
  "core/crates/ctx-http/tests/install_start_contract.rs",
  "core/crates/ctx-http/tests/jj_merge_queue_basics.rs",
  "core/crates/ctx-http/tests/live_provider_canary.rs",
  "core/crates/ctx-http/tests/memory_leak_e2e.rs",
  "core/crates/ctx-http/tests/merge_queue_isolation.rs",
  "core/crates/ctx-http/tests/message_idempotency.rs",
  "core/crates/ctx-http/tests/noisy_output_backpressure.rs",
  "core/crates/ctx-http/tests/provider_probe_runtime_env.rs",
  "core/crates/ctx-http/tests/provider_current_ctx_version_regressions.rs",
  "core/crates/ctx-http/tests/provider_scenarios_offline.rs",
  "core/crates/ctx-http/tests/provider_target_scoped_installs.rs",
  "core/crates/ctx-http/tests/provider_worker_reaping_offline.rs",
  "core/crates/ctx-http/tests/replay_properties.rs",
  "core/crates/ctx-http/tests/repo_clone_branch_and_safety.rs",
  "core/crates/ctx-http/tests/repo_init_initial_commit.rs",
  "core/crates/ctx-http/tests/repo_validate_destination.rs",
  "core/crates/ctx-http/tests/session_diff_unavailable.rs",
  "core/crates/ctx-http/tests/session_model_api.rs",
  "core/crates/ctx-http/tests/subagent_mcp_http.rs",
  "core/crates/ctx-http/tests/subscription_accounts_api.rs",
  "core/crates/ctx-http/tests/system_prompt_append_http.rs",
  "core/crates/ctx-http/tests/task_default_session_http.rs",
  "core/crates/ctx-http/tests/terminal_workspace_stream_separation.rs",
  "core/crates/ctx-http/tests/terminal_ws_reconnect.rs",
  "core/crates/ctx-http/tests/title_generation_local_e2e.rs",
  "core/crates/ctx-http/tests/turn_lifecycle_events.rs",
  "core/crates/ctx-http/tests/turn_terminal_reconciliation.rs",
  "core/crates/ctx-http/tests/workspace_execution_config_http.rs",
  "core/crates/ctx-http/tests/workspace_attachments_local_canonical.rs",
  "core/crates/ctx-http/tests/workspace_merge_queue_config_http.rs",
  "core/crates/ctx-http/tests/workspace_provider_model_preferences_http.rs",
  "core/crates/ctx-http/tests/workspace_active_snapshot_http.rs",
  "core/crates/ctx-http/tests/workspace_stream_context_window_metrics.rs",
  "core/crates/ctx-http/tests/workspace_stream_stress_active_heads_lag.rs",
  "core/crates/ctx-http/tests/workspace_stream_no_gaps_under_activity.rs",
  "core/crates/ctx-http/tests/worktree_archive_http.rs",
  "core/crates/ctx-http/tests/worktree_vcs_snapshot.rs",
];
const mobileStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/mobile_tokens.rs",
  "core/crates/ctx-http/src/lib_tests/auth_boundaries/mobile_tokens/",
  "core/crates/ctx-http/src/lib_tests/mobile_access_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_profile_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_secure_routes.rs",
  "core/crates/ctx-http/src/lib_tests/mobile_secure_routes/",
];

const providerCacheFacadeTestRoots = [
  "core/crates/ctx-http/src/api/providers/tests/restarts.rs",
  "core/crates/ctx-http/src/api/providers/tests/restarts/",
  "core/crates/ctx-http/src/lib_tests/provider_routes.rs",
  "core/crates/ctx-http/src/lib_tests/provider_routes/",
];

const mcpDaemonFacadeTestRoots = [
  "core/crates/ctx-http-test-support/src/mcp_daemon.rs",
  "core/crates/ctx-http-test-support/src/mcp_daemon/",
];

const sessionFixtureStoreFacadeTestRoots = [
  "core/crates/ctx-http/src/lib_tests/daemon_smoke.rs",
  "core/crates/ctx-http/src/lib_tests/daemon_smoke/",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries.rs",
  "core/crates/ctx-http/src/lib_tests/log_path_boundaries/",
  "core/crates/ctx-http/src/lib_tests/session_artifacts.rs",
  "core/crates/ctx-http/src/lib_tests/session_artifacts/",
  "core/crates/ctx-http/src/lib_tests/session_head_large_http.rs",
  "core/crates/ctx-http/src/lib_tests/session_head_large_http/",
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
    name: "router accepts broad daemon handle",
    regex: /a^/,
    contentRegex: /\bfn\s+router\s*\([^)]*\bDaemonHandle\b[^)]*\)/gm,
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
  {
    name: "raw daemon scheduler helper in migrated test surface",
    regex: /\bctx_daemon::daemon::\s*scheduler\b|(?<![\w:.])daemon::scheduler::/,
    contentRegex: /\bctx_daemon::daemon::\s*\{(?=[^}]*\bscheduler\b)[^}]*\}/g,
  },
  {
    name: "raw daemon module alias in migrated test surface",
    regex: /\bctx_daemon::daemon\s+as\s+\w+/,
  },
  {
    name: "ctx-daemon crate alias in migrated test surface",
    regex: /\b(?:use|extern\s+crate)\s+ctx_daemon\s+as\s+\w+/,
  },
  {
    name: "raw ctx-daemon outer grouped daemon import in migrated test surface",
    regex: /\bctx_daemon::\s*\{(?=[^}\n]*\b(?:daemon|self\s+as)\b)[^}\n]*\}/,
    contentRegex: /\bctx_daemon::\s*\{(?=[^}]*\n)(?=[^}]*\b(?:daemon|self\s+as)\b)[^}]*\}/g,
  },
];

const TEST_ROUTER_COMPOSITION_PATTERNS = [
  {
    name: "direct API router composition outside test router helper",
    regex: /\b(?:ctx_http::)?api::router\s*\(|\bcrate::api::router\s*\(/,
  },
];

const MOBILE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct mobile test global store access",
    regex: /\.global_store\s*\(/,
  },
];

const PROVIDER_TEST_CACHE_ACCESS_PATTERNS = [
  {
    name: "direct provider options cache closure access",
    regex: /\.test_with_provider_options_cache\s*\(/,
  },
  {
    name: "direct provider verify cache closure access",
    regex: /\.test_with_provider_verify_cache\s*\(/,
  },
  {
    name: "direct provider usage cache closure access",
    regex: /\.test_with_provider_usage_cache\s*\(/,
  },
];

const MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct MCP daemon global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct MCP daemon session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct MCP daemon workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct MCP daemon StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct MCP daemon StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
];

const SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS = [
  {
    name: "direct session fixture global store access",
    regex: /\.global_store\s*\(/,
  },
  {
    name: "direct session fixture session store access",
    regex: /\.store_for_session\s*\(/,
  },
  {
    name: "direct session fixture workspace store access",
    regex: /\.store_for_workspace\s*\(/,
  },
  {
    name: "direct session fixture uncached workspace store access",
    regex: /\.uncached_store_for_workspace\s*\(/,
  },
  {
    name: "direct session fixture task store access",
    regex: /\.store_for_task\s*\(/,
  },
  {
    name: "direct session fixture StoreManager access",
    regex: /\.stores\s*\(/,
  },
  {
    name: "direct session fixture StoreManager global access",
    regex: /\bstores\.global\s*\(/,
  },
  {
    name: "direct session fixture StoreManager workspace access",
    regex: /\bstores\.workspace\s*\(/,
  },
  {
    name: "raw session fixture ctx_store Store",
    regex: /\bctx_store::Store\b|\buse\s+ctx_store::[^;]*\bStore\b|\bStore\b/,
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
    || base === "test_support.rs"
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

function mobileStorePatternsForPath(relativePath) {
  if (mobileStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return MOBILE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function providerCachePatternsForPath(relativePath) {
  if (providerCacheFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return PROVIDER_TEST_CACHE_ACCESS_PATTERNS;
  }
  return [];
}

function mcpDaemonPatternsForPath(relativePath) {
  if (mcpDaemonFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function sessionFixtureStorePatternsForPath(relativePath) {
  if (sessionFixtureStoreFacadeTestRoots.some((root) => relativePath.startsWith(root))) {
    return SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS;
  }
  return [];
}

function routerCompositionPatternsForPath(relativePath) {
  const isLibTestsRoot = relativePath === "core/crates/ctx-http/src/lib_tests.rs";
  if (
    !relativePath.startsWith("core/crates/ctx-http/tests/")
    && !isLibTestsRoot
    && !relativePath.startsWith("core/crates/ctx-http/src/lib_tests/")
    && !relativePath.startsWith("core/crates/ctx-http/src/api/")
    && !relativePath.startsWith("core/crates/ctx-http/src/test_support")
    && !relativePath.startsWith("core/crates/ctx-http-test-support/src/")
  ) {
    return [];
  }
  if (
    relativePath === "core/crates/ctx-http/src/api/router.rs"
  ) {
    return [];
  }
  return TEST_ROUTER_COMPOSITION_PATTERNS;
}

function countRustBlockDelta(line) {
  let delta = 0;
  for (const char of line) {
    if (char === "{") {
      delta += 1;
    } else if (char === "}") {
      delta -= 1;
    }
  }
  return delta;
}

function functionSpanForDeclaration(lines, declarationIndex) {
  let sawBody = false;
  let depth = 0;
  for (let index = declarationIndex; index < lines.length; index += 1) {
    const line = lines[index];
    if (line.includes("{")) {
      sawBody = true;
    }
    depth += countRustBlockDelta(line);
    if (sawBody && depth <= 0) {
      return { start: declarationIndex, end: index };
    }
  }
  return null;
}

function isInsideDeclaredFunction(lines, index, declarationRegex) {
  for (let declarationIndex = index; declarationIndex >= 0; declarationIndex -= 1) {
    if (!declarationRegex.test(lines[declarationIndex])) {
      continue;
    }
    const span = functionSpanForDeclaration(lines, declarationIndex);
    return span !== null && index >= span.start && index <= span.end;
  }
  return false;
}

function isAllowedRouterHelperComposition({ filePath, lines, index, line }) {
  if (
    filePath === "core/crates/ctx-http/tests/common/mod.rs"
    && /api::router\s*\(\s*api::RouteHandles::from_daemon_handle\s*\(\s*daemon\.handle\s*\(\s*\)\s*\)\s*\)/.test(line)
    && isInsideDeclaredFunction(lines, index, /\bpub\s+fn\s+router_for_daemon\s*\(/)
  ) {
    return true;
  }
  if (
    filePath === "core/crates/ctx-http/src/lib_tests.rs"
    && /api::router\s*\(\s*api::RouteHandles::from_daemon_handle\s*\(\s*daemon\.handle\s*\(\s*\)\s*\)\s*\)/.test(line)
    && isInsideDeclaredFunction(lines, index, /\bfn\s+test_router\s*\(/)
  ) {
    return true;
  }
  if (
    filePath === "core/crates/ctx-http/src/api/tasks/storage_admission_http_tests/fixtures.rs"
    && /crate::api::router\s*\(\s*crate::api::RouteHandles::from_daemon_handle\s*\(\s*state\.handle\s*\(\s*\)\s*\)\s*\)/.test(line)
    && isInsideDeclaredFunction(lines, index, /\bpub\s*\(\s*super\s*\)\s+fn\s+test_router\s*\(/)
  ) {
    return true;
  }
  if (
    filePath === "core/crates/ctx-http-test-support/src/mcp_daemon/router.rs"
    && /ctx_http::api::router\s*\(\s*ctx_http::api::RouteHandles::from_daemon_handle\s*\(\s*daemon\.handle\s*\(\s*\)\s*,?\s*\)\s*\)/.test(lines.slice(index, index + 4).join(" "))
    && isInsideDeclaredFunction(lines, index, /\bpub\s*\(\s*crate\s*\)\s+fn\s+spawn_router_for_daemon\s*\(/)
  ) {
    return true;
  }
  return false;
}

function scanRouterComposition({ filePath, contents, patterns }) {
  const violations = [];
  const lines = contents.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    for (const pattern of patterns) {
      if (!pattern.regex.test(line)) {
        continue;
      }
      if (isAllowedRouterHelperComposition({ filePath, lines, index, line })) {
        continue;
      }
      violations.push({
        filePath,
        line: index + 1,
        name: pattern.name,
        text: line.trim(),
      });
    }
  }
  return violations;
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
    const contents = fs.readFileSync(filePath, "utf8");
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: TEST_RAW_DAEMON_BUCKET_PATTERNS,
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: migratedTestPatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: mobileStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: providerCachePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: mcpDaemonPatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanText({
        filePath: relativePath,
        contents,
        patterns: sessionFixtureStorePatternsForPath(relativePath),
      }),
    );
    violations.push(
      ...scanRouterComposition({
        filePath: relativePath,
        contents,
        patterns: routerCompositionPatternsForPath(relativePath),
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
  MCP_DAEMON_TEST_STORE_ACCESS_PATTERNS,
  MOBILE_TEST_STORE_ACCESS_PATTERNS,
  PROVIDER_TEST_CACHE_ACCESS_PATTERNS,
  SESSION_FIXTURE_TEST_STORE_ACCESS_PATTERNS,
  TEST_ROUTER_COMPOSITION_PATTERNS,
  TEST_RAW_DAEMON_BUCKET_PATTERNS,
  apiPatternsForPath,
  isTestRustPath,
  mcpDaemonPatternsForPath,
  migratedTestPatternsForPath,
  mobileStorePatternsForPath,
  providerCachePatternsForPath,
  routerCompositionPatternsForPath,
  scanRepo,
  scanRouterComposition,
  scanText,
  sessionFixtureStorePatternsForPath,
  stripCfgTestItems,
};
