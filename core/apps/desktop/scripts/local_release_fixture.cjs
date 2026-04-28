#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");
const crypto = require("node:crypto");
const { spawn } = require("node:child_process");

const REPO_ROOT = path.resolve(__dirname, "..", "..", "..", "..");
const DEFAULT_BUNDLE_DIR = path.join(
  REPO_ROOT,
  "core",
  "apps",
  "desktop",
  "src-tauri",
  "bundles",
  "daemons",
);
const DESKTOP_PACKAGE_JSON = path.join(REPO_ROOT, "core", "apps", "desktop", "package.json");
const DEFAULT_CHANNEL = "stable";
const DEFAULT_HOST = "127.0.0.1";
const START_TIMEOUT_MS = 10000;
const DAEMON_TARGETS = Object.freeze([
  {
    platformKey: "linux-x64",
    fileName: "ctx-daemon-linux-x86_64",
  },
  {
    platformKey: "linux-arm64",
    fileName: "ctx-daemon-linux-aarch64",
  },
]);

const shellQuote = (value) => `'${String(value).replace(/'/g, `'\\''`)}'`;

const sleepSync = (durationMs) => {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, durationMs);
};

const sha256File = (filePath) => {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
};

const b64UrlToBuffer = (value) => {
  const normalized = String(value).replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
  return Buffer.from(padded, "base64");
};

const generateManifestSignatureState = (manifestBody) => {
  const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
  const publicJwk = publicKey.export({ format: "jwk" });
  const rawPublicKey = b64UrlToBuffer(publicJwk.x);
  const keyId = Buffer.from("ctxsig01", "utf8");
  const pubkeyText =
    "untrusted comment: minisign public key: TESTKEY0\n" +
    `${Buffer.concat([Buffer.from([0x45, 0x64]), keyId, rawPublicKey]).toString("base64")}\n`;
  const trustedComment = "trusted comment: timestamp:1772585616\tfile:latest.json";
  const signatureBytes = crypto.sign(
    null,
    crypto.createHash("blake2b512").update(Buffer.from(manifestBody, "utf8")).digest(),
    privateKey,
  );
  const globalSignature = crypto.sign(
    null,
    Buffer.concat([
      signatureBytes,
      Buffer.from(trustedComment.slice("trusted comment: ".length), "utf8"),
    ]),
    privateKey,
  );
  const signatureText =
    "untrusted comment: signature from minisign secret key\n" +
    `${Buffer.concat([Buffer.from([0x45, 0x44]), keyId, signatureBytes]).toString("base64")}\n` +
    `${trustedComment}\n` +
    `${globalSignature.toString("base64")}\n`;
  return {
    manifestPubkeyB64: Buffer.from(pubkeyText, "utf8").toString("base64"),
    manifestSignatureB64: Buffer.from(signatureText, "utf8").toString("base64"),
  };
};

const discoverDaemonArtifacts = (bundleDir) => {
  const artifacts = {};
  for (const target of DAEMON_TARGETS) {
    const filePath = path.join(bundleDir, target.fileName);
    if (!fs.existsSync(filePath)) {
      continue;
    }
    artifacts[target.platformKey] = {
      filePath,
      sha256: sha256File(filePath),
    };
  }
  if (Object.keys(artifacts).length === 0) {
    throw new Error(
      `no bundled remote daemon artifacts found in ${bundleDir}; run 'pnpm -C core desktop:prep:release' or set CTX_DOWNLOAD_BASE_URL`,
    );
  }
  return artifacts;
};

const platformKeyForArch = (arch) => {
  const normalized = String(arch || "").trim().toLowerCase();
  if (normalized === "x86_64" || normalized === "amd64") {
    return "linux-x64";
  }
  if (normalized === "aarch64" || normalized === "arm64") {
    return "linux-arm64";
  }
  throw new Error(`unsupported remote fixture architecture: ${normalized || "<missing>"}`);
};

const resolveExpectedManagedVersion = (state) =>
  String(state?.expectedManagedVersion || state?.releaseManifest?.latest_version || "").trim();

const resolveArtifactForArch = (state, arch) => {
  const platformKey = platformKeyForArch(arch);
  const artifact = state?.artifacts?.[platformKey];
  if (!artifact || !artifact.filePath) {
    throw new Error(`missing ${platformKey} daemon artifact in fixture state`);
  }
  return {
    platformKey,
    filePath: String(artifact.filePath),
    sha256: String(artifact.sha256 || "").trim(),
  };
};

const buildManifest = ({ channel, artifacts }) => {
  const platforms = {};
  for (const [platformKey, artifact] of Object.entries(artifacts)) {
    platforms[platformKey] = {
      daemon: {
        url_path: `/releases/${channel}/${platformKey}/ctx`,
        sha256: artifact.sha256,
      },
    };
  }
  return {
    channel,
    latest_version: "automation-local",
    min_supported_version: "0.0.0",
    published_at: new Date().toISOString(),
    platforms,
  };
};

const readDesktopVersion = () => {
  const pkg = JSON.parse(fs.readFileSync(DESKTOP_PACKAGE_JSON, "utf8"));
  return String(pkg.version || "").trim() || "0.0.0";
};

const desktopUpdaterTarget = () => {
  const osPart = process.platform === "darwin"
    ? "macos"
    : process.platform === "win32"
      ? "windows"
      : "linux";
  const archPart = process.arch === "arm64"
    ? "arm64"
    : process.arch === "x64"
      ? "x64"
      : process.arch;
  return `${osPart}-${archPart}`;
};

const buildTauriManifest = ({ baseUrl, channel, version, target }) => {
  const artifactName = `ctx_${version}_${target}_updater.app.tar.gz`;
  return {
    version,
    notes: `ctx ${version}`,
    pub_date: new Date().toISOString(),
    platforms: {
      [target]: {
        url: `${baseUrl}/functions/v1/download/${channel}/${version}/${artifactName}`,
        signature: "sig",
      },
    },
  };
};

const parseArgs = (argv) => {
  const [command = "", ...rest] = argv;
  const args = {
    command,
    stateFile: "",
    bundleDir: DEFAULT_BUNDLE_DIR,
    channel: DEFAULT_CHANNEL,
    host: DEFAULT_HOST,
    port: 0,
    arch: "",
  };
  for (let i = 0; i < rest.length; i += 1) {
    const token = rest[i];
    switch (token) {
      case "--state-file":
        args.stateFile = String(rest[i + 1] || "");
        i += 1;
        break;
      case "--bundle-dir":
        args.bundleDir = String(rest[i + 1] || "");
        i += 1;
        break;
      case "--channel":
        args.channel = String(rest[i + 1] || "");
        i += 1;
        break;
      case "--host":
        args.host = String(rest[i + 1] || "");
        i += 1;
        break;
      case "--port":
        args.port = Number.parseInt(String(rest[i + 1] || "0"), 10) || 0;
        i += 1;
        break;
      case "--arch":
        args.arch = String(rest[i + 1] || "");
        i += 1;
        break;
      default:
        throw new Error(`unknown argument: ${token}`);
    }
  }
  if (!args.stateFile.trim()) {
    throw new Error("--state-file is required");
  }
  args.stateFile = path.resolve(args.stateFile);
  args.bundleDir = path.resolve(args.bundleDir || DEFAULT_BUNDLE_DIR);
  args.channel = args.channel.trim() || DEFAULT_CHANNEL;
  args.host = args.host.trim() || DEFAULT_HOST;
  return args;
};

const readState = (stateFile) => JSON.parse(fs.readFileSync(stateFile, "utf8"));

const writeState = (stateFile, state) => {
  fs.mkdirSync(path.dirname(stateFile), { recursive: true });
  fs.writeFileSync(stateFile, `${JSON.stringify(state, null, 2)}\n`, "utf8");
};

const artifactPathForRequest = (state, pathname) => {
  const prefix = `/functions/v1/releases/${state.channel}/`;
  if (!pathname.startsWith(prefix)) {
    const downloadPrefix = `/functions/v1/download/${state.channel}/${state.desktopVersion}/`;
    if (!pathname.startsWith(downloadPrefix)) {
      return null;
    }
    return { type: "updater-artifact" };
  }
  const suffix = pathname.slice(prefix.length);
  if (suffix === "latest.json") {
    return { type: "manifest" };
  }
  if (suffix === "latest.json.sig") {
    return { type: "manifest-signature" };
  }
  if (suffix === "latest-tauri.json") {
    return { type: "tauri-manifest" };
  }
  const [platformKey, fileName, ...rest] = suffix.split("/");
  if (rest.length > 0 || fileName !== "ctx") {
    return null;
  }
  const artifact = state.artifacts[platformKey];
  if (!artifact) {
    return null;
  }
  return {
    type: "artifact",
    filePath: artifact.filePath,
  };
};

const serve = (stateFile) => {
  let state = readState(stateFile);
  const server = http.createServer((req, res) => {
    const pathname = new URL(req.url || "/", `http://${req.headers.host || state.host}`).pathname;
    const match = artifactPathForRequest(state, pathname);
    if (!match) {
      res.statusCode = 404;
      res.end("not found");
      return;
    }
    if (match.type === "manifest") {
      res.setHeader("content-type", "application/json");
      res.end(state.releaseManifestBody);
      return;
    }
    if (match.type === "manifest-signature") {
      res.setHeader("content-type", "text/plain");
      res.end(`${state.releaseManifestSignatureB64}\n`);
      return;
    }
    if (match.type === "tauri-manifest") {
      res.setHeader("content-type", "application/json");
      res.end(`${JSON.stringify(state.tauriManifest, null, 2)}\n`);
      return;
    }
    if (match.type === "updater-artifact") {
      res.setHeader("content-type", "application/octet-stream");
      res.end("ctx-updater-placeholder");
      return;
    }
    res.setHeader("content-type", "application/octet-stream");
    fs.createReadStream(match.filePath).pipe(res);
  });

  const shutdown = () => {
    server.close(() => process.exit(0));
  };

  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);

  server.listen(state.port, state.host, () => {
    const address = server.address();
    const port = typeof address === "object" && address ? address.port : state.port;
    const baseUrl = `http://${state.host}:${port}`;
    state = {
      ...state,
      pid: process.pid,
      port,
      baseUrl,
      tauriManifest: buildTauriManifest({
        baseUrl,
        channel: state.channel,
        version: state.desktopVersion,
        target: state.updaterTarget,
      }),
      ready: true,
    };
    writeState(stateFile, state);
  });
};

