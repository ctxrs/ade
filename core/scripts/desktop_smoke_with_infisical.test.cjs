#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "desktop_smoke_with_infisical.sh");

function makeFakePnpm(binDir, capturePath, coreRoot) {
  const pnpmPath = path.join(binDir, "pnpm");
  const script = `#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ge 5 && "$1" == "-C" && "$2" == "apps/web" && "$3" == "exec" && "$4" == "which" && "$5" == "vite" ]]; then
  exit 0
fi

if [[ "$#" -ge 5 && "$1" == "-C" && "$2" == "apps/desktop" && "$3" == "exec" && "$4" == "which" && "$5" == "wdio" ]]; then
  exit 0
fi

if [[ "$#" -ge 4 && "$1" == "-C" && "$2" == "apps/desktop" && "$3" == "exec" && "$4" == "wdio" ]]; then
  printf '%s' "\${TMPDIR:-}" > "${capturePath}"
  exit 0
fi

if [[ "$#" -ge 1 && "$1" == "install" ]]; then
  mkdir -p "${coreRoot}/node_modules"
  exit 0
fi

if [[ "$#" -ge 3 && "$2" == "install" ]]; then
  mkdir -p "${coreRoot}/$1/node_modules"
  exit 0
fi

exit 0
`;
  fs.writeFileSync(pnpmPath, script, { encoding: "utf8", mode: 0o755 });
  return pnpmPath;
}

function runDesktopSmoke(envOverrides = {}) {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-smoke-test-"));
  const binDir = path.join(tempRoot, "bin");
  const tmpBaseDir = path.join(tempRoot, "tmp-base");
  const capturePath = path.join(tempRoot, "captured-tmpdir.txt");
  fs.mkdirSync(binDir, { recursive: true });
  fs.mkdirSync(tmpBaseDir, { recursive: true });
  makeFakePnpm(binDir, capturePath, path.join(repoRoot, "core"));

  const result = spawnSync("bash", [scriptPath], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${binDir}:${process.env.PATH || ""}`,
      CN_API_KEY: "test-cn-key",
      CTX_AUTOMATION_TMP_BASE_DIR: tmpBaseDir,
      CARGO_TARGET_DIR: path.join(tempRoot, "cargo-target"),
      ...envOverrides,
    },
  });

  const capturedTmpdir = fs.existsSync(capturePath)
    ? fs.readFileSync(capturePath, "utf8").trim()
    : "";
  return { result, tempRoot, tmpBaseDir, capturedTmpdir };
}

test("desktop smoke removes auto-created tmp dirs by default", () => {
  const { result, tmpBaseDir, capturedTmpdir } = runDesktopSmoke();

  assert.equal(result.status, 0, `script should succeed: ${result.stderr || result.stdout}`);
  assert.ok(capturedTmpdir, "expected fake wdio run to capture TMPDIR");
  assert.equal(
    fs.existsSync(capturedTmpdir),
    false,
    "auto-created automation tmpdir should be removed after the run",
  );
  const remainingEntries = fs
    .readdirSync(tmpBaseDir)
    .filter((entry) => entry.startsWith("ctx-desktop-e2e-tmp."));
  assert.deepEqual(
    remainingEntries,
    [],
    "tmp base dir should not keep leaked ctx-desktop-e2e temp dirs",
  );
});

test("desktop smoke preserves auto-created tmp dirs when explicitly requested", () => {
  const { result, capturedTmpdir } = runDesktopSmoke({
    CTX_AUTOMATION_KEEP_TMPDIR: "1",
  });

  assert.equal(result.status, 0, `script should succeed: ${result.stderr || result.stdout}`);
  assert.ok(capturedTmpdir, "expected fake wdio run to capture TMPDIR");
  assert.equal(
    fs.existsSync(capturedTmpdir),
    true,
    "CTX_AUTOMATION_KEEP_TMPDIR=1 should preserve the auto-created tmpdir",
  );
});
