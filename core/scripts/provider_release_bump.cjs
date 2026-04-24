#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const { applyIndexOverlay } = require("./provider_deps_matrix_overlay.cjs");

const coreRoot = path.resolve(__dirname, "..");
const defaultRepoRoot = path.resolve(coreRoot, "..");
const defaultMatrixPath = path.join(coreRoot, "crates", "ctx-provider-accounts", "src", "provider_matrix.json");

const workspaceVersionSources = {
  "acp-crp-bridge": { kind: "cargo", relPath: "external-harnesses/acp-crp-bridge/Cargo.toml" },
  amp: { kind: "package_json", relPath: "harness-adapters/example-acp/package.json" },
  "codex-crp": { kind: "cargo", relPath: "core/crates/codex-crp/Cargo.toml" },
  "claude-crp": { kind: "package_json", relPath: "external-harnesses/claude-crp/package.json" },
  droid: { kind: "cargo", relPath: "harness-adapters/droid-acp/Cargo.toml" },
  pi: { kind: "package_json", relPath: "harness-adapters/pi-acp/package.json" },
};

const DEFAULT_CODEX_PROVENANCE = Object.freeze({
  upstream_repo: "openai/codex",
  ctx_repo: "ctxorgrs/codex-crp",
});
const DEFAULT_FETCH_TIMEOUT_MS = 5 * 60 * 1000;
const DEFAULT_HTTP_HEADERS = Object.freeze({
  accept: "application/json",
  "user-agent": "ctx-provider-release-bump",
});

function resolveInputPath(raw, fallbackRoot = process.cwd()) {
  const value = String(raw || "").trim();
  if (!value) return "";
  if (path.isAbsolute(value)) return value;
  const fromCwd = path.resolve(fallbackRoot, value);
  if (fs.existsSync(fromCwd)) return fromCwd;
  return path.resolve(coreRoot, value);
}