const start = (args) => {
  const artifacts = discoverDaemonArtifacts(args.bundleDir);
  const releaseManifest = buildManifest({
    channel: args.channel,
    artifacts,
  });
  const releaseManifestBody = `${JSON.stringify(releaseManifest)}\n`;
  const manifestSignatureState = generateManifestSignatureState(releaseManifestBody);
  const desktopVersion = readDesktopVersion();
  const updaterTarget = desktopUpdaterTarget();
  const baseUrl = `http://${args.host}:${args.port || 0}`;
  writeState(args.stateFile, {
    host: args.host,
    port: args.port,
    channel: args.channel,
    bundleDir: args.bundleDir,
    artifacts,
    desktopVersion,
    updaterTarget,
    expectedManagedVersion: releaseManifest.latest_version,
    releaseManifest,
    releaseManifestBody,
    releaseManifestSignatureB64: manifestSignatureState.manifestSignatureB64,
    releaseManifestPubkeyB64: manifestSignatureState.manifestPubkeyB64,
    tauriManifest: buildTauriManifest({
      baseUrl,
      channel: args.channel,
      version: desktopVersion,
      target: updaterTarget,
    }),
    ready: false,
  });

  const child = spawn(process.execPath, [__filename, "serve", "--state-file", args.stateFile], {
    detached: true,
    stdio: "ignore",
  });
  child.unref();

  const deadline = Date.now() + START_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const state = readState(args.stateFile);
      if (state.ready && state.baseUrl) {
        process.stdout.write(`export CTX_DOWNLOAD_BASE_URL=${shellQuote(`${state.baseUrl}/functions/v1`)}\n`);
        process.stdout.write(
          `export CTX_RELEASE_MANIFEST_PUBKEY=${shellQuote(state.releaseManifestPubkeyB64)}\n`,
        );
        process.stdout.write(
          `export CTX_AUTOMATION_REMOTE_EXPECTED_MANAGED_VERSION=${shellQuote(resolveExpectedManagedVersion(state))}\n`,
        );
        process.stdout.write(
          `export CTX_AUTOMATION_REMOTE_RELEASE_FIXTURE_STATE_FILE=${shellQuote(args.stateFile)}\n`,
        );
        return;
      }
    } catch (error) {
      if (error.code !== "ENOENT") {
        throw error;
      }
    }
    sleepSync(100);
  }

  try {
    process.kill(child.pid, "SIGTERM");
  } catch (_error) {
    // no-op
  }
  throw new Error(`timed out waiting for local release fixture to start: ${args.stateFile}`);
};

