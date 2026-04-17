const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const scriptPath = path.resolve(__dirname, "..", "..", "scripts", "install_desktop_deps_linux_ubuntu.sh");
const script = fs.readFileSync(scriptPath, "utf8");

test("ubuntu desktop deps installer includes libcap for codex sandbox builds", () => {
  assert.match(
    script,
    /\n\s*binutils\n[\s\S]*\n\s*liblzma-dev\n[\s\S]*\n\s*unzip\n/,
    "install_desktop_deps_linux_ubuntu.sh must include the Linux release packaging utilities in the shared desktop deps set",
  );
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
  assert.match(
    script,
    /CTX_BUILDKITE_APT_FORCE_IPV4/,
    "install_desktop_deps_linux_ubuntu.sh must support forcing apt over IPv4 for IPv6-impaired hosts",
  );
  assert.match(
    script,
    /Acquire::ForceIPv4 "true";/,
    "install_desktop_deps_linux_ubuntu.sh must write an apt IPv4 override when requested",
  );
  assert.match(
    script,
    /run_apt_get_with_lock_retry\(\)/,
    "install_desktop_deps_linux_ubuntu.sh must centralize apt invocations behind a lock-aware retry helper",
  );
  assert.match(
    script,
    /Could not get lock\|Unable to acquire the dpkg frontend lock/,
    "install_desktop_deps_linux_ubuntu.sh must recognize transient apt\/dpkg lock contention",
  );
  assert.doesNotMatch(
    script,
    /docker buildx version 2>\/dev\/null \| grep -Fq/,
    "install_desktop_deps_linux_ubuntu.sh must not rely on a pipefail-sensitive docker buildx grep pipeline",
  );
  assert.doesNotMatch(
    script,
    /ldconfig -p \| grep -q/,
    "install_desktop_deps_linux_ubuntu.sh must not rely on a pipefail-sensitive ldconfig grep pipeline",
  );
  assert.match(
    script,
    /if \[\[ -t 1 && -z "\$\{CI:-\}" && -z "\$\{BUILDKITE:-\}" \]\]; then[\s\S]*Next steps/,
    "install_desktop_deps_linux_ubuntu.sh must only print the interactive next-steps banner outside CI and Buildkite",
  );
});
