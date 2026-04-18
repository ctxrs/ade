const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));

test("desktop system parity uses explicit Bazel-backed contract entrypoints instead of the runtime-lock umbrella", () => {
  const scripts = packageJson.scripts;

  assert.equal(scripts["bazel:desktop:provider-matrix:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_provider_matrix_archive_contracts");
  assert.equal(scripts["bazel:desktop:launch-mode:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_launch_mode_contracts");
  assert.equal(scripts["bazel:desktop:bundle:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_bundle_contracts");
  assert.equal(scripts["bazel:bundled-harness:dependency:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:bundled_harness_dependency_contracts");
  assert.equal(scripts["bazel:desktop:e2e:preflight:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_e2e_preflight_contracts");
  assert.equal(scripts["bazel:desktop:sync-resources:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:desktop_sync_resources_contracts");
  assert.equal(scripts["bazel:providers:e2e:bundle:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:providers_e2e_bundle_contracts");
  assert.equal(scripts["bazel:providers:linux-arm:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:linux_arm_provider_contracts");
  assert.equal(scripts["bazel:updater:cloud:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:updater_cloud_contracts");
  assert.equal(scripts["bazel:tauri:tools:lock:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:tauri_tools_lock_contracts");
  assert.equal(scripts["bazel:desktop:deps:contracts"], "node scripts/run_bazel_pilot.cjs run //core/scripts:install_desktop_deps_contracts");

  assert.equal(scripts["desktop:runtime:lock:test"], undefined);
  assert.equal(scripts["desktop:runtime:lock:test:distribution-install"], undefined);
  assert.equal(scripts["desktop:runtime:lock:test:provider-auth"], undefined);
  assert.equal(scripts["desktop:runtime:lock:test:sandbox-runtime"], undefined);
  assert.equal(scripts["desktop:runtime:lock:test:updates-release"], undefined);
  assert.equal(scripts["desktop:runtime:lock:test:toolchain-bootstrap"], undefined);
});
