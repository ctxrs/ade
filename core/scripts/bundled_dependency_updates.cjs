#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const https = require("node:https");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");

const providerMatrixPath = path.join(coreRoot, "crates", "ctx-http", "src", "provider_matrix.json");
const runtimeLockPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles", "runtime_lock.v2.json");
const installerRsPath = path.join(coreRoot, "crates", "ctx-http", "src", "installer.rs");
const providersE2ePath = path.join(coreRoot, "scripts", "providers_e2e.sh");

const modeArg = (process.argv[2] || "check").trim().toLowerCase();
if (!["check", "apply"].includes(modeArg)) {
  console.error("usage: node core/scripts/bundled_dependency_updates.cjs <check|apply>");
  process.exit(2);
}
const applyMode = modeArg === "apply";

const workspaceProviderVersionSources = {
  "acp-crp-bridge": { kind: "cargo", relPath: "external-harnesses/acp-crp-bridge/Cargo.toml" },
  amp: { kind: "package_json", relPath: "harness-adapters/example-acp/package.json" },
  droid: { kind: "cargo", relPath: "harness-adapters/droid-acp/Cargo.toml" },
  goose: { kind: "package_json", relPath: "harness-adapters/openhands-acp/package.json" },
  openhands: { kind: "package_json", relPath: "harness-adapters/openhands-acp/package.json" },
  pi: { kind: "package_json", relPath: "harness-adapters/pi-acp/package.json" },
  "claude-crp": { kind: "package_json", relPath: "external-harnesses/claude-crp/package.json" },
};

const providerUpstreamVersionSources = {
  amp: { kind: "npm", package: "@example/sdk" },
  goose: { kind: "github_release", repo: "block/goose" },
  kiro: {
    kind: "json_manifest_version",
    url: "https://desktop-release.q.us-east-1.amazonaws.com/latest/manifest.json",
    field: "version",
  },
  openhands: { kind: "github_release", repo: "All-Hands-AI/OpenHands" },
  pi: { kind: "npm", package: "@mariozechner/pi-coding-agent" },
};

const workspaceProviderIdsWithoutUpstreamTracking = new Set(["acp-crp-bridge", "claude-crp", "droid"]);

const githubReleaseCache = new Map();

const readJson = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));
const writeJson = (filePath, value) =>
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");

const normalizeVersion = (raw) => String(raw || "").trim().replace(/^v/i, "");

const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const readCargoVersion = (filePath) => {
  const text = fs.readFileSync(filePath, "utf8");
  const lines = text.split(/\n/);
  let inPackage = false;
  for (const rawLine of lines) {
    const line = rawLine.replace(/^\uFEFF/, "");
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1].trim() === "package";
      continue;
    }
    if (!inPackage) continue;
    const match = line.match(/^\s*version\s*=\s*"([^"]+)"\s*(?:#.*)?$/);
    if (match) return match[1];
  }
  throw new Error(`failed to read Cargo package version from ${filePath}`);
};

const readRustConst = (filePath, constName) => {
  const source = fs.readFileSync(filePath, "utf8");
  const match = source.match(new RegExp(`const\\s+${constName}[^=]*=\\s*"([^"]+)"`));
  if (!match || !match[1]) {
    throw new Error(`failed to resolve const ${constName} from ${filePath}`);
  }
  return match[1];
};

const updateRustConst = (filePath, constName, nextValue) => {
  const source = fs.readFileSync(filePath, "utf8");
  const pattern = new RegExp(`(const\\s+${constName}[^=]*=\\s*")([^"]+)(";)`);
  if (!pattern.test(source)) {
    throw new Error(`failed to update const ${constName} in ${filePath}`);
  }
  const updated = source.replace(pattern, `$1${nextValue}$3`);
  fs.writeFileSync(filePath, updated, "utf8");
};

const updateShellDefaultPodmanVersion = (filePath, nextVersion) => {
  const source = fs.readFileSync(filePath, "utf8");
  const pattern = /(local podman_version="\$\{PODMAN_VERSION:-)([^}]+)(\}")/;
  if (!pattern.test(source)) {
    throw new Error(`failed to update podman default in ${filePath}`);
  }
  const updated = source.replace(pattern, `$1${nextVersion}$3`);
  fs.writeFileSync(filePath, updated, "utf8");
};

