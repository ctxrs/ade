const test = require("node:test");
const assert = require("node:assert/strict");
const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const toolsBuild = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "BUILD.bazel"), "utf8");
const helperScript = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "linux_bundle_contracts.sh"), "utf8");
const releaseVerifier = fs.readFileSync(path.join(repoRoot, "scripts", "release_verify_supabase.sh"), "utf8");
const tauriConfig = JSON.parse(
  fs.readFileSync(
    path.join(repoRoot, "core", "apps", "desktop", "src-tauri", "tauri.conf.json"),
    "utf8",
  ),
);

test("linux bundle contracts expose a Bazel-owned release wrapper", () => {
  assert.match(toolsBuild, /"linux_bundle_contracts\.sh"/);
  assert.match(toolsBuild, /name = "linux_bundle_contracts"/);
  assert.match(helperScript, /BUILD_WORKSPACE_DIRECTORY/);
  assert.match(helperScript, /scripts\/linux_bundle_prune_glibc\.sh/);
  assert.match(helperScript, /scripts\/linux_bundle_gate\.sh/);
  assert.match(helperScript, /--bundles-dir "\$bundles_dir" --mode both/);
  assert.match(helperScript, /--platform "\$platform"/);
  assert.match(helperScript, /--prune-glibc/);
});

test("linux AppImage packaging includes manifest-declared ctx-mcp runtime payloads", () => {
  assert.ok(
    tauriConfig.bundle.resources.includes("bundles/runtimes/ctx-mcp/**/*"),
    "Linux AppImages must package the ctx-mcp runtime path declared in bundles/manifest.json",
  );
  assert.match(
    releaseVerifier,
    /verify_bundle_manifest_closure\.cjs/,
    "Supabase verification must reject AppImages with manifest-declared files missing from the packaged bundle",
  );
});

test("linux bundle gate requires ctx-mcp runtime for the release platform", () => {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-linux-bundle-gate-"));
  try {
    const stubBin = path.join(tempRoot, "bin");
    fs.mkdirSync(stubBin, { recursive: true });
    for (const tool of ["readelf", "ldd"]) {
      const toolPath = path.join(stubBin, tool);
      fs.writeFileSync(toolPath, "#!/bin/sh\nexit 0\n", "utf8");
      fs.chmodSync(toolPath, 0o755);
    }
    const env = {
      ...process.env,
      PATH: `${stubBin}${path.delimiter}${process.env.PATH || ""}`,
    };
    fs.writeFileSync(
      path.join(tempRoot, "manifest.json"),
      `${JSON.stringify({ version: 1, providers: [], runtimes: [], images: [], daemons: [] }, null, 2)}\n`,
      "utf8",
    );
    const missing = childProcess.spawnSync(
      "bash",
      [
        path.join(repoRoot, "scripts", "linux_bundle_gate.sh"),
        "--platform",
        "linux-x64",
        "--bundles-dir",
        tempRoot,
        "--mode",
        "closure",
      ],
      { encoding: "utf8", env },
    );
    assert.notEqual(missing.status, 0);
    assert.match(`${missing.stdout}\n${missing.stderr}`, /missing ctx-mcp runtime for linux\/x86_64/);

    const runtimeRoot = path.join(tempRoot, "runtimes", "ctx-mcp", "linux", "x86_64", "0.1.0");
    fs.mkdirSync(runtimeRoot, { recursive: true });
    const runtimeBin = path.join(runtimeRoot, "ctx-mcp");
    fs.writeFileSync(runtimeBin, "#!/bin/sh\nexit 0\n", "utf8");
    fs.chmodSync(runtimeBin, 0o755);
    fs.writeFileSync(
      path.join(tempRoot, "manifest.json"),
      `${JSON.stringify(
        {
          version: 1,
          providers: [],
          runtimes: [
            {
              id: "ctx-mcp",
              version: "0.1.0",
              os: "linux",
              arch: "x86_64",
              sha256: "0".repeat(64),
              root: "runtimes/ctx-mcp/linux/x86_64/0.1.0",
              bin: "ctx-mcp",
            },
          ],
          images: [],
          daemons: [],
        },
        null,
        2,
      )}\n`,
      "utf8",
    );
    const ok = childProcess.spawnSync(
      "bash",
      [
        path.join(repoRoot, "scripts", "linux_bundle_gate.sh"),
        "--platform",
        "linux-x64",
        "--bundles-dir",
        tempRoot,
        "--mode",
        "closure",
      ],
      { encoding: "utf8", env },
    );
    assert.equal(ok.status, 0, `${ok.stdout}\n${ok.stderr}`);
  } finally {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  }
});
