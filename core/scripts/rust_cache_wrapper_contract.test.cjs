const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const unsafeCargoCommandPattern = /(^|[^A-Za-z0-9_./-])cargo\s+(?:\+\S+\s+)?(build|test|run|clippy|check|doc|nextest|zigbuild|llvm-cov)\b/;

const pathAllowlist = new Map([
  [
    "scripts/codex_crp_stage_release_archives.sh",
    {
      owner: "provider-artifacts",
      category: "provider-artifact-packaging",
      rationale:
        "Builds distributable codex-crp release archives for explicit target triples; the script sets CARGO_TARGET_DIR for host and container builds and computes artifact paths from that target root.",
    },
  ],
  [
    "scripts/deploy_ctx_tunnel_hetzner.sh",
    {
      owner: "mobile-tunnel",
      category: "remote-release-build",
      rationale:
        "Deploys git archive HEAD on a remote host and pins remote Cargo output to REMOTE_SRC/core/target; this path is outside local workspace cache management.",
    },
  ],
  [
    "scripts/ensure_bundled_harnesses.sh",
    {
      owner: "provider-artifacts",
      category: "provider-artifact-packaging",
      rationale:
        "Assembles runtime provider bundles outside the core Bazel graph; the script exports an isolated bundle CARGO_TARGET_DIR before local bridge/adapter builds.",
    },
  ],
  [
    "scripts/lib/bundled_harnesses_providers.sh",
    {
      owner: "provider-artifacts",
      category: "provider-artifact-packaging",
      rationale:
        "Sourced by bundled harness assembly and preserves explicit target roots for cross-target provider artifacts.",
    },
  ],
  [
    "scripts/publish_adapter_supabase.sh",
    {
      owner: "provider-artifacts",
      category: "provider-artifact-packaging",
      rationale:
        "Publishes harness adapter artifacts outside the core Bazel graph; each Cargo build writes to a staging-local CARGO_TARGET_DIR.",
    },
  ],
  [
    "core/scripts/desktop_sync_resources.cjs",
    {
      owner: "desktop-runtime",
      category: "diagnostic-text",
      rationale:
        "Contains an actionable error message mentioning cargo build; desktop sidecar build execution is handled by prepared Bazel/automation paths.",
    },
  ],
  [
    "scripts/buildbuddy/reset_macos_release_state.sh",
    {
      owner: "release-ops",
      category: "process-cleanup-pattern",
      rationale:
        "Matches a command string while cleaning stale macOS release processes; it is not a build invocation.",
    },
  ],
]);
const nonInvocationAllowlistCategories = new Set([
  "diagnostic-text",
  "process-cleanup-pattern",
]);
const allowedCargoLinePatterns = new Map([
  [
    "scripts/codex_crp_stage_release_archives.sh",
    [
      /cargo build --manifest-path \/work\/Cargo\.toml -p codex-crp --release --target '\$rust_target'/,
      /cargo zigbuild --manifest-path "\$WORKSPACE_DIR\/Cargo\.toml" -p codex-crp --release --target "\$rust_target"/,
      /cargo build --manifest-path "\$WORKSPACE_DIR\/Cargo\.toml" -p codex-crp --release --target "\$rust_target"/,
    ],
  ],
  [
    "scripts/deploy_ctx_tunnel_hetzner.sh",
    [
      /CARGO_TARGET_DIR='\$REMOTE_SRC\/core\/target' cargo build --release --locked -p ctx-tunnel-control-plane -p ctx-tunnel-router -p ctx-tunnel-relay/,
      /CARGO_TARGET_DIR='\$REMOTE_SRC\/core\/target' cargo build --release --locked -p ctx-tunnel-store --bin ctx-tunnel-cleanup/,
    ],
  ],
  [
    "scripts/ensure_bundled_harnesses.sh",
    [
      /cargo \+stable build --release --target '\$rust_target'/,
      /\(cd "\$BRIDGE_DIR" && cargo build --release\)/,
      /\(cd "\$BRIDGE_DIR" && cargo build --release --target "\$rust_target"\)/,
      /\(cd "\$LOCAL_ADAPTERS_DIR\/\$dir" && cargo build --release --target "\$rust_target"\)/,
    ],
  ],
  [
    "scripts/lib/bundled_harnesses_providers.sh",
    [
      /cargo \+stable build --manifest-path \/work\/Cargo\.toml -p codex-crp --target '\$rust_target'/,
      /env CARGO_TARGET_DIR="\$target_dir" .* cargo zigbuild --manifest-path "\$CODEX_CRP_WORKSPACE\/Cargo\.toml"/,
      /env CARGO_TARGET_DIR="\$target_dir" .* cargo build --manifest-path "\$CODEX_CRP_WORKSPACE\/Cargo\.toml"/,
      /\(cd "\$LOCAL_ADAPTERS_DIR\/\$dir" && cargo build --release --target "\$rust_target"\)/,
    ],
  ],
  [
    "scripts/publish_adapter_supabase.sh",
    [
      /\(cd "\$ROOT_DIR\/harness-adapters\/\$dir" && CARGO_TARGET_DIR="\$target_dir" cargo build --release\)/,
    ],
  ],
]);