const workspaceVersionForProvider = (providerId) => {
  const source = workspaceProviderVersionSources[providerId];
  if (!source) return null;
  const absPath = path.join(repoRoot, source.relPath);
  if (!fs.existsSync(absPath)) {
    throw new Error(`workspace version source missing for ${providerId}: ${absPath}`);
  }
  if (source.kind === "package_json") {
    const parsed = readJson(absPath);
    const version = String(parsed?.version || "").trim();
    if (!version) throw new Error(`missing package.json version for ${providerId}: ${absPath}`);
    return version;
  }
  if (source.kind === "cargo") {
    return readCargoVersion(absPath);
  }
  throw new Error(`unsupported workspace version source kind: ${source.kind}`);
};

const workspaceVersionSourceRelPath = (providerId) =>
  workspaceProviderVersionSources[providerId]?.relPath || null;

const request = (url, { headers = {}, binary = false } = {}, redirects = 6) =>
  new Promise((resolve, reject) => {
    const req = https.get(
      url,
      {
        headers: {
          "user-agent": "ctx-bundled-dependency-updates",
          accept: binary ? "*/*" : "application/json",
          ...headers,
        },
      },
      (res) => {
        const status = Number(res.statusCode || 0);
        if ([301, 302, 303, 307, 308].includes(status) && res.headers.location && redirects > 0) {
          const nextUrl = new URL(res.headers.location, url).toString();
          res.resume();
          resolve(request(nextUrl, { headers, binary }, redirects - 1));
          return;
        }
        if (status < 200 || status >= 300) {
          const chunks = [];
          res.on("data", (chunk) => chunks.push(chunk));
          res.on("end", () => {
            const body = Buffer.concat(chunks).toString("utf8");
            reject(new Error(`request failed (${status}) ${url}${body ? `: ${body.slice(0, 280)}` : ""}`));
          });
          return;
        }
        const chunks = [];
        res.on("data", (chunk) => chunks.push(chunk));
        res.on("end", () => {
          const data = Buffer.concat(chunks);
          resolve(binary ? data : data.toString("utf8"));
        });
      },
    );
    req.on("error", reject);
  });

const fetchJson = async (url) => JSON.parse(await request(url));

const fetchGithubLatestRelease = async (repo) => {
  if (githubReleaseCache.has(repo)) return githubReleaseCache.get(repo);
  const data = await fetchJson(`https://api.github.com/repos/${repo}/releases/latest`);
  const tag = String(data?.tag_name || "").trim();
  if (!tag) throw new Error(`missing tag_name for ${repo} latest release`);
  const assets = Array.isArray(data?.assets) ? data.assets : [];
  const byName = new Map();
  const byUrl = new Map();
  for (const asset of assets) {
    const name = String(asset?.name || "").trim();
    const url = String(asset?.browser_download_url || "").trim();
    if (!name || !url) continue;
    byName.set(name, url);
    byUrl.set(url, name);
  }
  const resolved = { repo, tag, version: normalizeVersion(tag), assetsByName: byName, assetsByUrl: byUrl };
  githubReleaseCache.set(repo, resolved);
  return resolved;
};

const fetchNpmLatest = async (pkgName) => {
  const encoded = pkgName.replace("/", "%2F");
  const latest = await fetchJson(`https://registry.npmjs.org/${encoded}/latest`);
  const version = String(latest?.version || "").trim();
  if (!version) throw new Error(`missing npm latest version for ${pkgName}`);
  return version;
};

const fetchPypiLatest = async (pkgName) => {
  const payload = await fetchJson(`https://pypi.org/pypi/${pkgName}/json`);
  const version = String(payload?.info?.version || "").trim();
  if (!version) throw new Error(`missing PyPI latest version for ${pkgName}`);
  return version;
};

