const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const workflowPath = path.join(repoRoot, ".github", "workflows", "desktop-system-parity.yml");

test("desktop system parity workflow installs desktop deps and prepares parity bundles", () => {
  const text = fs.readFileSync(workflowPath, "utf8");

  assert.match(text, /name: Desktop system parity/);
  assert.match(text, /- name: Install Linux desktop system deps/);
  assert.match(text, /\.\/scripts\/install_desktop_deps_linux_ubuntu\.sh/);
  assert.match(text, /sudo apt-get install -y sqlite3 binutils/);
  assert.match(text, /- name: Runtime lock contract tests/);
  assert.match(text, /- name: Runtime lock matrix consistency/);
  assert.match(text, /install_desktop_deps_linux_ubuntu_contract\.sh/);
  assert.match(text, /updater_e2e_drill_full_linux_smoke_contract\.sh/);
  assert.match(text, /parity_target="\$\{GITHUB_WORKSPACE\}\/\.cargo-target\/desktop-system-parity"/);
  assert.match(
    text,
    /- name: Prepare desktop runtime bundles \(parity\)\s*\n\s*run: CTX_BUNDLE_REMOTE_DAEMONS=0 CTX_RUNTIME_PROFILE=parity pnpm -C core desktop:runtime:prepare/,
  );
  assert.match(text, /scripts\/linux_bundle_gate\.sh --platform linux-x64 --mode both/);
  assert.match(text, /CTX_RUNTIME_PROFILE=parity pnpm -C core desktop:runtime:lock:validate/);
});