function parseArgs(argv) {
  const opts = {
    artifactBaseUrl: process.env.CTX_PROVIDER_ARTIFACT_BASE_URL || "",
    index: "",
    matrix: defaultMatrixPath,
    repoRoot: defaultRepoRoot,
    updates: "",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--artifact-base-url") {
      opts.artifactBaseUrl = String(argv[++index] || "").trim();
      continue;
    }
    if (arg === "--index") {
      opts.index = resolveInputPath(argv[++index] || "");
      continue;
    }
    if (arg === "--matrix") {
      opts.matrix = resolveInputPath(argv[++index] || "");
      continue;
    }
    if (arg === "--repo-root") {
      opts.repoRoot = resolveInputPath(argv[++index] || "");
      continue;
    }
    if (arg === "--updates") {
      opts.updates = resolveInputPath(argv[++index] || "");
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  return opts;
}

function printHelp() {
  console.log(
    [
      "Usage: node core/scripts/provider_release_bump.cjs --updates <provider_updates.json> [options]",
      "",
      "Mutates checked-in provider version sources in place.",
      "",
      "Options:",
      "  --updates <path>            JSON spec describing provider bumps (required)",
      "  --matrix <path>             Provider matrix JSON (default: checked-in matrix)",
      "  --repo-root <path>          Repo root used to resolve workspace manifests",
      "  --index <path>              Optional staged provider index to refresh archive targets",
      "  --artifact-base-url <url>   Immutable provider artifact base URL used with --index",
      "  --help                      Show help",
      "",
      "Update spec shape:",
      '  { "providers": [',
      '      {',
      '        "id": "codex-crp",',
      '        "version": "1.0.0",',
      '        "upstream_version": "0.121.0",',
      '        "workspace_version": "1.0.0",',
      '        "package_dependencies": { "@scope/pkg": "1.2.3" },',
      '        "github_release": { "tag": "v1.2.3" },',
      '        "provenance": {',
      '          "upstream_release_tag": "rust-v0.121.0",',
      '          "upstream_commit_sha": "<40-char sha>",',
      '          "ctx_release_tag": "v1.0.0"',
      "        }",
      "      }",
      "    ] }",
    ].join("\n"),
  );
}

function readJson(filePath, label) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse ${label} at ${filePath}: ${error?.message || String(error)}`);
  }
}

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function normalizeCodexUpstreamVersion(rawTag) {
  return String(rawTag || "").trim().replace(/^rust-v/i, "");
}

function supportedRelease(entry) {
  const releases = Array.isArray(entry?.releases) ? entry.releases : [];
  return (
    releases.find((release) => release && typeof release === "object" && String(release.status || "supported") === "supported")
    || releases.find((release) => release && typeof release === "object")
    || null
  );
}

function sortObjectKeys(value) {
  return Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)));
}

function assertBumpSpec(update) {
  const providerId = String(update?.id || "").trim();
  const version = String(update?.version || "").trim();
  if (!providerId) {
    throw new Error("provider update is missing id");
  }
  if (!version) {
    throw new Error(`provider '${providerId}' is missing version`);
  }
  return { providerId, version };
}

function parseGithubReleaseRepoFromUrl(rawUrl) {
  const match = String(rawUrl || "").match(
    /^https:\/\/github\.com\/([^/]+\/[^/]+)\/releases\/download\/[^/]+\/[^/]+$/,
  );
  return match ? match[1] : "";
}

function basenameFromUrl(rawUrl) {
  const parsed = new URL(String(rawUrl || ""));
  return path.posix.basename(parsed.pathname);
}

async function fetchJson(url, headers = {}) {
  const response = await fetch(url, {
    headers: {
      ...DEFAULT_HTTP_HEADERS,
      ...headers,
    },
    redirect: "follow",
    signal: AbortSignal.timeout(DEFAULT_FETCH_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new Error(`request failed (${response.status}) ${url}`);
  }
  return response.json();
}

async function fetchSha256(url) {
  const response = await fetch(url, {
    redirect: "follow",
    signal: AbortSignal.timeout(DEFAULT_FETCH_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new Error(`request failed (${response.status}) ${url}`);
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function createNetworkHooks() {
  const githubReleaseCache = new Map();
  const shaCache = new Map();

  return {
    async fetchGithubRelease(repo, tag) {
      const normalizedRepo = String(repo || "").trim();
      const normalizedTag = String(tag || "").trim();
      const cacheKey = `${normalizedRepo}@${normalizedTag}`;
      if (githubReleaseCache.has(cacheKey)) {
        return githubReleaseCache.get(cacheKey);
      }
      const encodedTag = encodeURIComponent(normalizedTag);
      const payload = await fetchJson(`https://api.github.com/repos/${normalizedRepo}/releases/tags/${encodedTag}`);
      const assetsByName = new Map();
      for (const asset of Array.isArray(payload?.assets) ? payload.assets : []) {
        const name = String(asset?.name || "").trim();
        const url = String(asset?.browser_download_url || "").trim();
        if (!name || !url) continue;
        assetsByName.set(name, url);
      }
      const resolved = {
        assetsByName,
        repo: normalizedRepo,
        tag: normalizedTag,
      };
      githubReleaseCache.set(cacheKey, resolved);
      return resolved;
    },
    async hashUrlSha256(url) {
      const normalizedUrl = String(url || "").trim();
      if (shaCache.has(normalizedUrl)) {
        return shaCache.get(normalizedUrl);
      }
      const sha256 = await fetchSha256(normalizedUrl);
      shaCache.set(normalizedUrl, sha256);
      return sha256;
    },
  };
}

function updateCargoPackageVersion(filePath, nextVersion) {
  const source = fs.readFileSync(filePath, "utf8");
  const lines = source.split("\n");
  let inPackage = false;
  let updated = false;
  const output = lines.map((line) => {
    const section = line.match(/^\s*\[([^\]]+)\]\s*$/);
    if (section) {
      inPackage = section[1].trim() === "package";
      return line;
    }
    if (!inPackage) return line;
    if (/^\s*version\s*=/.test(line)) {
      updated = true;
      return line.replace(/^\s*version\s*=\s*"([^"]+)"(\s*(?:#.*)?)$/, `version = "${nextVersion}"$2`);
    }
    return line;
  });
  if (!updated) {
    throw new Error(`failed to update Cargo package version in ${filePath}`);
  }
  fs.writeFileSync(filePath, output.join("\n"), "utf8");
}