const fetchKiroCliManagedLatest = async () => {
  const manifestUrl = "https://desktop-release.q.us-east-1.amazonaws.com/latest/manifest.json";
  const baseUrl = "https://desktop-release.q.us-east-1.amazonaws.com";
  const payload = await fetchJson(manifestUrl);
  const version = normalizeVersion(String(payload?.version || "").trim());
  if (!version) {
    throw new Error("missing Kiro CLI manifest version");
  }
  const packages = Array.isArray(payload?.packages) ? payload.packages : [];
  const findPackage = ({ os, architecture, fileType, variant }) =>
    packages.find(
      (pkg) =>
        String(pkg?.os || "").trim() === os &&
        String(pkg?.architecture || "").trim() === architecture &&
        String(pkg?.fileType || "").trim() === fileType &&
        String(pkg?.variant || "").trim() === variant,
    );

  const mac = findPackage({
    os: "macos",
    architecture: "universal",
    fileType: "dmg",
    variant: "full",
  });
  const linuxArm = findPackage({
    os: "linux",
    architecture: "aarch64",
    fileType: "zip",
    variant: "headless",
  });
  const linuxX64 = findPackage({
    os: "linux",
    architecture: "x86_64",
    fileType: "zip",
    variant: "headless",
  });
  if (!mac || !linuxArm || !linuxX64) {
    throw new Error("failed to resolve required Kiro CLI packages from manifest");
  }
  const macDownload = String(mac.download || "").trim();
  const linuxArmDownload = String(linuxArm.download || "").trim();
  const linuxX64Download = String(linuxX64.download || "").trim();
  if (!macDownload || !linuxArmDownload || !linuxX64Download) {
    throw new Error("Kiro CLI manifest package missing download path");
  }

  const targets = {
    "darwin-aarch64": {
      url: `${baseUrl}/${macDownload}`,
      archive: "dmg",
      bin_path: "kiro-cli",
    },
    "darwin-x86_64": {
      url: `${baseUrl}/${macDownload}`,
      archive: "dmg",
      bin_path: "kiro-cli",
    },
    "linux-aarch64": {
      url: `${baseUrl}/${linuxArmDownload}`,
      archive: "zip",
      bin_path: "kiro-cli",
    },
    "linux-x86_64": {
      url: `${baseUrl}/${linuxX64Download}`,
      archive: "zip",
      bin_path: "kiro-cli",
    },
  };

  return {
    version,
    manifestUrl,
    targets,
  };
};

const parseGithubReleaseRepoFromUrl = (rawUrl) => {
  const match = String(rawUrl || "").match(
    /^https:\/\/github\.com\/([^/]+\/[^/]+)\/releases\/download\/[^/]+\/[^/]+$/,
  );
  return match ? match[1] : null;
};

const currentProviderVersion = (entry) => {
  const managed = entry.managed_install || {};
  const kind = managed.kind;
  if (kind === "archive" || kind === "python") {
    const managedVersion = String(managed.version || "").trim();
    if (managedVersion) return managedVersion;
  }
  const supported = (entry.releases || []).find((release) => String(release?.status || "supported") === "supported");
  if (supported?.version) return String(supported.version).trim();
  if (entry.releases && entry.releases[0]?.version) return String(entry.releases[0].version).trim();
  return "";
};

const currentProviderUpstreamVersion = (entry) => {
  const primaryRelease = Array.isArray(entry?.releases) ? entry.releases[0] : null;
  return String(primaryRelease?.upstream_version || "").trim();
};

const ensurePrimaryRelease = (entry) => {
  if (!Array.isArray(entry.releases)) entry.releases = [];
  if (entry.releases.length === 0) {
    entry.releases.push({
      version: "",
      status: "supported",
      context_min: "0.1.0",
      notes: "",
    });
  }
  return entry.releases[0];
};

