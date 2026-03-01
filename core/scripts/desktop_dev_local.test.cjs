const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  shouldEnableSystemPodmanFallback,
} = require("./desktop_dev_local.cjs");

const writeJson = (filePath, value) => {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const makeManifestPath = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "desktop-dev-local-"));
  return path.join(dir, "runtime_manifest.effective.json");
};

test("enables system podman fallback when CTX_BUNDLE_PODMAN is explicitly disabled", () => {
  const manifestPath = makeManifestPath();
  writeJson(manifestPath, { version: 1, runtimes: [] });

  const enabled = shouldEnableSystemPodmanFallback({
    bundlePodmanEnv: "0",
    allowSystemPodmanEnv: "",
    effectiveManifestPath: manifestPath,
  });
  assert.equal(enabled, true);
});

test("enables system podman fallback when podman runtime is not bundled in effective manifest", () => {
  const manifestPath = makeManifestPath();
  writeJson(manifestPath, {
    version: 1,
    runtimes: [
      { id: "node", os: "macos", arch: "aarch64", root: "runtimes/node", bin: "bin/node" },
    ],
  });

  const enabled = shouldEnableSystemPodmanFallback({
    bundlePodmanEnv: "",
    allowSystemPodmanEnv: "",
    effectiveManifestPath: manifestPath,
    platform: "darwin",
    arch: "arm64",
  });
  assert.equal(enabled, true);
});

test("does not force fallback when podman runtime is bundled for host target", () => {
  const manifestPath = makeManifestPath();
  writeJson(manifestPath, {
    version: 1,
    runtimes: [
      { id: "podman", os: "macos", arch: "aarch64", root: "runtimes/podman", bin: "usr/bin/podman" },
    ],
  });

  const enabled = shouldEnableSystemPodmanFallback({
    bundlePodmanEnv: "",
    allowSystemPodmanEnv: "",
    effectiveManifestPath: manifestPath,
    platform: "darwin",
    arch: "arm64",
  });
  assert.equal(enabled, false);
});

test("does not override explicit CTX_ALLOW_SYSTEM_PODMAN value", () => {
  const manifestPath = makeManifestPath();
  writeJson(manifestPath, { version: 1, runtimes: [] });

  const enabled = shouldEnableSystemPodmanFallback({
    bundlePodmanEnv: "",
    allowSystemPodmanEnv: "0",
    effectiveManifestPath: manifestPath,
  });
  assert.equal(enabled, false);
});
