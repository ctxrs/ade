const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const http = require("node:http");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const scriptPath = path.join(repoRoot, "core/apps/desktop/scripts/local_release_fixture.cjs");
const desktopVersion = JSON.parse(
  fs.readFileSync(path.join(repoRoot, "core/apps/desktop/package.json"), "utf8"),
).version;

const httpGet = (url) => new Promise((resolve, reject) => {
  const req = http.get(url, (res) => {
    const chunks = [];
    res.on("data", (chunk) => chunks.push(chunk));
    res.on("end", () => {
      resolve({
        statusCode: res.statusCode,
        body: Buffer.concat(chunks),
      });
    });
  });
  req.on("error", reject);
});

const parseExport = (stdout, name) => {
  const match = String(stdout || "").match(new RegExp(`export ${name}='([^']+)'`));
  return match ? match[1] : "";
};

const updaterTarget = () => {
  const osPart = process.platform === "darwin" ? "macos" : process.platform === "win32" ? "windows" : "linux";
  const archPart = process.arch === "arm64" ? "arm64" : process.arch === "x64" ? "x64" : process.arch;
  return `${osPart}-${archPart}`;
};

const writeDaemon = (dir, fileName, body) => {
  const filePath = path.join(dir, fileName);
  fs.writeFileSync(filePath, body);
  fs.chmodSync(filePath, 0o755);
};