const resolveProviderUpstreamLatest = async (providerId) => {
  const source = providerUpstreamVersionSources[providerId];
  if (!source) {
    return null;
  }
  if (source.kind === "npm") {
    const pkg = String(source.package || "").trim();
    if (!pkg) throw new Error(`missing upstream npm package for ${providerId}`);
    return {
      resolver: "upstream:npm",
      sourcePath: pkg,
      latestVersion: await fetchNpmLatest(pkg),
    };
  }
  if (source.kind === "github_release") {
    const repo = String(source.repo || "").trim();
    if (!repo) throw new Error(`missing upstream github repo for ${providerId}`);
    const release = await fetchGithubLatestRelease(repo);
    return {
      resolver: `upstream:github:${repo}`,
      sourcePath: repo,
      latestVersion: release.version,
    };
  }
  if (source.kind === "json_manifest_version") {
    const url = String(source.url || "").trim();
    const field = String(source.field || "").trim();
    if (!url || !field) throw new Error(`missing upstream manifest resolver fields for ${providerId}`);
    const payload = await fetchJson(url);
    const rawVersion = String(payload?.[field] || "").trim();
    if (!rawVersion) {
      throw new Error(`missing upstream manifest version field '${field}' for ${providerId}`);
    }
    return {
      resolver: "upstream:manifest",
      sourcePath: url,
      latestVersion: normalizeVersion(rawVersion),
    };
  }
  throw new Error(`unsupported upstream source kind for ${providerId}: ${source.kind}`);
};

const rewriteGithubUrlVersion = ({ url, currentVersion, nextVersion }) => {
  const current = String(currentVersion || "").trim();
  const next = String(nextVersion || "").trim();
  if (!current || !next || current === next) return url;
  let out = String(url);
  const replacements = [
    [`/releases/download/v${current}/`, `/releases/download/v${next}/`],
    [`/releases/download/${current}/`, `/releases/download/v${next}/`],
    [`-${current}-`, `-${next}-`],
    [`_${current}_`, `_${next}_`],
    [`-${current}.`, `-${next}.`],
    [`_${current}.`, `_${next}.`],
    [`v${current}`, `v${next}`],
  ];
  for (const [from, to] of replacements) {
    out = out.replaceAll(from, to);
  }
  return out;
};

const resolveProviderLatest = async (entry) => {
  const providerId = String(entry.id || "").trim();
  const managed = entry.managed_install || {};
  const kind = managed.kind;

  if (providerId === "kiro") {
    const kiro = await fetchKiroCliManagedLatest();
    return {
      providerId,
      resolver: "manifest:kiro",
      latestVersion: kiro.version,
      sourcePath: kiro.manifestUrl,
      updateTargets: { type: "replace_targets", targets: kiro.targets },
    };
  }

  const workspaceVersion = workspaceVersionForProvider(providerId);
  if (workspaceVersion) {
    return {
      providerId,
      resolver: "workspace",
      latestVersion: workspaceVersion,
      sourcePath: workspaceVersionSourceRelPath(providerId),
      updateTargets: null,
    };
  }

  if (kind === "npm") {
    const pkg = String(managed.package || "").trim();
    return {
      providerId,
      resolver: "npm",
      latestVersion: await fetchNpmLatest(pkg),
      sourcePath: pkg,
      updateTargets: null,
    };
  }

  if (kind === "python") {
    const pkg = String(managed.package || "").trim();
    return {
      providerId,
      resolver: "pypi",
      latestVersion: await fetchPypiLatest(pkg),
      sourcePath: pkg,
      updateTargets: null,
    };
  }

  if (kind === "archive") {
    const targets = managed.targets && typeof managed.targets === "object" ? Object.values(managed.targets) : [];
    const firstUrl = String(targets[0]?.url || "").trim();
    const repo = parseGithubReleaseRepoFromUrl(firstUrl);
    if (!repo) {
      return {
        providerId,
        resolver: "archive_unresolved",
        latestVersion: currentProviderVersion(entry),
        sourcePath: null,
        updateTargets: null,
      };
    }
    const release = await fetchGithubLatestRelease(repo);
    return {
      providerId,
      resolver: `github:${repo}`,
      latestVersion: release.version,
      sourcePath: repo,
      updateTargets: { repo, release },
    };
  }

  throw new Error(`unsupported managed install kind for ${providerId}: ${kind}`);
};

const hashUrlSha256 = async (url) => {
  const data = await request(url, { binary: true });
  return crypto.createHash("sha256").update(data).digest("hex");
};

