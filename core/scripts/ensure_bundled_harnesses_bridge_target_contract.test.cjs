const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptText = fs.readFileSync(
  path.join(repoRoot, "scripts", "ensure_bundled_harnesses.sh"),
  "utf8",
);

test("ensure_bundled_harnesses treats explicit cargo targets as target-specific bridge output", () => {
  assert.match(scriptText, /should_prefer_native_cargo_layout\(\)/);
  assert.match(
    scriptText,
    /local configured_target="\$\{CARGO_BUILD_TARGET:-\$\{TAURI_ENV_TARGET_TRIPLE:-\}\}"/,
  );
  assert.match(
    scriptText,
    /if \[\[ -n "\$configured_target" \]\]; then\s+return 1\s+fi/,
  );
  assert.match(
    scriptText,
    /printf '%s' "\$CARGO_TARGET_DIR\/\$rust_target\/release\/\$\{BRIDGE_BIN\}\$\{BIN_EXT\}"/,
  );
});