test("local release fixture serves a managed daemon manifest and daemon bytes", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-release-fixture-"));
  const bundlesDir = path.join(tmp, "bundles");
  const stateFile = path.join(tmp, "fixture.state.json");
  fs.mkdirSync(bundlesDir, { recursive: true });
  writeDaemon(bundlesDir, "ctx-daemon-linux-x86_64", "linux-x64-daemon");
  writeDaemon(bundlesDir, "ctx-daemon-linux-aarch64", "linux-arm64-daemon");

  let baseUrl = "";
  try {
    const start = spawnSync("node", [scriptPath, "start", "--state-file", stateFile, "--bundle-dir", bundlesDir], {
      cwd: repoRoot,
      encoding: "utf8",
    });
    assert.equal(start.status, 0, start.stderr);
    baseUrl = parseExport(start.stdout, "CTX_DOWNLOAD_BASE_URL");
    const expectedManagedVersion = parseExport(
      start.stdout,
      "CTX_AUTOMATION_REMOTE_EXPECTED_MANAGED_VERSION",
    );
    const manifestPubkey = parseExport(start.stdout, "CTX_RELEASE_MANIFEST_PUBKEY");
    assert.match(baseUrl, /^http:\/\/127\.0\.0\.1:\d+\/functions\/v1$/);
    assert.equal(expectedManagedVersion, "automation-local");
    assert.match(manifestPubkey, /^[A-Za-z0-9+/=]+$/);

    const manifestResp = await httpGet(`${baseUrl}/releases/stable/latest.json`);
    assert.equal(manifestResp.statusCode, 200);
    const manifest = JSON.parse(manifestResp.body.toString("utf8"));
    assert.equal(manifest.channel, "stable");
    assert.equal(manifest.latest_version, expectedManagedVersion);
    assert.equal(
      manifest.platforms["linux-arm64"].daemon.url_path,
      "/releases/stable/linux-arm64/ctx",
    );
    const manifestSigResp = await httpGet(`${baseUrl}/releases/stable/latest.json.sig`);
    assert.equal(manifestSigResp.statusCode, 200);
    assert.match(manifestSigResp.body.toString("utf8").trim(), /^[A-Za-z0-9+/=]+$/);

    const tauriManifestResp = await httpGet(`${baseUrl}/releases/stable/latest-tauri.json`);
    assert.equal(tauriManifestResp.statusCode, 200);
    const tauriManifest = JSON.parse(tauriManifestResp.body.toString("utf8"));
    assert.equal(tauriManifest.version, desktopVersion);
    const expectedTarget = updaterTarget();
    assert.equal(
      tauriManifest.platforms[expectedTarget].url,
      `${baseUrl}/download/stable/${desktopVersion}/ctx_${desktopVersion}_${expectedTarget}_updater.app.tar.gz`,
    );
    assert.equal(tauriManifest.platforms[expectedTarget].signature, "sig");

    const daemonResp = await httpGet(`${baseUrl}/releases/stable/linux-arm64/ctx`);
    assert.equal(daemonResp.statusCode, 200);
    assert.equal(daemonResp.body.toString("utf8"), "linux-arm64-daemon");

    const expectedVersion = spawnSync(
      "node",
      [scriptPath, "print-expected-version", "--state-file", stateFile],
      {
        cwd: repoRoot,
        encoding: "utf8",
      },
    );
    assert.equal(expectedVersion.status, 0, expectedVersion.stderr);
    assert.equal(expectedVersion.stdout.trim(), expectedManagedVersion);

    const x64Artifact = spawnSync(
      "node",
      [scriptPath, "print-artifact", "--state-file", stateFile, "--arch", "x86_64"],
      {
        cwd: repoRoot,
        encoding: "utf8",
      },
    );
    assert.equal(x64Artifact.status, 0, x64Artifact.stderr);
    assert.equal(x64Artifact.stdout.trim(), path.join(bundlesDir, "ctx-daemon-linux-x86_64"));

    const arm64Artifact = spawnSync(
      "node",
      [scriptPath, "print-artifact", "--state-file", stateFile, "--arch", "arm64"],
      {
        cwd: repoRoot,
        encoding: "utf8",
      },
    );
    assert.equal(arm64Artifact.status, 0, arm64Artifact.stderr);
    assert.equal(arm64Artifact.stdout.trim(), path.join(bundlesDir, "ctx-daemon-linux-aarch64"));
  } finally {
    spawnSync("node", [scriptPath, "stop", "--state-file", stateFile], {
      cwd: repoRoot,
      encoding: "utf8",
    });
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("local release fixture fails fast when no bundled daemon artifacts are available", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-release-fixture-empty-"));
  const bundlesDir = path.join(tmp, "bundles");
  const stateFile = path.join(tmp, "fixture.state.json");
  fs.mkdirSync(bundlesDir, { recursive: true });

  try {
    const start = spawnSync("node", [scriptPath, "start", "--state-file", stateFile, "--bundle-dir", bundlesDir], {
      cwd: repoRoot,
      encoding: "utf8",
    });
    assert.notEqual(start.status, 0);
    assert.match(start.stderr, /run 'pnpm -C core desktop:prep:release' or set CTX_DOWNLOAD_BASE_URL/);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});

test("local release fixture rejects unsupported preseed architectures", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-local-release-fixture-arch-"));
  const bundlesDir = path.join(tmp, "bundles");
  const stateFile = path.join(tmp, "fixture.state.json");
  fs.mkdirSync(bundlesDir, { recursive: true });
  writeDaemon(bundlesDir, "ctx-daemon-linux-x86_64", "linux-x64-daemon");

  try {
    const start = spawnSync("node", [scriptPath, "start", "--state-file", stateFile, "--bundle-dir", bundlesDir], {
      cwd: repoRoot,
      encoding: "utf8",
    });
    assert.equal(start.status, 0, start.stderr);

    const artifact = spawnSync(
      "node",
      [scriptPath, "print-artifact", "--state-file", stateFile, "--arch", "riscv64"],
      {
        cwd: repoRoot,
        encoding: "utf8",
      },
    );
    assert.notEqual(artifact.status, 0);
    assert.match(artifact.stderr, /unsupported remote fixture architecture/);
  } finally {
    spawnSync("node", [scriptPath, "stop", "--state-file", stateFile], {
      cwd: repoRoot,
      encoding: "utf8",
    });
    fs.rmSync(tmp, { recursive: true, force: true });
  }
});