function updatePackageJsonFile(filePath, { version, dependencies }) {
  const parsed = readJson(filePath, "package.json");
  if (version) {
    parsed.version = version;
  }
  for (const [depName, depVersion] of Object.entries(dependencies || {})) {
    let updated = false;
    for (const section of ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"]) {
      if (!parsed[section] || typeof parsed[section] !== "object") continue;
      if (!Object.prototype.hasOwnProperty.call(parsed[section], depName)) continue;
      parsed[section][depName] = depVersion;
      updated = true;
      break;
    }
    if (!updated) {
      throw new Error(`dependency '${depName}' not found in ${filePath}`);
    }
  }
  writeJson(filePath, parsed);
}

function applyWorkspaceManifestUpdate({ providerId, repoRoot, workspaceVersion, packageDependencies }) {
  const source = workspaceVersionSources[providerId];
  if (!source) {
    if (packageDependencies && Object.keys(packageDependencies).length > 0) {
      throw new Error(`provider '${providerId}' does not support package dependency rewrites`);
    }
    return null;
  }

  const absPath = path.join(repoRoot, source.relPath);
  if (!fs.existsSync(absPath)) {
    throw new Error(`workspace version source missing for ${providerId}: ${absPath}`);
  }

  if (source.kind === "cargo") {
    if (packageDependencies && Object.keys(packageDependencies).length > 0) {
      throw new Error(`provider '${providerId}' has a Cargo workspace source; package dependency rewrites are unsupported`);
    }
    updateCargoPackageVersion(absPath, workspaceVersion);
    return absPath;
  }

  if (source.kind === "package_json") {
    updatePackageJsonFile(absPath, {
      version: workspaceVersion,
      dependencies: packageDependencies,
    });
    return absPath;
  }

  throw new Error(`unsupported workspace source kind '${source.kind}' for ${providerId}`);
}

function applyCodexProvenance({ release, update, version }) {
  const provided = update?.provenance && typeof update.provenance === "object" ? update.provenance : null;
  if (!provided) {
    throw new Error("codex bumps require a provenance object");
  }

  const upstreamReleaseTag = String(provided.upstream_release_tag || "").trim();
  const upstreamCommitSha = String(provided.upstream_commit_sha || "").trim();
  const ctxReleaseTag = String(provided.ctx_release_tag || `v${version}`).trim();

  if (!upstreamReleaseTag) {
    throw new Error("codex bumps require provenance.upstream_release_tag");
  }
  if (!/^[0-9a-f]{40}$/i.test(upstreamCommitSha)) {
    throw new Error("codex bumps require provenance.upstream_commit_sha to be a 40-character git SHA");
  }

  release.provenance = {
    ...DEFAULT_CODEX_PROVENANCE,
    ...(release.provenance && typeof release.provenance === "object" ? release.provenance : {}),
    ...provided,
    upstream_release_tag: upstreamReleaseTag,
    upstream_commit_sha: upstreamCommitSha,
    ctx_release_tag: ctxReleaseTag,
  };
  if (!String(update.upstream_version || "").trim()) {
    release.upstream_version = normalizeCodexUpstreamVersion(upstreamReleaseTag);
  }
  if (!String(update.notes || "").trim()) {
    release.notes = `Thin app-server adapter over stock Codex ${upstreamReleaseTag}`;
  }
}

