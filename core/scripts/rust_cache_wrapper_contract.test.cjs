const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");

function read(relativePath) {
  return fs.readFileSync(path.resolve(repoRoot, relativePath), "utf8");
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
