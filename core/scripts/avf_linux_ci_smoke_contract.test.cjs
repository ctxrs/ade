const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const scriptPath = path.resolve(__dirname, "avf_linux_ci_smoke.sh");
const text = fs.readFileSync(scriptPath, "utf8");

test("avf smoke resolves Cargo target roots from cargo metadata", () => {
  assert.match(text, /cargo_target_dir\(\)/);
  assert.match(text, /cargo metadata --manifest-path "\$manifest" --format-version 1 --no-deps/);
  assert.match(text, /helper_target_root="\$\{CARGO_TARGET_DIR:-\$\(cargo_target_dir "\$helper_manifest"\)\}"/);
  assert.match(text, /guest_target_root="\$\{CARGO_TARGET_DIR:-\$\(cargo_target_dir "\$\{repo_root\}\/Cargo\.toml"\)\}"/);
  assert.doesNotMatch(text, /helper_target_root="\$\{CARGO_TARGET_DIR:-\$\{repo_root\}\/apps\/desktop\/src-tauri\/target\}"/);
});
