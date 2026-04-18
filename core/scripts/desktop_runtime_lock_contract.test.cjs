const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));

test("desktop runtime lock umbrella delegates to family-shaped subcommands", () => {
  const scripts = packageJson.scripts;

  assert.match(
    scripts["desktop:runtime:lock:test:distribution-install"],
    /pnpm bazel:desktop:runtime:lock:check-matrix && pnpm bazel:desktop:runtime:lock:validate && pnpm bazel:desktop:check:versions/,
  );
  assert.doesNotMatch(
    scripts["desktop:runtime:lock:test:distribution-install"],
    /validate_provider_auth_matrix\.test\.cjs|desktop_e2e_secret_contract\.test\.cjs|updater_e2e_aws\.test\.cjs|tauri_tools_lock_contract\.test\.cjs/,
  );

  assert.equal(
    scripts["desktop:runtime:lock:test:provider-auth"],
    "pnpm bazel:provider-auth:validate && node --test scripts/desktop_e2e_preflight.test.cjs",
  );
  assert.equal(
    scripts["desktop:runtime:lock:test:sandbox-runtime"],
    "node --test scripts/desktop_sync_resources_remote_daemon_policy.test.cjs scripts/desktop_sync_resources_avf_guest_runtime.test.cjs scripts/desktop_daemon_sidecar_name.test.cjs apps/desktop/automation/wdio.container-assets.test.cjs",
  );
  assert.equal(
    scripts["desktop:runtime:lock:test:updates-release"],
    "node --test scripts/updater_e2e_aws.test.cjs scripts/updater_e2e_remote_matrix_cloud_provider.test.cjs",
  );
  assert.equal(
    scripts["desktop:runtime:lock:test:toolchain-bootstrap"],
    "node --test scripts/tauri_tools_lock_contract.test.cjs",
  );
  assert.equal(
    scripts["desktop:runtime:lock:test"],
    "pnpm desktop:runtime:lock:test:distribution-install && pnpm desktop:runtime:lock:test:provider-auth && pnpm desktop:runtime:lock:test:sandbox-runtime && pnpm desktop:runtime:lock:test:updates-release && pnpm desktop:runtime:lock:test:toolchain-bootstrap",
  );
});
