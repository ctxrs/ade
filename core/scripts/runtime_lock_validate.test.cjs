const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { validateRuntimeLock } = require("./runtime_lock_validate.cjs");

const writeJson = (filePath, value) => {
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
};

const hostOs = process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
const hostArch = process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : process.arch;

const makeFixture = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "runtime-lock-"));
  const lockPath = path.join(dir, "runtime_lock.json");
  const manifestPath = path.join(dir, "manifest.json");
  const overridesPath = path.join(dir, "runtime_overrides.json");

  const providerHostPath = path.join(dir, "providers", "gemini-host");
  const providerLinuxPath = path.join(dir, "providers", "gemini-linux");
  const bridgeHostPath = path.join(dir, "providers", "bridge-host");
  const bridgeLinuxPath = path.join(dir, "providers", "bridge-linux");
  fs.mkdirSync(path.dirname(providerHostPath), { recursive: true });
  fs.writeFileSync(providerHostPath, "ok\n", "utf8");
  fs.writeFileSync(providerLinuxPath, "ok\n", "utf8");
  fs.writeFileSync(bridgeHostPath, "ok\n", "utf8");
  fs.writeFileSync(bridgeLinuxPath, "ok\n", "utf8");

  const runtimeHostRoot = path.join(dir, "runtimes", "node-host");
  const runtimeLinuxRoot = path.join(dir, "runtimes", "node-linux");
  const pythonHostRoot = path.join(dir, "runtimes", "python-host");
  const pythonLinuxRoot = path.join(dir, "runtimes", "python-linux");
  for (const runtimeRoot of [runtimeHostRoot, runtimeLinuxRoot, pythonHostRoot, pythonLinuxRoot]) {
    fs.mkdirSync(path.join(runtimeRoot, "bin"), { recursive: true });
  }
  fs.writeFileSync(path.join(runtimeHostRoot, "bin", "node"), "ok\n", "utf8");
  fs.writeFileSync(path.join(runtimeLinuxRoot, "bin", "node"), "ok\n", "utf8");
  fs.writeFileSync(path.join(pythonHostRoot, "bin", "python3"), "ok\n", "utf8");
  fs.writeFileSync(path.join(pythonLinuxRoot, "bin", "python3"), "ok\n", "utf8");

  const imageTar = path.join(dir, "images", "ctx-harness-linux.tar");
  fs.mkdirSync(path.dirname(imageTar), { recursive: true });
  fs.writeFileSync(imageTar, "tar\n", "utf8");

  const manifest = {
    version: 1,
    providers: [
      { id: "gemini", os: hostOs, arch: hostArch, command: providerHostPath },
      { id: "gemini", os: "linux", arch: hostArch, command: providerLinuxPath },
      { id: "acp-crp-bridge", os: hostOs, arch: hostArch, command: bridgeHostPath },
      { id: "acp-crp-bridge", os: "linux", arch: hostArch, command: bridgeLinuxPath },
    ],
    runtimes: [
      { id: "node", os: hostOs, arch: hostArch, root: runtimeHostRoot, bin: "bin/node" },
      { id: "node", os: "linux", arch: hostArch, root: runtimeLinuxRoot, bin: "bin/node" },
      { id: "python", os: hostOs, arch: hostArch, root: pythonHostRoot, bin: "bin/python3" },
      { id: "python", os: "linux", arch: hostArch, root: pythonLinuxRoot, bin: "bin/python3" },
    ],
    images: [{ id: "ctx-harness", os: "linux", arch: hostArch, tar: imageTar }],
  };

  writeJson(manifestPath, manifest);

  return {
    dir,
    lockPath,
    manifestPath,
    overridesPath,
    imageTar,
    providerHostPath,
    providerLinuxPath,
  };
};

const makeV2Sources = (sourceType) => [{ source_type: sourceType, uri: `locked://${sourceType}`, sha256: "0".repeat(64) }];

const makeV2Component = ({ kind, id, os, arch, sourceType }) => ({
  kind,
  id,
  os,
  arch,
  variant: "default",
  version: "test",
  sources: sourceType === "local" ? [{ source_type: "local", build: "desktop:prep" }] : makeV2Sources(sourceType),
});

test("runtime lock v1 validation accepts required host + linux entries", () => {
  const fixture = makeFixture();
  writeJson(fixture.lockPath, {
    version: 1,
    required: {
      provider_ids: ["acp-crp-bridge", "gemini"],
      runtime_ids: ["node", "python"],
      image_ids: ["ctx-harness"],
    },
  });

  const result = validateRuntimeLock({ lockPath: fixture.lockPath, manifestPath: fixture.manifestPath });
  assert.equal(result.ok, true);
  assert.deepEqual(result.errors, []);
});