const fetchNodeLatestLts = async () => {
  const index = await fetchJson("https://nodejs.org/dist/index.json");
  if (!Array.isArray(index) || index.length === 0) throw new Error("unexpected Node index response");
  const lts = index.find((entry) => entry && entry.lts);
  const version = normalizeVersion(lts?.version || "");
  if (!version) throw new Error("failed to determine latest Node LTS");
  return version;
};

const fetchLatestPython313 = async () => {
  const release = await fetchGithubLatestRelease("indygreg/python-build-standalone");
  const buildTag = normalizeVersion(release.tag);
  const targetSuffixes = [
    "aarch64-apple-darwin-install_only.tar.gz",
    "aarch64-unknown-linux-gnu-install_only.tar.gz",
    "x86_64-unknown-linux-gnu-install_only.tar.gz",
  ];
  const versions = new Set();
  for (const assetName of release.assetsByName.keys()) {
    if (!targetSuffixes.some((suffix) => assetName.endsWith(suffix))) continue;
    const match = assetName.match(/^cpython-(3\.13\.\d+)\+\d{8}-/);
    if (match?.[1]) versions.add(match[1]);
  }
  if (versions.size === 0) throw new Error("failed to determine latest Python 3.13 from python-build-standalone");
  const sorted = [...versions].sort((a, b) => {
    const pa = a.split(".").map((v) => Number(v));
    const pb = b.split(".").map((v) => Number(v));
    for (let idx = 0; idx < Math.max(pa.length, pb.length); idx += 1) {
      const aa = pa[idx] || 0;
      const bb = pb[idx] || 0;
      if (aa !== bb) return aa - bb;
    }
    return 0;
  });
  return { version: sorted[sorted.length - 1], buildTag };
};

const fetchLatestPodmanBundleMetadata = async () => {
  const podman = await fetchGithubLatestRelease("containers/podman");
  const gvproxy = await fetchGithubLatestRelease("containers/gvisor-tap-vsock");
  const vfkit = await fetchGithubLatestRelease("crc-org/vfkit");

  const armAsset = "podman-remote-release-darwin_arm64.zip";
  const x64Asset = "podman-remote-release-darwin_amd64.zip";
  const gvAsset = "gvproxy-darwin";
  const vfAsset = "vfkit-unsigned";

  const armUrl = podman.assetsByName.get(armAsset);
  const x64Url = podman.assetsByName.get(x64Asset);
  const gvUrl = gvproxy.assetsByName.get(gvAsset);
  const vfUrl = vfkit.assetsByName.get(vfAsset);
  if (!armUrl || !x64Url || !gvUrl || !vfUrl) {
    throw new Error("failed to resolve required podman helper assets");
  }

  const [armSha, x64Sha, gvSha, vfSha] = await Promise.all([
    hashUrlSha256(armUrl),
    hashUrlSha256(x64Url),
    hashUrlSha256(gvUrl),
    hashUrlSha256(vfUrl),
  ]);

  return {
    version: podman.version,
    arm64: { url: armUrl, sha256: armSha },
    x64: { url: x64Url, sha256: x64Sha },
    gvproxy: { url: gvUrl, sha256: gvSha },
    vfkit: { url: vfUrl, sha256: vfSha },
  };
};

const updatePodmanRuntimeLock = (runtimeLock, podmanMeta) => {
  const components = Array.isArray(runtimeLock.components) ? runtimeLock.components : [];
  for (const component of components) {
    if (!component || component.kind !== "runtime" || component.id !== "podman" || component.os !== "macos") {
      continue;
    }
    const arch = component.arch;
    const isArm = arch === "aarch64";
    const podmanAsset = isArm ? podmanMeta.arm64 : podmanMeta.x64;
    if (!podmanAsset) continue;
    component.version = podmanMeta.version;
    component.bin = "usr/bin/podman";
    component.helpers = {
      gvproxy: { uri: podmanMeta.gvproxy.url, sha256: podmanMeta.gvproxy.sha256 },
      vfkit: { uri: podmanMeta.vfkit.url, sha256: podmanMeta.vfkit.sha256 },
    };
    if (!Array.isArray(component.sources)) component.sources = [];
    let updatedVendor = false;
    for (const source of component.sources) {
      const sourceType = String(source?.source_type || "").trim();
      if (sourceType !== "vendor" && sourceType !== "ci") continue;
      source.uri = podmanAsset.url;
      source.sha256 = podmanAsset.sha256;
      updatedVendor = true;
    }
    if (!updatedVendor) {
      component.sources.push({
        source_type: "vendor",
        uri: podmanAsset.url,
        sha256: podmanAsset.sha256,
      });
    }
  }
};

