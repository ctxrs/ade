#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultMatrixPath = path.join(coreRoot, "crates", "ctx-provider-accounts", "src", "provider_matrix.json");

function trimValue(value) {
  return String(value || "").trim();
}

function resolveInputPath(raw) {
  const value = trimValue(raw);
  if (!value) return "";
  if (path.isAbsolute(value)) return value;
  const fromCwd = path.resolve(process.cwd(), value);
  if (fs.existsSync(fromCwd)) {
    return fromCwd;
  }
  return path.resolve(coreRoot, value);
}

function resolveOutputPath(raw) {
  const value = trimValue(raw);
  if (!value) return "";
  if (path.isAbsolute(value)) return value;
  return path.resolve(process.cwd(), value);
}

function parseArgs(argv) {
  const options = {
    channel: "",
    matrixPath: defaultMatrixPath,
    outPath: "",
    sourceCommit: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--matrix") {
      options.matrixPath = resolveInputPath(argv[++index] || "");
      continue;
    }
    if (arg === "--out") {
      options.outPath = resolveOutputPath(argv[++index] || "");
      continue;
    }
    if (arg === "--source-commit") {
      options.sourceCommit = trimValue(argv[++index] || "");
      continue;
    }
    if (arg === "--channel") {
      options.channel = trimValue(argv[++index] || "");
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }
  return options;
}

function printUsage() {
  console.log(`usage: node core/scripts/write_resolved_provider_manifest.cjs [options]

options:
  --matrix <path>         Base provider matrix JSON (default: checked-in provider_matrix.json)
  --source-commit <sha>   Source commit this manifest resolves for
  --channel <name>        Release channel metadata to stamp into the manifest
  --out <path>            Output path (required)
`);
}

function readJson(filePath, label) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse ${label} at ${filePath}: ${error?.message || String(error)}`);
  }
}

function stableStringify(value) {
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableStringify(entry)).join(",")}]`;
  }
  if (!value || typeof value !== "object") {
    return JSON.stringify(value);
  }
  return `{${Object.keys(value)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
    .join(",")}}`;
}

function buildResolvedProviderManifest({
  channel = "",
  matrix,
  matrixPath = "",
  sourceCommit = "",
} = {}) {
  const normalizedCommit = trimValue(sourceCommit);
  if (!normalizedCommit) {
    throw new Error("sourceCommit is required");
  }
  const base = {
    ...matrix,
    provider_manifest_version: 1,
    source_commit: normalizedCommit,
    release_channel: trimValue(channel),
    source_matrix_path: path.basename(matrixPath || ""),
    generated_at: new Date().toISOString(),
  };
  const digestPayload = {
    ...base,
    generated_at: undefined,
    manifest_id: undefined,
  };
  const digest = crypto.createHash("sha256").update(stableStringify(digestPayload)).digest("hex");
  return {
    ...base,
    manifest_id: `sha256:${digest}`,
    manifest_sha256: digest,
  };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printUsage();
    return;
  }
  if (!options.outPath) {
    throw new Error("--out is required");
  }
  if (!options.sourceCommit) {
    throw new Error("--source-commit is required");
  }
  const matrix = readJson(options.matrixPath, "provider matrix");
  const resolvedManifest = buildResolvedProviderManifest({
    channel: options.channel,
    matrix,
    matrixPath: options.matrixPath,
    sourceCommit: options.sourceCommit,
  });
  fs.mkdirSync(path.dirname(options.outPath), { recursive: true });
  fs.writeFileSync(options.outPath, `${JSON.stringify(resolvedManifest, null, 2)}\n`, "utf8");
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(error?.stack || error?.message || String(error));
    process.exit(1);
  }
}

module.exports = {
  buildResolvedProviderManifest,
  parseArgs,
  stableStringify,
};