test("runtime lock v1 validation rejects missing linux provider entry", () => {
  const fixture = makeFixture();
  const manifest = JSON.parse(fs.readFileSync(fixture.manifestPath, "utf8"));
  manifest.providers = manifest.providers.filter((entry) => !(entry.id === "gemini" && entry.os === "linux"));
  writeJson(fixture.manifestPath, manifest);

  writeJson(fixture.lockPath, {
    version: 1,
    required: {
      provider_ids: ["gemini"],
      runtime_ids: ["node"],
      image_ids: ["ctx-harness"],
    },
  });

  const result = validateRuntimeLock({ lockPath: fixture.lockPath, manifestPath: fixture.manifestPath });
  assert.equal(result.ok, false);
  assert.match(result.errors.join("\n"), /missing provider bundle entry for gemini \(container linux\//);
});

test("runtime lock v2 parity profile enforces allowed source types", () => {
  const fixture = makeFixture();

  writeJson(fixture.lockPath, {
    version: 2,
    profiles: {
      parity: { allowed_source_types: ["ci", "vendor"] },
      override: { allowed_source_types: ["ci", "vendor", "local"] },
      "source-all": { allowed_source_types: ["local"] },
    },
    required: {
      provider_ids: ["gemini", "acp-crp-bridge"],
      runtime_ids: ["node", "python"],
      image_ids: ["ctx-harness"],
    },
    components: [
      makeV2Component({ kind: "provider", id: "gemini", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "provider", id: "gemini", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "provider", id: "acp-crp-bridge", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "provider", id: "acp-crp-bridge", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "node", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "node", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "python", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "python", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "image", id: "ctx-harness", os: "linux", arch: "host", sourceType: "local" }),
    ],
  });

  const result = validateRuntimeLock({ lockPath: fixture.lockPath, manifestPath: fixture.manifestPath, profile: "parity" });
  assert.equal(result.ok, false);
  assert.match(result.errors.join("\n"), /missing allowed source for profile 'parity'/);
});

test("runtime lock v2 source-all profile accepts local sources", () => {
  const fixture = makeFixture();

  writeJson(fixture.lockPath, {
    version: 2,
    profiles: {
      parity: { allowed_source_types: ["ci", "vendor"] },
      override: { allowed_source_types: ["ci", "vendor", "local"] },
      "source-all": { allowed_source_types: ["local"] },
    },
    required: {
      provider_ids: ["gemini", "acp-crp-bridge"],
      runtime_ids: ["node", "python"],
      image_ids: ["ctx-harness"],
    },
    components: [
      makeV2Component({ kind: "provider", id: "gemini", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "provider", id: "gemini", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "provider", id: "acp-crp-bridge", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "provider", id: "acp-crp-bridge", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "node", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "node", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "python", os: "host", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "runtime", id: "python", os: "linux", arch: "host", sourceType: "local" }),
      makeV2Component({ kind: "image", id: "ctx-harness", os: "linux", arch: "host", sourceType: "local" }),
    ],
  });

  const result = validateRuntimeLock({
    lockPath: fixture.lockPath,
    manifestPath: fixture.manifestPath,
    profile: "source-all",
  });
  assert.equal(result.ok, true);
  assert.deepEqual(result.errors, []);
});

test("runtime lock v2 override profile rewrites effective manifest entries", () => {
  const fixture = makeFixture();
  const overrideProviderPath = path.join(fixture.dir, "providers", "gemini-host-override");
  fs.writeFileSync(overrideProviderPath, "override\n", "utf8");

  writeJson(fixture.lockPath, {
    version: 2,
    profiles: {
      parity: { allowed_source_types: ["ci", "vendor"] },
      override: { allowed_source_types: ["ci", "vendor", "local"] },
      "source-all": { allowed_source_types: ["local"] },
    },
    required: {
      provider_ids: ["gemini", "acp-crp-bridge"],
      runtime_ids: ["node", "python"],
      image_ids: ["ctx-harness"],
    },
    components: [
      makeV2Component({ kind: "provider", id: "gemini", os: "host", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "provider", id: "gemini", os: "linux", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "provider", id: "acp-crp-bridge", os: "host", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "provider", id: "acp-crp-bridge", os: "linux", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "runtime", id: "node", os: "host", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "runtime", id: "node", os: "linux", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "runtime", id: "python", os: "host", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "runtime", id: "python", os: "linux", arch: "host", sourceType: "ci" }),
      makeV2Component({ kind: "image", id: "ctx-harness", os: "linux", arch: "host", sourceType: "ci" }),
    ],
  });

  writeJson(fixture.overridesPath, {
    version: 1,
    overrides: [
      {
        kind: "provider",
        id: "gemini",
        os: "host",
        arch: "host",
        path: overrideProviderPath,
      },
    ],
  });

  const result = validateRuntimeLock({
    lockPath: fixture.lockPath,
    manifestPath: fixture.manifestPath,
    profile: "override",
    overridesPath: fixture.overridesPath,
  });
  assert.equal(result.ok, true);
  assert.equal(result.appliedOverrides.length, 1);

  const hostGemini = result.effectiveManifest.providers.find(
    (entry) => entry.id === "gemini" && entry.os === hostOs && entry.arch === hostArch,
  );
  assert.ok(hostGemini);
  assert.equal(hostGemini.command, overrideProviderPath);
});