const requiredProviderIdsFromRuntimeLock = (runtimeLock) => {
  const raw = runtimeLock?.required?.provider_ids;
  if (!Array.isArray(raw)) return [];
  return raw
    .map((value) => String(value || "").trim())
    .filter((value) => value.length > 0);
};

const providerPolicyIssues = ({ providerId, entry, latestInfo, upstreamInfo, currentUpstreamVersion }) => {
  const issues = [];
  const managed = entry?.managed_install || {};
  const primaryRelease = Array.isArray(entry?.releases) ? entry.releases[0] : null;
  const releaseStatus = String(primaryRelease?.status || "supported").trim();
  const releaseNotes = String(primaryRelease?.notes || "").trim().toLowerCase();

  if (releaseStatus !== "supported") {
    issues.push(`release status '${releaseStatus}' (expected supported)`);
  }
  if (releaseNotes.includes("internal adapter source") || releaseNotes.includes("internal bridge source")) {
    issues.push("release notes contain internal placeholder wording");
  }
  if (releaseNotes.includes("pending for managed installs")) {
    issues.push("release notes indicate pending managed install placeholder");
  }

  if (managed.kind === "archive") {
    const targets = managed.targets && typeof managed.targets === "object" ? managed.targets : {};
    const targetCount = Object.keys(targets).length;
    const workspaceBacked = Boolean(workspaceProviderVersionSources[providerId]);
    if (!workspaceBacked && targetCount === 0) {
      issues.push("archive managed install has no targets");
    }
  }

  if (latestInfo.resolver === "archive_unresolved") {
    issues.push("archive source resolver is unresolved");
  }

  if (
    workspaceProviderVersionSources[providerId] &&
    !workspaceProviderIdsWithoutUpstreamTracking.has(providerId)
  ) {
    if (!upstreamInfo) {
      issues.push("workspace adapter is missing upstream version source mapping");
    } else {
      if (!currentUpstreamVersion) {
        issues.push("workspace adapter release is missing upstream_version");
      }
      if (currentUpstreamVersion && upstreamInfo.latestVersion && currentUpstreamVersion !== upstreamInfo.latestVersion) {
        issues.push(
          `workspace adapter upstream stale (${currentUpstreamVersion} -> ${upstreamInfo.latestVersion})`,
        );
      }
    }
  }

  return issues;
};

const workspacePinnedReleaseNote = (providerId) => {
  const sourcePath = workspaceVersionSourceRelPath(providerId);
  if (!sourcePath) {
    return "Pinned workspace adapter source (bundled from repository at build time)";
  }
  return `Pinned workspace adapter source (${sourcePath})`;
};