async function applyGithubReleaseTargetRefresh({ entry, update, version, network }) {
  const refreshSpec = update?.github_release && typeof update.github_release === "object" ? update.github_release : null;
  if (!refreshSpec) {
    return false;
  }

  const managedInstall =
    entry?.managed_install && typeof entry.managed_install === "object" ? entry.managed_install : null;
  if (!managedInstall || String(managedInstall.kind || "").trim() !== "archive") {
    throw new Error(`provider '${String(entry?.id || "<unknown>")}' github_release refresh requires archive managed_install`);
  }

  const targets = managedInstall.targets && typeof managedInstall.targets === "object" ? managedInstall.targets : null;
  if (!targets || Object.keys(targets).length === 0) {
    throw new Error(`provider '${String(entry?.id || "<unknown>")}' archive target refresh requires existing managed_install.targets`);
  }

  const firstUrl = String(Object.values(targets)[0]?.url || "").trim();
  const repo = String(refreshSpec.repo || "").trim() || parseGithubReleaseRepoFromUrl(firstUrl);
  const tag = String(refreshSpec.tag || "").trim();
  if (!repo) {
    throw new Error(`provider '${String(entry?.id || "<unknown>")}' github_release refresh could not resolve repo`);
  }
  if (!tag) {
    throw new Error(`provider '${String(entry?.id || "<unknown>")}' github_release refresh requires tag`);
  }

  const release = await network.fetchGithubRelease(repo, tag);
  const nextTargets = {};
  for (const [targetKey, target] of Object.entries(targets)) {
    const currentUrl = String(target?.url || "").trim();
    if (!currentUrl) {
      throw new Error(`provider '${String(entry?.id || "<unknown>")}' target '${targetKey}' is missing url`);
    }
    const assetName = basenameFromUrl(currentUrl);
    const assetUrl = release.assetsByName.get(assetName);
    if (!assetUrl) {
      throw new Error(
        `provider '${String(entry?.id || "<unknown>")}' target '${targetKey}' asset '${assetName}' missing from ${repo}@${tag}`,
      );
    }
    nextTargets[targetKey] = {
      ...target,
      sha256: await network.hashUrlSha256(assetUrl),
      url: assetUrl,
    };
  }
  managedInstall.targets = sortObjectKeys(nextTargets);
  return true;
}

async function applyProviderUpdate({ matrix, repoRoot, update, warnings, targetRefreshExpected, network }) {
  const { providerId, version } = assertBumpSpec(update);
  const entry = (Array.isArray(matrix?.providers) ? matrix.providers : []).find((provider) => provider?.id === providerId);
  if (!entry) {
    throw new Error(`provider '${providerId}' not found in matrix`);
  }

  const managedInstall = entry.managed_install && typeof entry.managed_install === "object" ? entry.managed_install : null;
  const release = supportedRelease(entry);
  if (!release) {
    throw new Error(`provider '${providerId}' is missing release metadata`);
  }

  const previousVersion = String(release.version || managedInstall?.version || "").trim();
  if (managedInstall && ["archive", "npm", "python"].includes(String(managedInstall.kind || "").trim())) {
    managedInstall.version = version;
  }
  release.version = version;
  if (!release.status) release.status = "supported";
  if (!release.context_min) release.context_min = "0.1.0";

  const upstreamVersion = String(update?.upstream_version || "").trim();
  if (upstreamVersion) {
    release.upstream_version = upstreamVersion;
  }
  const notes = String(update?.notes || "").trim();
  if (notes) {
    release.notes = notes;
  }

  if (providerId === "codex-crp") {
    applyCodexProvenance({ release, update, version });
  }

  const githubReleaseRefreshed = await applyGithubReleaseTargetRefresh({
    entry,
    update,
    version,
    network,
  });

  const workspaceSourcePath = workspaceVersionSources[providerId]
    ? applyWorkspaceManifestUpdate({
        providerId,
        repoRoot,
        workspaceVersion: String(update.workspace_version || version).trim() || version,
        packageDependencies:
          update.package_dependencies && typeof update.package_dependencies === "object"
            ? update.package_dependencies
            : {},
      })
    : null;

  if (
    managedInstall
    && String(managedInstall.kind || "").trim() === "archive"
    && previousVersion
    && previousVersion !== version
    && !targetRefreshExpected
    && !githubReleaseRefreshed
  ) {
    warnings.push(
      `provider '${providerId}' archive version changed from ${previousVersion} to ${version} without refreshed target artifacts`,
    );
  }

  return {
    id: providerId,
    previousVersion,
    version,
    workspaceSourcePath,
  };
}

