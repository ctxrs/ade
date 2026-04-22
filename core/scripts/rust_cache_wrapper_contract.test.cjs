const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");

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

test("ctx cache wrapper entrypoints expose cwd-aware observability metadata", () => {
  const printScript = read("core/scripts/print_ctx_cache_env.cjs");
  const runScript = read("core/scripts/run_with_ctx_cache_env.cjs");

  assert.match(printScript, /--cwd <dir>/);
  assert.match(printScript, /CTX_RUST_CACHE_SOURCE/);
  assert.match(printScript, /CTX_RUST_CACHE_MODE/);
  assert.match(printScript, /CTX_RUST_CACHE_SCOPE_KEY/);
  assert.match(printScript, /CTX_RUST_CACHE_SCCACHE/);

  assert.match(runScript, /CTX_RUST_CACHE_SOURCE = "run_with_ctx_cache_env"/);
  assert.match(runScript, /CTX_RUST_CACHE_WRAPPED = "1"/);
  assert.match(runScript, /\[ctx-cache\] source=%s mode=%s scope=%s target=%s sccache=%s volatile_root_mode=%s/);
});

test("cloud, mobile, and coverage automation route rust work through ctx cache wrappers", () => {
  const cases = [
    "scripts/run_mobile_e2e.sh",
    "scripts/cloud_gateway_e2e_azure.sh",
    "scripts/cloud_gateway_e2e_gcp.sh",
    "core/scripts/run-daemon-coverage.sh",
  ];

  for (const relativePath of cases) {
    const text = read(relativePath);
    assert.match(text, /run_with_ctx_cache_env\.cjs/, `${relativePath} should use the ctx cache wrapper`);
    assert.match(text, /--mode workspace/, `${relativePath} should use the workspace cache mode`);
  }
  for (const relativePath of [
    "scripts/cloud_gateway_e2e_azure.sh",
    "scripts/cloud_gateway_e2e_gcp.sh",
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
      path: "scripts/ensure_bundled_harnesses.sh",
      pattern: /cargo (build|\+stable build)/,
      rationale: "bundle builds export per-bundle CARGO_TARGET_DIR or container /target",
    },
    {
      path: "scripts/lib/bundled_harnesses_providers.sh",
      pattern: /cargo (zigbuild|build|\+stable build)/,
      rationale: "provider bundle builds pass per-target CARGO_TARGET_DIR",
    },
    {
      path: "scripts/publish_adapter_supabase.sh",
      pattern: /CARGO_TARGET_DIR="\$cargo_target_dir" cargo build --release/,
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
      path: "core/scripts/build_avf_linux_guest_agent.sh",
      pattern: /cargo metadata --manifest-path/,
      rationale: "metadata lookup discovers Cargo target directories and does not build targets",
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