const main = async () => {
  const matrix = readJson(providerMatrixPath);
  const runtimeLock = readJson(runtimeLockPath);
  const providerEntries = Array.isArray(matrix.providers) ? matrix.providers : [];
  const providerEntriesById = new Map(
    providerEntries
      .filter((entry) => entry && entry.id)
      .map((entry) => [String(entry.id), entry]),
  );
  const requiredProviderIds = requiredProviderIdsFromRuntimeLock(runtimeLock);

  const missingRequiredProviders = [];
  const missingRequiredManagedSources = [];
  const providersToCheck = [];

  if (requiredProviderIds.length > 0) {
    for (const providerId of requiredProviderIds) {
      const entry = providerEntriesById.get(providerId);
      if (!entry) {
        missingRequiredProviders.push(providerId);
        continue;
      }
      if (!entry.managed_install) {
        missingRequiredManagedSources.push(providerId);
        continue;
      }
      providersToCheck.push(entry);
    }
  } else {
    for (const entry of providerEntries) {
      if (!entry || !entry.managed_install) continue;
      providersToCheck.push(entry);
    }
  }

  const providerReports = [];
  for (const entry of providersToCheck) {
    const providerId = String(entry.id || "").trim();
    const current = currentProviderVersion(entry);
    const currentUpstream = currentProviderUpstreamVersion(entry);
    const latestInfo = await resolveProviderLatest(entry);
    const upstreamInfo = await resolveProviderUpstreamLatest(providerId);
    const policyIssues = providerPolicyIssues({
      providerId,
      entry,
      latestInfo,
      upstreamInfo,
      currentUpstreamVersion: currentUpstream,
    });
    providerReports.push({
      id: providerId,
      resolver: latestInfo.resolver,
      currentVersion: current,
      latestVersion: latestInfo.latestVersion,
      sourcePath: latestInfo.sourcePath || null,
      currentUpstreamVersion: currentUpstream,
      latestUpstreamVersion: upstreamInfo?.latestVersion || "",
      upstreamResolver: upstreamInfo?.resolver || "",
      upstreamSourcePath: upstreamInfo?.sourcePath || "",
      policyIssues,
      updateTargets: latestInfo.updateTargets,
    });
  }

  const currentNode = readRustConst(installerRsPath, "NODE_VERSION");
  const currentPython = readRustConst(installerRsPath, "PYTHON_VERSION");
  const currentPythonTag = readRustConst(installerRsPath, "PYTHON_BUILD_TAG");
  const latestNode = await fetchNodeLatestLts();
  const latestPython = await fetchLatestPython313();

  const podmanComponents = (runtimeLock.components || []).filter(
    (c) => c && c.kind === "runtime" && c.id === "podman" && c.os === "macos",
  );
  const currentPodman = String(podmanComponents[0]?.version || "").trim();
  const podmanRelease = await fetchGithubLatestRelease("containers/podman");
  const latestPodmanVersion = podmanRelease.version;

  const staleProviders = providerReports.filter(
    (row) => row.currentVersion && row.latestVersion && row.currentVersion !== row.latestVersion,
  );
  const staleProviderUpstreams = providerReports.filter(
    (row) =>
      row.latestUpstreamVersion &&
      (!row.currentUpstreamVersion || row.currentUpstreamVersion !== row.latestUpstreamVersion),
  );
  const policyViolations = providerReports.filter((row) => row.policyIssues.length > 0);
  const staleRuntimeRows = [
    { id: "node", current: currentNode, latest: latestNode },
    { id: "python", current: currentPython, latest: latestPython.version },
    { id: "python-build-tag", current: currentPythonTag, latest: latestPython.buildTag },
    { id: "podman", current: currentPodman, latest: latestPodmanVersion },
  ].filter((row) => row.current !== row.latest);

  for (const row of providerReports) {
    const status =
      row.policyIssues.length > 0
        ? "policy"
        : row.currentVersion === row.latestVersion
          ? "ok"
          : "update";
    console.log(
      `provider:${status}\t${row.id}\t${row.currentVersion || "<unset>"}\t${row.latestVersion || "<unknown>"}\t${row.resolver}\t${row.sourcePath || "-"}`,
    );
    if (row.upstreamResolver) {
      const upstreamStatus =
        row.currentUpstreamVersion === row.latestUpstreamVersion ? "ok" : "update";
      console.log(
        `provider-upstream:${upstreamStatus}\t${row.id}\t${row.currentUpstreamVersion || "<unset>"}\t${row.latestUpstreamVersion || "<unknown>"}\t${row.upstreamResolver}\t${row.upstreamSourcePath || "-"}`,
      );
    }
    for (const issue of row.policyIssues) {
      console.log(`provider:policy-detail\t${row.id}\t${issue}`);
    }
  }
  for (const missing of missingRequiredProviders) {
    console.log(`provider:error\t${missing}\tmissing provider matrix entry`);
  }
  for (const missing of missingRequiredManagedSources) {
    console.log(`provider:error\t${missing}\tmissing managed_install source`);
  }
  for (const row of [
    { id: "node", current: currentNode, latest: latestNode, resolver: "nodejs-lts" },
    { id: "python", current: currentPython, latest: latestPython.version, resolver: "python-build-standalone" },
    {
      id: "python-build-tag",
      current: currentPythonTag,
      latest: latestPython.buildTag,
      resolver: "python-build-standalone",
    },
    { id: "podman", current: currentPodman, latest: latestPodmanVersion, resolver: "github:containers/podman" },
  ]) {
    const status = row.current === row.latest ? "ok" : "update";
    console.log(`runtime:${status}\t${row.id}\t${row.current || "<unset>"}\t${row.latest}\t${row.resolver}`);
  }

  if (!applyMode) {
    if (
      staleProviders.length > 0 ||
      staleProviderUpstreams.length > 0 ||
      staleRuntimeRows.length > 0 ||
      policyViolations.length > 0 ||
      missingRequiredProviders.length > 0 ||
      missingRequiredManagedSources.length > 0
    ) {
      process.exit(1);
    }
    return;
  }

  if (missingRequiredProviders.length > 0 || missingRequiredManagedSources.length > 0) {
    throw new Error(
      `cannot apply with missing managed providers (missing=${missingRequiredProviders.length}, missing_managed=${missingRequiredManagedSources.length})`,
    );
  }

  for (const row of providerReports) {
    const entry = providerEntriesById.get(row.id);
    if (!entry) continue;
    const managed = entry.managed_install || {};
    const nextVersion = row.latestVersion || row.currentVersion;

    if (nextVersion && (managed.kind === "archive" || managed.kind === "python")) {
      managed.version = nextVersion;
    }

    const release = ensurePrimaryRelease(entry);
    if (nextVersion) {
      release.version = nextVersion;
    }
    release.status = "supported";
    if (!release.context_min) release.context_min = "0.1.0";

    if (workspaceProviderVersionSources[row.id]) {
      release.notes = workspacePinnedReleaseNote(row.id);
    }
    if (row.latestUpstreamVersion) {
      release.upstream_version = row.latestUpstreamVersion;
    }

    if (
      managed.kind === "archive" &&
      row.updateTargets?.release &&
      managed.targets &&
      typeof managed.targets === "object" &&
      row.currentVersion !== nextVersion
    ) {
      const currentVersion = row.currentVersion;
      const assetUrls = row.updateTargets.release.assetsByUrl;
      for (const targetKey of Object.keys(managed.targets)) {
        const target = managed.targets[targetKey];
        if (!target || !target.url) continue;
        const rewritten = rewriteGithubUrlVersion({
          url: target.url,
          currentVersion,
          nextVersion,
        });
        if (assetUrls.has(rewritten)) {
          target.url = rewritten;
        }
      }
    }

    if (managed.kind === "archive" && row.updateTargets?.type === "replace_targets") {
      managed.targets = JSON.parse(JSON.stringify(row.updateTargets.targets || {}));
    }
  }

  if (currentNode !== latestNode) {
    updateRustConst(installerRsPath, "NODE_VERSION", latestNode);
  }
  if (currentPython !== latestPython.version) {
    updateRustConst(installerRsPath, "PYTHON_VERSION", latestPython.version);
  }
  if (currentPythonTag !== latestPython.buildTag) {
    updateRustConst(installerRsPath, "PYTHON_BUILD_TAG", latestPython.buildTag);
  }

  if (currentPodman !== latestPodmanVersion) {
    const podmanMeta = await fetchLatestPodmanBundleMetadata();
    updatePodmanRuntimeLock(runtimeLock, podmanMeta);
    updateShellDefaultPodmanVersion(providersE2ePath, podmanMeta.version);
  }

  writeJson(providerMatrixPath, matrix);
  writeJson(runtimeLockPath, runtimeLock);
};

main().catch((error) => {
  console.error(`error: ${error?.message || error}`);
  process.exit(1);
});
