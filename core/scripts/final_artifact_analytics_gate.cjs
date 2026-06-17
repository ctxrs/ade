#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const DEFAULT_EXPECTED_ANALYTICS_ENVIRONMENT = "production";
const DEFAULT_MAX_TEXT_ASSET_BYTES = 25 * 1024 * 1024;
const TEXT_EXTENSIONS = new Set([
  ".cjs",
  ".css",
  ".html",
  ".js",
  ".json",
  ".mjs",
  ".txt",
  ".webmanifest",
]);
const SKIP_DIR_NAMES = new Set([".git", "__MACOSX", "node_modules"]);
const ENVIRONMENTS = new Set(["production", "staging"]);

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const out = {
    artifactRoot: "",
    expectedAnalyticsEnvironment: DEFAULT_EXPECTED_ANALYTICS_ENVIRONMENT,
    expectedVersion: "",
    maxTextAssetBytes: DEFAULT_MAX_TEXT_ASSET_BYTES,
    webDist: "",
  };
  const positional = [];
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--artifact-root" || arg === "--artifact") {
      out.artifactRoot = argv[++i] || "";
      continue;
    }
    if (arg === "--web-dist") {
      out.webDist = argv[++i] || "";
      continue;
    }
    if (arg === "--expected-version" || arg === "--app-version") {
      out.expectedVersion = argv[++i] || "";
      continue;
    }
    if (arg === "--expected-analytics-environment" || arg === "--expected-env") {
      out.expectedAnalyticsEnvironment = String(argv[++i] || "").trim().toLowerCase();
      continue;
    }
    if (arg === "--max-text-asset-bytes") {
      const raw = argv[++i] || "";
      const parsed = Number(raw);
      if (!Number.isSafeInteger(parsed) || parsed <= 0) {
        fail(`invalid --max-text-asset-bytes value: ${raw}`);
      }
      out.maxTextAssetBytes = parsed;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      console.log(
        [
          "Usage: node core/scripts/final_artifact_analytics_gate.cjs --expected-version <version> (--web-dist <dist> | --artifact-root <root>)",
          "",
          "Verifies compiled desktop web assets contain the expected app version and resolve analytics_environment to production.",
          "Use --artifact-root for an extracted AppImage/AppDir root; use --web-dist for a mounted or copied web/dist directory.",
        ].join("\n"),
      );
      process.exit(0);
    }
    if (arg.startsWith("-")) {
      fail(`unknown argument: ${arg}`);
    }
    positional.push(arg);
  }
  if (!out.webDist && !out.artifactRoot && positional.length > 0) {
    out.artifactRoot = positional[0];
  }
  out.expectedVersion = String(out.expectedVersion || "").trim();
  if (!out.expectedVersion) {
    fail("missing required --expected-version");
  }
  if (!ENVIRONMENTS.has(out.expectedAnalyticsEnvironment)) {
    fail(`invalid --expected-analytics-environment: ${out.expectedAnalyticsEnvironment}`);
  }
  if (!out.webDist && !out.artifactRoot) {
    fail("missing --web-dist or --artifact-root");
  }
  return out;
}