function read(relativePath) {
  return fs.readFileSync(path.resolve(repoRoot, relativePath), "utf8");
}

function listFiles(relativeDir, predicate) {
  const absoluteDir = path.resolve(repoRoot, relativeDir);
  const result = [];
  for (const entry of fs.readdirSync(absoluteDir, { withFileTypes: true })) {
    const absolutePath = path.join(absoluteDir, entry.name);
    const relativePath = path.relative(repoRoot, absolutePath);
    if (entry.isDirectory()) {
      result.push(...listFiles(relativePath, predicate));
      continue;
    }
    if (predicate(relativePath)) {
      result.push(relativePath);
    }
  }
  return result;
}

function cargoCommandLines(relativePath) {
  const lines = read(relativePath).split(/\r?\n/);
  const matches = [];
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (/^\s*#/.test(line)) {
      continue;
    }
    if (/^\s*cargo\s*,?\s*$/.test(line)) {
      continue;
    }
    if (!/(^\s*|[;&|({]\s*)(env\s+)?([A-Z_][A-Z0-9_]*=(?:"[^"]*"|'[^']*'|[^\s]+)\s+)*cargo(\s|\+|$)/.test(line)) {
      continue;
    }
    if (/(command -v|require_cmd|require_command|need_cmd|missing|requires|echo|printf|assert\.|doesNotMatch|match\(|cargoHome|cargo = Path)/.test(line)) {
      continue;
    }
    matches.push({
      line: index + 1,
      text: line,
      context: lines.slice(Math.max(0, index - 4), index + 1).join("\n"),
    });
  }
  return matches;
}

function listFilesRecursive(rootRelativePath, predicate) {
  const rootPath = path.resolve(repoRoot, rootRelativePath);
  if (!fs.existsSync(rootPath)) {
    return [];
  }
  const results = [];
  const stack = [rootPath];
  while (stack.length > 0) {
    const current = stack.pop();
    const stat = fs.statSync(current);
    if (stat.isDirectory()) {
      for (const entry of fs.readdirSync(current)) {
        if (entry === "node_modules" || entry === ".git") {
          continue;
        }
        stack.push(path.join(current, entry));
      }
      continue;
    }
    const relativePath = path.relative(repoRoot, current).replace(/\\/g, "/");
    if (predicate(relativePath)) {
      results.push(relativePath);
    }
  }
  return results.sort();
}

function isLiveAutomationFile(relativePath) {
  if (relativePath.startsWith(".ctx/docs/")) {
    const basename = path.basename(relativePath);
    return relativePath.endsWith(".md")
      && !basename.startsWith("exec-plan-")
      && !basename.endsWith(".generated.md");
  }
  if (relativePath === "core/package.json") {
    return true;
  }
  if (relativePath.startsWith(".buildkite/")) {
    return /\.(ya?ml|sh|cjs|mjs)$/.test(relativePath);
  }
  if (relativePath.startsWith("scripts/") || relativePath.startsWith("core/scripts/")) {
    if (/\.(test|spec)\.(cjs|mjs|js|ts)$/.test(relativePath)) {
      return false;
    }
    return /\.(sh|cjs|mjs|js|yaml|yml)$/.test(relativePath);
  }
  return false;
}

function collectLiveScanFiles() {
  const roots = [
    ".ctx/docs",
    "core/package.json",
    ".buildkite",
    "scripts",
    "core/scripts",
  ];
  const files = new Set();
  for (const root of roots) {
    const rootPath = path.resolve(repoRoot, root);
    if (!fs.existsSync(rootPath)) {
      continue;
    }
    if (fs.statSync(rootPath).isFile()) {
      if (isLiveAutomationFile(root)) {
        files.add(root);
      }
      continue;
    }
    for (const file of listFilesRecursive(root, isLiveAutomationFile)) {
      files.add(file);
    }
  }
  return [...files].sort();
}

function hasWrapperContext(context) {
  return /run_with_ctx_cache_env\.cjs/.test(context)
    || /ctx_cache_run_workspace_rust/.test(context)
    || /ctx_cache_run_verify_quick_rust/.test(context);
}

function isTargetIsolatedContext(context) {
  return /CARGO_TARGET_DIR=/.test(context) || /--target-dir\b/.test(context);
}

function validateAllowlist() {
  for (const [relativePath, entry] of pathAllowlist.entries()) {
    assert.ok(entry.owner, `${relativePath} allowlist entry needs an owner`);
    assert.ok(entry.category, `${relativePath} allowlist entry needs a category`);
    assert.ok(entry.rationale, `${relativePath} allowlist entry needs a rationale`);
    if (!nonInvocationAllowlistCategories.has(entry.category)) {
      assert.ok(
        allowedCargoLinePatterns.has(relativePath),
        `${relativePath} build-producing allowlist entry needs line patterns`,
      );
    }
  }
}

function unsafeCargoFindings() {
  validateAllowlist();
  const findings = [];
  for (const relativePath of collectLiveScanFiles()) {
    const text = read(relativePath);
    const lines = text.split(/\r?\n/);
    const allowlistEntry = pathAllowlist.get(relativePath);
    for (let index = 0; index < lines.length; index += 1) {
      const line = lines[index];
      if (!unsafeCargoCommandPattern.test(line)) {
        continue;
      }
      const contextStart = Math.max(0, index - 6);
      const context = lines.slice(contextStart, index + 1).join("\n");
      if (hasWrapperContext(context)) {
        continue;
      }
      if (allowlistEntry) {
        if (nonInvocationAllowlistCategories.has(allowlistEntry.category)) {
          continue;
        }
        const patterns = allowedCargoLinePatterns.get(relativePath) || [];
        if (isTargetIsolatedContext(text) && patterns.some((pattern) => pattern.test(line))) {
          continue;
        }
      }
      findings.push(
        `${relativePath}:${index + 1}: unsafe raw Cargo command without ctx cache wrapper or allowlisted target isolation: ${line.trim()}`,
      );
    }
  }
  return findings;
}

function readCorePackageScripts() {
  return JSON.parse(read("core/package.json")).scripts || {};
}

test("ctx cache wrapper entrypoints expose cwd-aware observability metadata", () => {
  const printScript = read("core/scripts/print_ctx_cache_env.cjs");
  const runScript = read("core/scripts/run_with_ctx_cache_env.cjs");

  assert.match(printScript, /--cwd <dir>/);
  assert.match(printScript, /CTX_RUST_CACHE_SOURCE/);
  assert.match(printScript, /CTX_RUST_CACHE_MODE/);
  assert.match(printScript, /CTX_RUST_CACHE_SCOPE_KEY/);
  assert.match(printScript, /CTX_RUST_CACHE_SCCACHE/);
  assert.match(printScript, /TMPDIR: env\.TMPDIR/);
  assert.match(printScript, /TMP: env\.TMP/);
  assert.match(printScript, /TEMP: env\.TEMP/);

  assert.match(runScript, /CTX_RUST_CACHE_SOURCE = "run_with_ctx_cache_env"/);
  assert.match(runScript, /CTX_RUST_CACHE_WRAPPED = "1"/);
  assert.match(runScript, /\[ctx-cache\] source=%s mode=%s scope=%s target=%s sccache=%s volatile_root_mode=%s/);
});

test("codex crp rust helpers export wrapper-managed cache env", () => {
  const helperScript = read("scripts/lib/codex_crp_build_env.sh");

  assert.match(helperScript, /codex_crp_export_cache_env\(\)/);
  assert.match(helperScript, /print_ctx_cache_env\.cjs/);
  assert.match(helperScript, /codex_crp_run_rust\(\)/);
  assert.match(helperScript, /run_with_ctx_cache_env\.cjs/);
  assert.match(helperScript, /CTX_SESSION_ID/);
});

test("cloud, mobile, and coverage automation route rust work through ctx cache wrappers", () => {
  const cases = [
    "scripts/run_mobile_e2e.sh",
    "core/scripts/run-daemon-coverage.sh",
  ];

  for (const relativePath of cases) {
    const text = read(relativePath);
    assert.match(text, /run_with_ctx_cache_env\.cjs/, `${relativePath} should use the ctx cache wrapper`);
    assert.match(text, /--mode workspace/, `${relativePath} should use the workspace cache mode`);
  }
  for (const relativePath of [
  ]) {
    const text = read(relativePath);
    assert.match(text, /print_ctx_cache_env\.cjs/, `${relativePath} should derive the volatile cache layout`);
    assert.doesNotMatch(text, /CARGO_TARGET_DIR=target\b/, `${relativePath} must not write to repo-local target`);
  }
});

test("automation cargo invocations are wrapper-managed or explicitly isolated", () => {
  const packageJson = JSON.parse(read("core/package.json"));
  const packageRawCargo = Object.entries(packageJson.scripts)
    .filter(([, command]) => /\bcargo\b/.test(command) && !/run_with_ctx_cache_env\.cjs/.test(command))
    .map(([name, command]) => ({ path: "core/package.json", line: name, text: command, context: command }));

  const shellFiles = [
    ...listFiles("scripts", (relativePath) => relativePath.endsWith(".sh")),
    ...listFiles("core/scripts", (relativePath) => relativePath.endsWith(".sh")),
  ];
  const shellCargo = shellFiles.flatMap((relativePath) =>
    cargoCommandLines(relativePath).map((match) => ({ path: relativePath, ...match })));

  const allowedRawCargo = [
    {
      path: "core/package.json",
      pattern: /cargo fmt --all -- --check/,
      rationale: "cargo fmt does not create a build target tree",
    },
    {
      path: "scripts/buildbuddy/release_job_lib.sh",
      pattern: /cargo\s*\\?$/,
      rationale: "desktop automation preflight builds with DESKTOP_AUTOMATION_TARGET_DIR",
    },
    {
      path: "scripts/codex_crp_stage_release_archives.sh",
      pattern: /cargo (zigbuild|build)|cargo \+stable build/,
      rationale: "release archive staging pins CARGO_TARGET_DIR to stage/container target roots",
    },
    {
      path: "scripts/deploy_ctx_tunnel_hetzner.sh",
      pattern: /CARGO_TARGET_DIR='\$REMOTE_SRC\/core\/target' cargo build --release --locked/,
      rationale: "remote tunnel deploy builds git archive HEAD into a remote target directory outside local cache management",
    },
    {
      path: "scripts/ensure_bundled_harnesses.sh",
      pattern: /cargo (build|\+stable build)/,
      rationale: "bundle builds export per-bundle CARGO_TARGET_DIR or container /target",
    },
    {
      path: "scripts/publish_adapter_supabase.sh",
      pattern: /CARGO_TARGET_DIR="\$target_dir" cargo build --release/,
      rationale: "legacy adapter publish stages into a temp target under STAGING",
    },
    {
      path: "scripts/ensure_macos_avf_build_tools.sh",
      pattern: /cargo install cargo-zigbuild --locked/,
      rationale: "host tool bootstrap installs cargo-zigbuild rather than building repo targets",
    },
    {
      path: "core/scripts/avf_linux_ci_smoke.sh",
      pattern: /cargo metadata --manifest-path/,
      rationale: "metadata lookup discovers Cargo target directories and does not build targets",
    },
    {
      path: "scripts/tests/macos_agent_pre_command_cleanup_tolerates_failure.sh",
      pattern: /cargo --version >/,
      rationale: "macOS cleanup smoke only probes whether cargo is installed before continuing",
    },
  ];

  const failures = [];
  const hits = new Set();
  for (const match of [...packageRawCargo, ...shellCargo]) {
    if (/run_with_ctx_cache_env\.cjs|ctx_cache_run_workspace_rust/.test(match.context)) {
      continue;
    }
    const allowed = allowedRawCargo.find((entry) =>
      entry.path === match.path && entry.pattern.test(match.text));
    if (allowed) {
      hits.add(`${allowed.path}:${allowed.pattern}`);
      continue;
    }
    failures.push(`${match.path}:${match.line}: ${match.text.trim()}`);
  }

  assert.deepEqual(failures, [], "raw cargo in automation must be wrapper-managed or explicitly allowlisted");
  for (const entry of allowedRawCargo) {
    assert.equal(
      hits.has(`${entry.path}:${entry.pattern}`),
      true,
      `allowed raw cargo exception is stale or unproven: ${entry.path} (${entry.rationale})`,
    );
  }
});

test("repo-owned automation rust callers use ctx cache wrappers instead of naked cargo", () => {
  const cases = [
    {
      path: "core/Makefile",
      required: [
        /CTX_MAKE_SESSION_ID/,
        /CTX_CACHE_SCOPE_KEY/,
        /print_ctx_cache_env\.cjs/,
        /run_with_ctx_cache_env\.cjs/,
      ],
    },
    {
      path: "scripts/buildbuddy/run_agent_gate.sh",
      required: [/ctx_cache_env_lib\.sh/, /ctx_cache_export_workspace_env/],
    },
    {
      path: "scripts/buildbuddy/run_linux_aux_ci.sh",
      required: [/ctx_cache_env_lib\.sh/, /run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "scripts/buildbuddy/run_provider_auth_matrix.sh",
      required: [/ctx_cache_env_lib\.sh/, /ctx_cache_run_workspace_rust/],
    },
    {
      path: "scripts/buildbuddy/run_avf_smoke.sh",
      required: [/ctx_cache_env_lib\.sh/, /ctx_cache_run_workspace_rust/],
    },
    {
      path: "scripts/buildbuddy/run_desktop_break_matrix.sh",
      required: [/ctx_cache_env_lib\.sh/, /ctx_cache_run_workspace_rust/],
    },
    {
      path: "scripts/buildbuddy/run_desktop_automation_smoke.sh",
      required: [/ctx_cache_env_lib\.sh/, /ctx_cache_run_workspace_rust/],
    },
    {
      path: "scripts/e2e_install_all_docker.sh",
      required: [/print_ctx_cache_env\.cjs/, /run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "scripts/provider_deps_build_staging.sh",
      required: [/print_ctx_cache_env\.cjs/, /run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/run-rust-suite.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/providers_e2e.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/run-anomaly-suite.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/load_smoke.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/run-fuzz-regression.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/avf_linux_ci_smoke.sh",
      required: [/print_ctx_cache_env\.cjs/, /run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/title_generation_local_e2e.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "core/scripts/memleak_soak.sh",
      required: [/run_with_ctx_cache_env\.cjs/],
    },
    {
      path: "scripts/build_codex_crp.sh",
      required: [/codex_crp_export_cache_env/, /codex_crp_run_rust/],
    },
    {
      path: "scripts/test_codex_crp.sh",
      required: [/codex_crp_export_cache_env/, /codex_crp_run_rust/],
    },
    {
      path: "scripts/dev_install_codex_crp.sh",
      required: [/codex_crp_export_cache_env/, /codex_crp_run_rust/],
    },
    {
      path: "scripts/codex_crp_capture.sh",
      required: [/codex_crp_export_cache_env/, /codex_crp_run_rust/],
    },
    {
      path: "scripts/codex_crp_dump_codex_events.sh",
      required: [/codex_crp_export_cache_env/, /codex_crp_run_rust/],
    },
    {
      path: "scripts/crp_dev_harness.sh",
      required: [/codex_crp_export_cache_env/, /codex_crp_run_rust/, /run_with_ctx_cache_env\.cjs/],
    },
  ];

  for (const entry of cases) {
    const scriptText = read(entry.path);
    for (const pattern of entry.required) {
      assert.match(
        scriptText,
        pattern,
        `${entry.path} should route rust work through the shared ctx cache wrapper path`,
      );
    }
    assert.doesNotMatch(
      scriptText,
      /(^|\n)\s*cargo\s+(build|test|run)\b/m,
      `${entry.path} should not invoke cargo directly from the script body`,
    );
  }
});

test("remote daemon network soak pins cache scope across wrapped cargo calls", () => {
  const scriptText = read("scripts/buildkite/run_remote_daemon_ui_network_soak.sh");
  const scopeIndex = scriptText.indexOf('CTX_CACHE_SCOPE_KEY="remote-daemon-ui-network-soak-');
  const buildIndex = scriptText.indexOf("cargo build");
  const metadataIndex = scriptText.indexOf("cargo metadata");

  assert.notEqual(scopeIndex, -1, "remote daemon soak should derive a stable cache scope");
  assert.notEqual(buildIndex, -1, "remote daemon soak should build ctx binaries");
  assert.notEqual(metadataIndex, -1, "remote daemon soak should read cargo metadata");
  assert.ok(scopeIndex < buildIndex, "cache scope must be pinned before the wrapped cargo build");
  assert.ok(buildIndex < metadataIndex, "metadata lookup should reuse the build cache scope");
  assert.match(scriptText, /export CTX_CACHE_SCOPE_KEY/);
});

test("core Makefile uses the make session for both cache scope keys", () => {
  const makefile = read("core/Makefile");

  assert.match(
    makefile,
    /CTX_CACHE_EXPORT_CMD = export CTX_SESSION_ID="\$\(CTX_MAKE_SESSION_ID\)"; export CTX_CACHE_SCOPE_KEY="\$\(CTX_MAKE_SESSION_ID\)"; eval/,
  );
  assert.match(makefile, /--setenv CTX_SESSION_ID="\$\(CTX_MAKE_SESSION_ID\)" \\\n\s*--setenv CTX_CACHE_SCOPE_KEY="\$\(CTX_MAKE_SESSION_ID\)"/);
  assert.match(
    makefile,
    /CTX_SESSION_ID="\$\(CTX_MAKE_SESSION_ID\)" CTX_CACHE_SCOPE_KEY="\$\(CTX_MAKE_SESSION_ID\)" CTX_BUNDLE_DIR/,
  );
});

test("bundled harness helper scripts wrap host-side rust builds and leave container builds explicit", () => {
  const bundleScript = read("scripts/ensure_bundled_harnesses.sh");
  const providerHelper = read("scripts/lib/bundled_harnesses_providers.sh");

  assert.match(bundleScript, /CTX_SESSION_ID:-bundled-harnesses-/);
  assert.match(bundleScript, /run_with_ctx_cache_env\.cjs/);
  assert.doesNotMatch(bundleScript, /\(cd "\$BRIDGE_DIR" && cargo build --release(?: --target "\$rust_target")?\)/);
  assert.doesNotMatch(bundleScript, /\(cd "\$LOCAL_ADAPTERS_DIR\/\$dir" && cargo build --release --target "\$rust_target"\)/);
  assert.match(bundleScript, /cargo \+stable build --release --target '\$rust_target'/);

  assert.match(providerHelper, /run_with_ctx_cache_env\.cjs/);
  assert.doesNotMatch(providerHelper, /env CARGO_TARGET_DIR="\$target_dir" "\$\{cargo_profile_env\[@\]\}" cargo build/);
  assert.doesNotMatch(providerHelper, /\(cd "\$LOCAL_ADAPTERS_DIR\/\$dir" && cargo build --release --target "\$rust_target"\)/);
  assert.match(providerHelper, /\$\{cargo_profile_env\[\*\]\} cargo \+stable build/);
});

test("live automation and agent-facing docs do not introduce unsafe raw Cargo build paths", () => {
  assert.deepEqual(unsafeCargoFindings(), []);
});

test("top-level package test script routes through the stable agent gate", () => {
  const scripts = readCorePackageScripts();
  assert.equal(scripts.test, "pnpm test:agent");
});