const stop = (args) => {
  let state = null;
  try {
    state = readState(args.stateFile);
  } catch (error) {
    if (error.code === "ENOENT") {
      return;
    }
    throw error;
  }
  if (state && Number.isInteger(state.pid) && state.pid > 0) {
    try {
      process.kill(state.pid, "SIGTERM");
    } catch (error) {
      if (error.code !== "ESRCH") {
        throw error;
      }
    }
  }
};

const printArtifact = (args) => {
  const state = readState(args.stateFile);
  const artifact = resolveArtifactForArch(state, args.arch);
  process.stdout.write(`${artifact.filePath}\n`);
};

const printExpectedVersion = (args) => {
  const state = readState(args.stateFile);
  process.stdout.write(`${resolveExpectedManagedVersion(state)}\n`);
};

if (require.main === module) {
  try {
    const args = parseArgs(process.argv.slice(2));
    switch (args.command) {
      case "start":
        start(args);
        break;
      case "serve":
        serve(args.stateFile);
        break;
      case "stop":
        stop(args);
        break;
      case "print-artifact":
        printArtifact(args);
        break;
      case "print-expected-version":
        printExpectedVersion(args);
        break;
      default:
        throw new Error(`unknown command: ${args.command || "<missing>"}`);
    }
  } catch (error) {
    const message = error && error.stack ? error.stack : String(error);
    process.stderr.write(`${message}\n`);
    process.exit(1);
  }
}

module.exports = {
  buildManifest,
  buildTauriManifest,
  discoverDaemonArtifacts,
  platformKeyForArch,
  resolveArtifactForArch,
  resolveExpectedManagedVersion,
};