function collectIndexRefreshSet({ index, updatesByProvider }) {
  const filteredProviders = (Array.isArray(index?.providers) ? index.providers : []).filter((row) =>
    updatesByProvider.has(String(row?.provider_id || "").trim()),
  );
  if (filteredProviders.length === 0) {
    throw new Error("staged provider index does not contain rows for the requested providers");
  }

  for (const row of filteredProviders) {
    const providerId = String(row?.provider_id || "").trim();
    const expectedVersion = String(updatesByProvider.get(providerId)?.version || "").trim();
    const rowVersion = String(row?.version || "").trim();
    if (expectedVersion && rowVersion && rowVersion !== expectedVersion) {
      throw new Error(
        `staged provider index version mismatch for '${providerId}' (${rowVersion} != ${expectedVersion})`,
      );
    }
  }

  return filteredProviders;
}

function applyIndexRefresh({ matrix, filteredProviders, indexPath, artifactBaseUrl }) {
  const { errors } = applyIndexOverlay({
    matrix,
    index: { providers: filteredProviders },
    indexDir: path.dirname(indexPath),
    artifactBaseUrl,
  });
  if (errors.length > 0) {
    throw new Error(errors.join("\n"));
  }
}

async function applyBumpPlan({ matrix, repoRoot, updates, index, indexPath, artifactBaseUrl, network = createNetworkHooks() }) {
  const updatesByProvider = new Map();
  for (const update of updates) {
    const { providerId } = assertBumpSpec(update);
    if (updatesByProvider.has(providerId)) {
      throw new Error(`provider '${providerId}' was specified more than once`);
    }
    updatesByProvider.set(providerId, { ...update });
  }

  const filteredIndexRows = index ? collectIndexRefreshSet({ index, updatesByProvider }) : [];
  const refreshedProviders = new Set(filteredIndexRows.map((row) => String(row.provider_id || "").trim()));

  const warnings = [];
  const applied = [];
  for (const update of updatesByProvider.values()) {
    applied.push(
      await applyProviderUpdate({
        matrix,
        repoRoot,
        update,
        warnings,
        targetRefreshExpected: refreshedProviders.has(String(update.id || "").trim()),
        network,
      }),
    );
  }

  if (filteredIndexRows.length > 0) {
    applyIndexRefresh({
      matrix,
      filteredProviders: filteredIndexRows,
      indexPath,
      artifactBaseUrl,
    });
  }

  return { applied, warnings };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printHelp();
    return;
  }
  if (!options.updates) {
    throw new Error("--updates is required");
  }

  const updateSpec = readJson(options.updates, "provider update spec");
  const updates = Array.isArray(updateSpec?.providers) ? updateSpec.providers : [];
  if (updates.length === 0) {
    throw new Error("provider update spec must contain a non-empty providers array");
  }

  const matrix = readJson(options.matrix, "provider matrix");
  const index = options.index ? readJson(options.index, "provider deps index") : null;
  const { applied, warnings } = await applyBumpPlan({
    matrix,
    repoRoot: options.repoRoot,
    updates,
    index,
    indexPath: options.index,
    artifactBaseUrl: options.artifactBaseUrl,
  });

  writeJson(options.matrix, matrix);
  for (const item of applied) {
    console.log(`provider:bumped\t${item.id}\t${item.previousVersion || "<unset>"}\t${item.version}`);
    if (item.workspaceSourcePath) {
      console.log(`provider:workspace\t${item.id}\t${item.workspaceSourcePath}`);
    }
  }
  for (const warning of warnings) {
    console.log(`provider:warning\t${warning}`);
  }
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`error: ${error?.message || error}`);
    process.exit(1);
  });
}

module.exports = {
  applyBumpPlan,
  applyCodexProvenance,
  applyGithubReleaseTargetRefresh,
  createNetworkHooks,
  parseArgs,
  parseGithubReleaseRepoFromUrl,
  supportedRelease,
  updateCargoPackageVersion,
  updatePackageJsonFile,
};
