const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const scriptPath = path.resolve(__dirname, "..", "..", "scripts", "install_desktop_deps_linux_ubuntu.sh");
const script = fs.readFileSync(scriptPath, "utf8");

test("ubuntu desktop deps installer includes libcap for codex sandbox builds", () => {
  assert.match(
    script,
    /\n\s*libcap-dev\n/,
    "install_desktop_deps_linux_ubuntu.sh must install libcap-dev for codex-linux-sandbox",
  );
  assert.match(
    script,
    /for pc in [^\n]*\blibcap\b/,
    "install_desktop_deps_linux_ubuntu.sh must sanity-check libcap via pkg-config",
  );
});
