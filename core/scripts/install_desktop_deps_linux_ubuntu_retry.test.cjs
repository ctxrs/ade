const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "install_desktop_deps_linux_ubuntu.sh");
const testTmpRoot = "/tmp";

function shellQuote(value) {
  return `'${String(value).replace(/'/g, `'\\''`)}'`;
}

function createFixture() {
  const root = fs.mkdtempSync(path.join(testTmpRoot, "ctx-install-desktop-deps-"));
  const stateDir = path.join(root, "state");
  fs.mkdirSync(stateDir, { recursive: true });
  return { stateDir };
}

test("install_desktop_deps_linux_ubuntu retries transient apt/dpkg lock contention", () => {
  const fixture = createFixture();
  const availablePackages = [
    "build-essential",
    "binutils",
    "pkg-config",
    "curl",
    "sshpass",
    "file",
    "xdg-utils",
    "xauth",
    "xvfb",
    "desktop-file-utils",
    "squashfs-tools",
    "zsync",
    "patchelf",
    "appstream",
    "libgtk-3-bin",
    "libglib2.0-dev",
    "libgtk-3-dev",
    "librsvg2-dev",
    "libssl-dev",
    "libcap-dev",
    "liblzma-dev",
    "musl",
    "libc++1",
    "tk",
    "unzip",
    "libwebkit2gtk-4.1-dev",
    "libsoup-3.0-dev",
    "webkit2gtk-driver",
    "libayatana-appindicator3-dev",
    "libfuse2t64",
  ].join(" ");

  const bashCommand = `
set -euo pipefail
sudo() {
  if [[ "$#" -gt 0 && "$1" == "-n" ]]; then
    shift
  fi
  "$@"
}
env() {
  while [[ "$#" -gt 0 && "$1" == *=* ]]; do
    export "$1"
    shift
  done
  "$@"
}
apt-get() {
  local state_dir="\${CTX_TEST_STATE_DIR:?}"
  local cmd="$1"
  case "$cmd" in
    update)
      local attempts=0
      if [[ -f "$state_dir/update-attempts" ]]; then
        attempts="$(<"$state_dir/update-attempts")"
      fi
      attempts=$((attempts + 1))
      printf '%s' "$attempts" >"$state_dir/update-attempts"
      if (( attempts <= \${CTX_TEST_LOCK_FAILURES:-0} )); then
        echo 'E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 1234 (unattended-upgr)' >&2
        echo 'E: Unable to acquire the dpkg frontend lock (/var/lib/dpkg/lock-frontend), is another process using it?' >&2
        return 100
      fi
      echo 'update ok'
      ;;
    install)
      printf '%s\\n' "$*" >"$state_dir/install-args.txt"
      echo 'install ok'
      ;;
    *)
      echo "unexpected apt-get invocation: $*" >&2
      return 1
      ;;
  esac
}
pkg-config() { return 0; }
xdg-mime() { return 0; }
xauth() { return 0; }
xvfb-run() { return 0; }
desktop-file-validate() { return 0; }
mksquashfs() { return 0; }
zsyncmake() { return 0; }
patchelf() { return 0; }
appstreamcli() { return 0; }
gtk-update-icon-cache() { return 0; }
WebKitWebDriver() { return 0; }
ldconfig() {
  if [[ "\${1:-}" == "-p" ]]; then
    printf 'libfuse.so.2 (libc6,x86-64) => /usr/lib/libfuse.so.2\\n'
  fi
}
docker() {
  if [[ "\${1:-}" == "buildx" && "\${2:-}" == "version" ]]; then
    printf 'github.com/docker/buildx v0.30.1\\n'
    return 0
  fi
  return 0
}
export -f sudo env apt-get pkg-config xdg-mime xauth xvfb-run desktop-file-validate mksquashfs zsyncmake patchelf appstreamcli gtk-update-icon-cache WebKitWebDriver ldconfig docker
bash ${shellQuote(scriptPath)}
`;

  const result = spawnSync("bash", ["-lc", bashCommand], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      CI: "1",
      CTX_TEST_STATE_DIR: fixture.stateDir,
      CTX_TEST_LOCK_FAILURES: "1",
      CTX_APT_LOCK_RETRY_ATTEMPTS: "3",
      CTX_APT_LOCK_RETRY_SLEEP_SECONDS: "0",
      CTX_TEST_APT_CACHE_AVAILABLE_PACKAGES: availablePackages,
    },
  });

  assert.equal(
    result.status,
    0,
    `stdout:\n${result.stdout}\n\nstderr:\n${result.stderr}`,
  );
  assert.match(result.stderr, /apt\/dpkg lock is busy/);
  assert.doesNotMatch(result.stdout, /Next steps/);
  assert.equal(fs.readFileSync(path.join(fixture.stateDir, "update-attempts"), "utf8"), "2");
  const installArgs = fs.readFileSync(path.join(fixture.stateDir, "install-args.txt"), "utf8");
  assert.match(installArgs, /\bbinutils\b/);
  assert.match(installArgs, /\bliblzma-dev\b/);
  assert.match(installArgs, /\bunzip\b/);
});
