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

test("build_avf_linux_guest_agent.sh defaults guest helpers to the ctx cache target root", () => {
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-avf-guest-build-"));
  const fakeHome = path.join(tmpRoot, "home");
  const cargoBinDir = path.join(fakeHome, ".cargo", "bin");
  const toolchainBinDir = path.join(fakeHome, ".rustup", "toolchains", "1.94.1", "bin");
  const volatileRoot = path.join(tmpRoot, "volatile");
  const targetRoot = path.join(volatileRoot, "targets", "ctx-monorepo", "avf-guest-test");
  const rustflagsReport = path.join(tmpRoot, "encoded-rustflags.txt");
  const pathReport = path.join(tmpRoot, "path.txt");
  const rustupReport = path.join(tmpRoot, "rustup-report.txt");
  const rustcReport = path.join(tmpRoot, "rustc-path.txt");

  writeExecutable(
    path.join(cargoBinDir, "cargo"),
    [
      "#!/bin/sh",
      "set -eu",
      "if [ \"${1:-}\" = \"metadata\" ]; then",
      "  echo \"unexpected cargo metadata invocation\" >&2",
      "  exit 1",
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
      "printf '%s\\n' \"$*\" >> \"$FAKE_RUSTUP_REPORT\"",
      "if [ \"${1:-}\" = \"toolchain\" ] && [ \"${2:-}\" = \"install\" ]; then",
      "  exit 0",
      "fi",
      "if [ \"${1:-}\" = \"target\" ] && [ \"${2:-}\" = \"add\" ]; then",
      "  exit 0",
      "fi",
      "if [ \"${1:-}\" = \"which\" ]; then",
      "  [ \"${2:-}\" = \"--toolchain\" ] || { echo \"expected --toolchain\" >&2; exit 1; }",
      "  case \"${4:-}\" in",
      "    cargo)",
      "      printf '%s\\n' \"$FAKE_TOOLCHAIN_CARGO\"",
      "      exit 0",
      "      ;;",
      "    rustc)",
      "      printf '%s\\n' \"$FAKE_TOOLCHAIN_RUSTC\"",
      "      exit 0",
      "      ;;",
      "  esac",
      "fi",
      "echo \"unexpected rustup invocation: $*\" >&2",
      "exit 1",
    ].join("\n"),
  );

  writeExecutable(
    path.join(toolchainBinDir, "cargo"),
    [
      "#!/bin/sh",
      "set -eu",
      "printf '%s' \"${CARGO_ENCODED_RUSTFLAGS:-}\" > \"$FAKE_ENCODED_RUSTFLAGS_REPORT\"",
      "printf '%s' \"${PATH:-}\" > \"$FAKE_PATH_REPORT\"",
      "printf '%s' \"${RUSTC:-}\" > \"$FAKE_RUSTC_REPORT\"",
      "[ \"${1:-}\" = \"zigbuild\" ] || { echo \"expected zigbuild\" >&2; exit 1; }",
      "shift 1",
      "package=\"\"",
      "target=\"\"",
      "profile=\"debug\"",
      "while [ $# -gt 0 ]; do",
      "  case \"$1\" in",
      "    -p)",
      "      package=\"$2\"",
      "      shift 2",
      "      ;;",
      "    --target)",
      "      target=\"$2\"",
      "      shift 2",
      "      ;;",
      "    --release)",
      "      profile=\"release\"",
      "      shift 1",
      "      ;;",
      "    --manifest-path)",
      "      shift 2",
      "      ;;",
      "    *)",
      "      shift 1",
      "      ;;",
      "  esac",
      "done",
      "[ -n \"$package\" ] || { echo \"missing package\" >&2; exit 1; }",
      "[ -n \"$target\" ] || { echo \"missing target\" >&2; exit 1; }",
      "mkdir -p \"$CARGO_TARGET_DIR/$target/$profile\"",
      "printf '#!/bin/sh\\nexit 0\\n' > \"$CARGO_TARGET_DIR/$target/$profile/$package\"",
      "chmod 755 \"$CARGO_TARGET_DIR/$target/$profile/$package\"",
    ].join("\n"),
  );

  writeExecutable(path.join(toolchainBinDir, "rustc"), "#!/bin/sh\nexit 0\n");

  writeExecutable(path.join(cargoBinDir, "cargo-zigbuild"), "#!/bin/sh\nexit 0\n");
  writeExecutable(path.join(cargoBinDir, "zig"), "#!/bin/sh\nexit 0\n");

  const output = execFileSync("bash", [scriptPath, "--release"], {
    encoding: "utf8",
    env: {
      ...process.env,
      HOME: fakeHome,
      PATH: `${path.dirname(process.execPath)}:/usr/bin:/bin:/usr/sbin:/sbin`,
      CARGO_TARGET_DIR: "",
      CTX_VOLATILE_ROOT: volatileRoot,
      CTX_CACHE_SCOPE_KEY: "avf-guest-test",
      FAKE_ENCODED_RUSTFLAGS_REPORT: rustflagsReport,
      FAKE_PATH_REPORT: pathReport,
      FAKE_RUSTUP_REPORT: rustupReport,
      FAKE_RUSTC_REPORT: rustcReport,
      FAKE_TOOLCHAIN_CARGO: path.join(toolchainBinDir, "cargo"),
      FAKE_TOOLCHAIN_RUSTC: path.join(toolchainBinDir, "rustc"),
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
  const invokedPath = fs.readFileSync(pathReport, "utf8");
  assert.ok(invokedPath.startsWith(`${toolchainBinDir}:`));
  const rustupCalls = fs.readFileSync(rustupReport, "utf8");
  assert.match(rustupCalls, /toolchain install 1\.94\.1 --profile minimal --no-self-update/);
  assert.match(rustupCalls, /target add --toolchain 1\.94\.1 aarch64-unknown-linux-gnu/);
  assert.match(rustupCalls, /which --toolchain 1\.94\.1 cargo/);
  assert.match(rustupCalls, /which --toolchain 1\.94\.1 rustc/);
  assert.equal(fs.readFileSync(rustcReport, "utf8"), path.join(toolchainBinDir, "rustc"));

  fs.rmSync(tmpRoot, { recursive: true, force: true });
});
