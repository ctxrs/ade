const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const scriptPath = path.join(__dirname, "build_avf_linux_guest_agent.sh");

function writeExecutable(filePath, contents) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents, { mode: 0o755 });
}

test("build_avf_linux_guest_agent.sh defaults guest helpers to the cargo metadata target root", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-guest-build-"));
  const fakeHome = path.join(tmpRoot, "home");
  const cargoBinDir = path.join(fakeHome, ".cargo", "bin");
  const targetRoot = path.join(tmpRoot, "cargo-target");
  const rustflagsReport = path.join(tmpRoot, "encoded-rustflags.txt");

  writeExecutable(
    path.join(cargoBinDir, "cargo"),
    [
      "#!/bin/sh",
      "set -eu",
      "if [ \"${1:-}\" = \"metadata\" ]; then",
      "  printf '{\"target_directory\":\"%s\"}' \"$FAKE_TARGET_ROOT\"",
      "  exit 0",
      "fi",
      "echo \"unexpected cargo invocation: $*\" >&2",
      "exit 1",
    ].join("\n"),
  );

  writeExecutable(
    path.join(cargoBinDir, "rustup"),
    [
      "#!/bin/sh",
      "set -eu",
      "if [ \"${1:-}\" = \"target\" ] && [ \"${2:-}\" = \"add\" ]; then",
      "  exit 0",
      "fi",
      "if [ \"${1:-}\" = \"run\" ]; then",
      "  shift 2",
      "  [ \"${1:-}\" = \"cargo\" ] || { echo \"expected cargo after rustup run\" >&2; exit 1; }",
      "  [ \"${2:-}\" = \"zigbuild\" ] || { echo \"expected zigbuild after rustup run cargo\" >&2; exit 1; }",
      "  printf '%s' \"${CARGO_ENCODED_RUSTFLAGS:-}\" > \"$FAKE_ENCODED_RUSTFLAGS_REPORT\"",
      "  shift 2",
      "  package=\"\"",
      "  target=\"\"",
      "  profile=\"debug\"",
      "  while [ $# -gt 0 ]; do",
      "    case \"$1\" in",
      "      -p)",
      "        package=\"$2\"",
      "        shift 2",
      "        ;;",
      "      --target)",
      "        target=\"$2\"",
      "        shift 2",
      "        ;;",
      "      --release)",
      "        profile=\"release\"",
      "        shift 1",
      "        ;;",
      "      --manifest-path)",
      "        shift 2",
      "        ;;",
      "      *)",
      "        shift 1",
      "        ;;",
      "    esac",
      "  done",
      "  [ -n \"$package\" ] || { echo \"missing package\" >&2; exit 1; }",
      "  [ -n \"$target\" ] || { echo \"missing target\" >&2; exit 1; }",
      "  mkdir -p \"$CARGO_TARGET_DIR/$target/$profile\"",
      "  printf '#!/bin/sh\\nexit 0\\n' > \"$CARGO_TARGET_DIR/$target/$profile/$package\"",
      "  chmod 755 \"$CARGO_TARGET_DIR/$target/$profile/$package\"",
      "  exit 0",
      "fi",
      "echo \"unexpected rustup invocation: $*\" >&2",
      "exit 1",
    ].join("\n"),
  );

  writeExecutable(path.join(cargoBinDir, "cargo-zigbuild"), "#!/bin/sh\nexit 0\n");
  writeExecutable(path.join(cargoBinDir, "zig"), "#!/bin/sh\nexit 0\n");

  const output = execFileSync("bash", [scriptPath, "--release"], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: fakeHome,
      PATH: `/usr/bin:/bin:/usr/sbin:/sbin`,
      FAKE_TARGET_ROOT: targetRoot,
      FAKE_ENCODED_RUSTFLAGS_REPORT: rustflagsReport,
    },
  });

  const expectedGuestAgent = path.join(targetRoot, "aarch64-unknown-linux-gnu", "release", "ctx-avf-linux-guest-agent");
  const expectedEgressProxy = path.join(targetRoot, "aarch64-unknown-linux-gnu", "release", "ctx-egress-proxy");

  assert.ok(fs.existsSync(expectedGuestAgent));
  assert.ok(fs.existsSync(expectedEgressProxy));
  assert.match(output, new RegExp(expectedGuestAgent.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  assert.match(output, new RegExp(expectedEgressProxy.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  const encodedRustflags = fs.readFileSync(rustflagsReport, "utf8");
  assert.match(encodedRustflags, new RegExp(`--remap-path-prefix=${fakeHome.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}=\\/ctx-home`));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
