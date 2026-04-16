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
  assert.match(
    script,
    /DOCKER_BUILDX_VERSION/,
    "install_desktop_deps_linux_ubuntu.sh must pin a Docker buildx version for BuildBuddy release bundle contracts",
  );
  assert.match(
    script,
    /docker buildx version/,
    "install_desktop_deps_linux_ubuntu.sh must sanity-check docker buildx availability",
  );
  assert.match(
    script,
    /\n\s*xauth\n\s*xvfb\n/,
    "install_desktop_deps_linux_ubuntu.sh must install xauth and xvfb for Linux desktop automation",
  );
  assert.match(
    script,
    /for cmd in xauth xvfb-run;/,
    "install_desktop_deps_linux_ubuntu.sh must sanity-check xauth and xvfb-run availability",
  );
});