function escapeRegex(value) {
  return String(value).replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function normalizePathForMessage(filePath) {
  return filePath.split(path.sep).join("/");
}

function readDirSorted(dirPath) {
  return fs.readdirSync(dirPath, { withFileTypes: true })
    .sort((a, b) => a.name.localeCompare(b.name));
}

function walkFiles(rootDir) {
  const files = [];
  const stack = [path.resolve(rootDir)];
  while (stack.length > 0) {
    const current = stack.pop();
    const stat = fs.lstatSync(current);
    if (stat.isSymbolicLink()) {
      continue;
    }
    if (stat.isDirectory()) {
      for (const entry of readDirSorted(current).reverse()) {
        if (entry.isDirectory() && SKIP_DIR_NAMES.has(entry.name)) {
          continue;
        }
        stack.push(path.join(current, entry.name));
      }
      continue;
    }
    if (stat.isFile()) {
      files.push(current);
    }
  }
  return files.sort();
}

function isTextAssetPath(filePath) {
  if (filePath.endsWith(".map")) {
    return false;
  }
  return TEXT_EXTENSIONS.has(path.extname(filePath).toLowerCase());
}

function hasJavaScriptAsset(webDist) {
  const assetsDir = path.join(webDist, "assets");
  const roots = fs.existsSync(assetsDir) ? [assetsDir] : [webDist];
  return roots.some((root) =>
    fs.existsSync(root)
      && walkFiles(root).some((filePath) => {
        const ext = path.extname(filePath).toLowerCase();
        return ext === ".js" || ext === ".mjs" || ext === ".cjs";
      }),
  );
}

function isWebDistDir(dirPath) {
  return fs.existsSync(path.join(dirPath, "index.html")) && hasJavaScriptAsset(dirPath);
}

function candidateScore(candidate, rootDir) {
  const normalized = normalizePathForMessage(path.resolve(candidate));
  const root = path.resolve(rootDir);
  let score = 0;
  if (path.resolve(candidate) === root) score += 80;
  if (normalized.endsWith("/web/dist")) score += 60;
  if (normalized.includes("/usr/lib/ctx/web/dist")) score += 30;
  if (normalized.includes("/Contents/Resources/web/dist")) score += 30;
  if (fs.existsSync(path.join(candidate, "assets"))) score += 10;
  if (path.basename(candidate) === "dist") score += 5;
  return score;
}

function addCandidate(candidates, candidate) {
  const resolved = path.resolve(candidate);
  if (!fs.existsSync(resolved) || !fs.statSync(resolved).isDirectory()) {
    return;
  }
  if (!isWebDistDir(resolved)) {
    return;
  }
  candidates.set(resolved, resolved);
}

function resolveWebDistCandidates({ artifactRoot = "", webDist = "" }) {
  if (webDist) {
    const resolved = path.resolve(webDist);
    if (!fs.existsSync(resolved) || !fs.statSync(resolved).isDirectory()) {
      throw new Error(`web dist does not exist: ${resolved}`);
    }
    if (!isWebDistDir(resolved)) {
      throw new Error(`web dist is missing index.html or JavaScript assets: ${resolved}`);
    }
    return [resolved];
  }

  const root = path.resolve(artifactRoot);
  if (!fs.existsSync(root) || !fs.statSync(root).isDirectory()) {
    throw new Error(`artifact root does not exist: ${root}`);
  }

  const candidates = new Map();
  for (const candidate of [
    root,
    path.join(root, "web", "dist"),
    path.join(root, "usr", "lib", "ctx", "web", "dist"),
    path.join(root, "Contents", "Resources", "web", "dist"),
  ]) {
    addCandidate(candidates, candidate);
  }

  for (const filePath of walkFiles(root)) {
    if (path.basename(filePath) !== "index.html") {
      continue;
    }
    addCandidate(candidates, path.dirname(filePath));
  }

  return [...candidates.values()].sort((a, b) => {
    const scoreDiff = candidateScore(b, root) - candidateScore(a, root);
    if (scoreDiff !== 0) return scoreDiff;
    return a.localeCompare(b);
  });
}

function collectTextAssets(webDist, { maxTextAssetBytes = DEFAULT_MAX_TEXT_ASSET_BYTES } = {}) {
  const assets = [];
  for (const filePath of walkFiles(webDist)) {
    if (!isTextAssetPath(filePath)) {
      continue;
    }
    const stat = fs.statSync(filePath);
    if (stat.size > maxTextAssetBytes) {
      continue;
    }
    assets.push({
      filePath,
      relativePath: normalizePathForMessage(path.relative(webDist, filePath)),
      text: fs.readFileSync(filePath, "utf8"),
    });
  }
  return assets;
}

function resolveAnalyticsEnvironment(explicitEnv, mode, appVersion) {
  const normalizedMode = String(mode ?? "").trim().toLowerCase();
  if (normalizedMode === "development" || normalizedMode === "dev") {
    return "staging";
  }
  const env = String(explicitEnv ?? "").trim().toLowerCase();
  if (env === "production") return "production";
  if (env === "staging") return "staging";
  if (normalizedMode === "production" && String(appVersion ?? "").trim()) {
    return "production";
  }
  return "staging";
}

function snippetAround(text, index, radius = 180) {
  return text.slice(Math.max(0, index - radius), Math.min(text.length, index + radius))
    .replace(/\s+/g, " ")
    .trim();
}

function directAnalyticsEnvironmentEvidence(asset) {
  const evidence = [];
  const directRe = /(?:^|[,{])\s*(?:"analytics_environment"|'analytics_environment'|analytics_environment)\s*:\s*(["'])(production|staging)\1/g;
  let match;
  while ((match = directRe.exec(asset.text)) !== null) {
    evidence.push({
      environment: match[2],
      file: asset.relativePath,
      kind: "direct-property",
      snippet: snippetAround(asset.text, match.index),
    });
  }
  return evidence;
}

function resolverIdsForAsset(asset) {
  const ids = new Set();
  const resolverRe = /(?:^|[,{])\s*(?:"analytics_environment"|'analytics_environment'|analytics_environment)\s*:\s*([A-Za-z_$][\w$]*)\s*\(/g;
  let match;
  while ((match = resolverRe.exec(asset.text)) !== null) {
    ids.add(match[1]);
  }
  return [...ids];
}

function parseSimpleJsLiteral(rawValue) {
  const value = String(rawValue || "").trim();
  if (value === "void 0" || value === "undefined" || value === "null") {
    return undefined;
  }
  const quote = value[0];
  if ((quote === "\"" || quote === "'") && value[value.length - 1] === quote) {
    return value.slice(1, -1);
  }
  return value;
}

function resolverDefinitionEvidence(asset, resolverId) {
  const id = escapeRegex(resolverId);
  const simpleArgPattern = String.raw`(?:"[^"]*"|'[^']*'|void\s+0|undefined|null)`;
  const callArgPattern = String.raw`([A-Za-z_$][\w$]*)\s*\(\s*(${simpleArgPattern})\s*,\s*(${simpleArgPattern})\s*,\s*(${simpleArgPattern})\s*\)`;
  const definitions = [
    new RegExp(String.raw`${id}\s*=\s*\(\)\s*=>\s*${callArgPattern}`, "g"),
    new RegExp(String.raw`function\s+${id}\s*\(\)\s*\{\s*return\s+${callArgPattern}`, "g"),
  ];
  const directDefinitions = [
    new RegExp(String.raw`${id}\s*=\s*\(\)\s*=>\s*(["'])(production|staging)\1`, "g"),
    new RegExp(String.raw`function\s+${id}\s*\(\)\s*\{\s*return\s+(["'])(production|staging)\1`, "g"),
  ];
  const evidence = [];

  for (const re of definitions) {
    let match;
    while ((match = re.exec(asset.text)) !== null) {
      const explicitEnv = parseSimpleJsLiteral(match[2]);
      const mode = parseSimpleJsLiteral(match[3]);
      const appVersion = parseSimpleJsLiteral(match[4]);
      evidence.push({
        appVersion,
        environment: resolveAnalyticsEnvironment(explicitEnv, mode, appVersion),
        explicitEnv,
        file: asset.relativePath,
        helper: match[1],
        kind: "resolver-call",
        mode,
        resolver: resolverId,
        snippet: snippetAround(asset.text, match.index),
      });
    }
  }

  for (const re of directDefinitions) {
    let match;
    while ((match = re.exec(asset.text)) !== null) {
      evidence.push({
        environment: match[2],
        file: asset.relativePath,
        kind: "resolver-direct-return",
        resolver: resolverId,
        snippet: snippetAround(asset.text, match.index),
      });
    }
  }

  return evidence;
}

function detectAnalyticsEnvironmentEvidence(assets) {
  const evidence = [];
  const unresolved = [];

  for (const asset of assets) {
    evidence.push(...directAnalyticsEnvironmentEvidence(asset));
    for (const resolverId of resolverIdsForAsset(asset)) {
      const resolverEvidence = resolverDefinitionEvidence(asset, resolverId);
      if (resolverEvidence.length > 0) {
        evidence.push(...resolverEvidence);
      } else {
        unresolved.push({ file: asset.relativePath, resolver: resolverId });
      }
    }
  }

  return { evidence, unresolved };
}

function findVersionEvidence(assets, expectedVersion) {
  return assets
    .filter((asset) => asset.text.includes(expectedVersion))
    .map((asset) => asset.relativePath);
}

function formatEvidence(evidence) {
  const detail = [
    `${evidence.kind} in ${evidence.file}`,
    evidence.resolver ? `resolver=${evidence.resolver}` : "",
    evidence.explicitEnv ? `explicit_env=${evidence.explicitEnv}` : "",
    evidence.mode ? `mode=${evidence.mode}` : "",
    evidence.appVersion ? `app_version=${evidence.appVersion}` : "",
  ].filter(Boolean).join(" ");
  return evidence.snippet ? `${detail}: ${evidence.snippet}` : detail;
}

function capturePolicyEvidenceForAsset(asset) {
  const evidence = [];
  let offset = 0;
  while (offset < asset.text.length) {
    const settingsIndex = asset.text.indexOf("settingsLoaded", offset);
    if (settingsIndex === -1) {
      break;
    }
    offset = settingsIndex + "settingsLoaded".length;
    const slice = asset.text.slice(Math.max(0, settingsIndex - 700), settingsIndex + 1700);
    if (!slice.includes("telemetryEnabled")) {
      continue;
    }
    const devOnlyOverride =
      /\bsettingsLoaded\b[^;]{0,220}\btelemetryEnabled\b[^;]{0,180}\?\s*!1\s*:\s*[A-Za-z_$][\w$]*\(\)/.test(slice);
    const normalUserCapture =
      /(?:return|:)\s*!0\b/.test(slice) ||
      /(?:return|:)\s*true\b/.test(slice);
    evidence.push({
      file: asset.relativePath,
      kind: devOnlyOverride ? "dev-only-override" : normalUserCapture ? "normal-user-capture" : "unknown",
      normalUserCapture,
      devOnlyOverride,
      snippet: snippetAround(asset.text, settingsIndex, 260),
    });
  }
  return evidence;
}

function detectCapturePolicyEvidence(assets) {
  return assets.flatMap(capturePolicyEvidenceForAsset);
}

function verifyCapturePolicyEnabled(assets) {
  const evidence = detectCapturePolicyEvidence(assets);
  if (evidence.length === 0) {
    throw new Error("could not find compiled analytics capture policy");
  }
  const devOnly = evidence.find((item) => item.devOnlyOverride);
  if (devOnly) {
    throw new Error(
      `compiled analytics capture policy appears to use only the dev/CI override path; ${formatEvidence(devOnly)}`,
    );
  }
  const enabled = evidence.filter((item) => item.normalUserCapture);
  if (enabled.length === 0) {
    throw new Error("could not prove compiled analytics capture policy allows normal user capture");
  }
  return enabled;
}

function verifyWebDistAnalytics(webDist, {
  expectedAnalyticsEnvironment = DEFAULT_EXPECTED_ANALYTICS_ENVIRONMENT,
  expectedVersion,
  maxTextAssetBytes = DEFAULT_MAX_TEXT_ASSET_BYTES,
} = {}) {
  const assets = collectTextAssets(webDist, { maxTextAssetBytes });
  if (assets.length === 0) {
    throw new Error(`no text assets found in web dist: ${webDist}`);
  }

  const versionEvidence = findVersionEvidence(assets, expectedVersion);
  if (versionEvidence.length === 0) {
    throw new Error(`compiled web assets do not contain expected app version: ${expectedVersion}`);
  }

  const { evidence, unresolved } = detectAnalyticsEnvironmentEvidence(assets);
  const expectedEvidence = evidence.filter((item) => item.environment === expectedAnalyticsEnvironment);
  const wrongEvidence = evidence.filter((item) => item.environment !== expectedAnalyticsEnvironment);
  if (wrongEvidence.length > 0) {
    throw new Error(
      `compiled web assets resolve analytics_environment to ${wrongEvidence[0].environment}, expected ${expectedAnalyticsEnvironment}; ${formatEvidence(wrongEvidence[0])}`,
    );
  }
  if (expectedEvidence.length === 0) {
    const unresolvedMessage = unresolved.length > 0
      ? ` unresolved resolver(s): ${unresolved.map((item) => `${item.file}:${item.resolver}`).join(", ")}`
      : "";
    throw new Error(
      `could not prove compiled web assets resolve analytics_environment to ${expectedAnalyticsEnvironment}.${unresolvedMessage}`,
    );
  }

  const captureEvidence = verifyCapturePolicyEnabled(assets);

  return {
    analyticsEvidenceCount: expectedEvidence.length,
    captureEvidenceCount: captureEvidence.length,
    textAssetCount: assets.length,
    versionEvidence,
    webDist: path.resolve(webDist),
  };
}

function verifyFinalArtifactAnalytics({
  artifactRoot = "",
  expectedAnalyticsEnvironment = DEFAULT_EXPECTED_ANALYTICS_ENVIRONMENT,
  expectedVersion,
  maxTextAssetBytes = DEFAULT_MAX_TEXT_ASSET_BYTES,
  webDist = "",
} = {}) {
  const candidates = resolveWebDistCandidates({ artifactRoot, webDist });
  if (candidates.length === 0) {
    throw new Error(`could not find a compiled web dist under ${path.resolve(artifactRoot || webDist)}`);
  }

  const errors = [];
  for (const candidate of candidates) {
    try {
      return verifyWebDistAnalytics(candidate, {
        expectedAnalyticsEnvironment,
        expectedVersion,
        maxTextAssetBytes,
      });
    } catch (error) {
      errors.push(`${candidate}: ${error?.message ?? error}`);
    }
  }

  throw new Error(errors.join("\n"));
}

function main() {
  const args = parseArgs(process.argv);
  try {
    const result = verifyFinalArtifactAnalytics(args);
    console.log(
      `ok: final artifact analytics verified web_dist=${normalizePathForMessage(result.webDist)} version=${args.expectedVersion} analytics_environment=${args.expectedAnalyticsEnvironment} text_assets=${result.textAssetCount} analytics_evidence=${result.analyticsEvidenceCount} capture_evidence=${result.captureEvidenceCount}`,
    );
  } catch (error) {
    fail(error?.message ?? String(error));
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  collectTextAssets,
  detectCapturePolicyEvidence,
  detectAnalyticsEnvironmentEvidence,
  findVersionEvidence,
  parseArgs,
  resolveAnalyticsEnvironment,
  resolveWebDistCandidates,
  verifyCapturePolicyEnabled,
  verifyFinalArtifactAnalytics,
  verifyWebDistAnalytics,
};
