const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const toolsBuild = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "BUILD.bazel"), "utf8");
const helperScript = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "provider_node_archive.sh"), "utf8");
const managedHelperScript = fs.readFileSync(
  path.join(repoRoot, "tools", "bazel", "provider_managed_install_archive.sh"),
  "utf8",
);
const piBuild = fs.readFileSync(path.join(repoRoot, "harness-adapters", "pi-acp", "BUILD.bazel"), "utf8");
const providerAccountsBuild = fs.readFileSync(
  path.join(repoRoot, "core", "crates", "ctx-provider-accounts", "BUILD.bazel"),
  "utf8",
);

test("provider node archive helper is exported for Bazel package targets", () => {
  assert.match(toolsBuild, /"provider_node_archive\.sh"/);
  assert.match(toolsBuild, /"provider_managed_install_archive\.sh"/);
  assert.match(helperScript, /pnpm install --frozen-lockfile --ignore-scripts/);
  assert.match(helperScript, /pnpm run build/);
  assert.match(helperScript, /create_deterministic_tar_gz "\$archive_path" "\$WORKSPACE_DIR" package\.json dist node_modules/);
  assert.match(managedHelperScript, /scripts\/provider_deps_build_staging\.sh/);
  assert.match(managedHelperScript, /--providers "\$PROVIDER_ID"/);
  assert.match(managedHelperScript, /missing staged provider artifact/);
});

test("open-source provider adapters expose Bazel-run archive targets", () => {
  assert.match(piBuild, /name = "provider-stage-archive"/);
  assert.match(piBuild, /"\/\/tools\/bazel:provider_node_archive\.sh"/);
  assert.match(piBuild, /"harness-adapters\/pi-acp"/);
  assert.match(piBuild, /"dist\/bin\/pi-acp\.js"/);

  assert.match(providerAccountsBuild, /name = "goose-provider-stage-archive"/);
  assert.match(providerAccountsBuild, /name = "openhands-provider-stage-archive"/);
  assert.match(providerAccountsBuild, /"\/\/tools\/bazel:provider_managed_install_archive\.sh"/);
  assert.match(providerAccountsBuild, /"goose"/);
  assert.match(providerAccountsBuild, /"openhands"/);
});
