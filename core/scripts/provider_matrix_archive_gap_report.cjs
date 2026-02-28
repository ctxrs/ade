#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultMatrixPath = path.join(coreRoot, "crates", "ctx-http", "src", "provider_matrix.json");
const defaultTargets = ["darwin/aarch64", "darwin/x86_64", "linux/aarch64", "linux/x86_64"];

const resolveInputPath = (raw) => {
  const value = String(raw || "").trim();
  if (!value) return "";
  if (path.isAbsolute(value)) return value;
  const fromCwd = path.resolve(process.cwd(), value);
  if (fs.existsSync(fromCwd)) return fromCwd;
  return path.resolve(coreRoot, value);
};

const parseCsv = (value) =>
  String(value || "")
    .split(",")
    .map((v) => v.trim())
    .filter(Boolean);

const parseArgs = (argv) => {
  const opts = {
    matrix: defaultMatrixPath,
    targets: [...defaultTargets],
    includeProviders: [],
    strictSizeBytes: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--matrix") {
      opts.matrix = resolveInputPath(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--targets") {
      opts.targets = parseCsv(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--target") {
      const raw = String(argv[i + 1] || "").trim();
      if (raw) opts.targets.push(raw);
      i += 1;
      continue;
    }
    if (arg === "--include-providers") {
      opts.includeProviders = parseCsv(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--strict-size-bytes") {
      opts.strictSizeBytes = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  opts.targets = Array.from(new Set(opts.targets.map((value) => value.trim()).filter(Boolean)));
  if (opts.targets.length === 0) opts.targets = [...defaultTargets];
  return opts;
};

const normalizeTarget = (value) => {
  const normalized = String(value || "").trim().toLowerCase().replace(/-/g, "/");
  const [osRaw, arch] = normalized.split("/");
  if (!osRaw || !arch) return null;
  const os = osRaw === "macos" ? "darwin" : osRaw;
  return `${os}/${arch}`;
};

const targetKeyForMatrix = (target) => {
  const [os, arch] = target.split("/");
  return `${os}-${arch}`;
};

const readJson = (filePath) => {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/provider_matrix_archive_gap_report.cjs [options]",
    "",
    "Options:",
    "  --matrix <path>             Provider matrix JSON path",
    "  --target <os/arch>          Add required archive target (repeatable)",
    "  --targets <csv>             Required archive targets CSV",
    "  --include-providers <csv>   Restrict check to selected provider IDs",
    "  --strict-size-bytes          Treat missing size_bytes as error",
    "  --help                      Show help",
  ].join("\n"));
};

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    return;
  }

  const includeSet = new Set(opts.includeProviders);
  const requiredTargets = opts.targets.map(normalizeTarget).filter(Boolean);
  const matrix = readJson(opts.matrix);
  const providers = Array.isArray(matrix?.providers) ? matrix.providers : [];

  const rows = [];
  const errors = [];
  const warnings = [];

  for (const provider of providers) {
    if (!provider || typeof provider !== "object") continue;
    const providerId = String(provider.id || "").trim();
    if (!providerId) continue;
    if (includeSet.size > 0 && !includeSet.has(providerId)) continue;

    const managedInstall = provider.managed_install && typeof provider.managed_install === "object"
      ? provider.managed_install
      : null;
    const kind = String(managedInstall?.kind || "").trim().toLowerCase();
    if (kind !== "archive") continue;

    const targets = managedInstall.targets && typeof managedInstall.targets === "object"
      ? managedInstall.targets
      : {};

    const missingTargets = [];
    const missingSha = [];
    const missingSize = [];
    const invalidEntries = [];

    for (const target of requiredTargets) {
      const key = targetKeyForMatrix(target);
      const entry = targets[key];
      if (!entry || typeof entry !== "object") {
        missingTargets.push(target);
        continue;
      }
      const url = String(entry.url || "").trim();
      const archive = String(entry.archive || "").trim();
      const binPath = String(entry.bin_path || "").trim();
      const sha = String(entry.sha256 || "").trim();
      const sizeBytes = Number(entry.size_bytes);

      if (!url || !archive || !binPath) {
        invalidEntries.push(`${target} (missing url/archive/bin_path)`);
      }
      if (!/^[0-9a-f]{64}$/i.test(sha)) {
        missingSha.push(target);
      }
      if (!Number.isFinite(sizeBytes) || sizeBytes <= 0) {
        missingSize.push(target);
      }
    }

    rows.push({
      provider_id: providerId,
      missing_targets: missingTargets,
      missing_sha256: missingSha,
      missing_size_bytes: missingSize,
      invalid_entries: invalidEntries,
    });

    if (missingTargets.length > 0) {
      errors.push(`provider ${providerId} missing targets: ${missingTargets.join(", ")}`);
    }
    if (missingSha.length > 0) {
      errors.push(`provider ${providerId} missing/invalid sha256: ${missingSha.join(", ")}`);
    }
    if (invalidEntries.length > 0) {
      errors.push(`provider ${providerId} invalid entries: ${invalidEntries.join(", ")}`);
    }
    if (missingSize.length > 0) {
      const msg = `provider ${providerId} missing size_bytes: ${missingSize.join(", ")}`;
      if (opts.strictSizeBytes) {
        errors.push(msg);
      } else {
        warnings.push(msg);
      }
    }
  }

  console.log(`archive providers checked: ${rows.length}`);
  console.log(`required targets: ${requiredTargets.join(", ")}`);
  console.log("provider_id\tmissing_targets\tmissing_sha256\tmissing_size_bytes");
  for (const row of rows) {
    const t = row.missing_targets.length > 0 ? row.missing_targets.join(",") : "-";
    const s = row.missing_sha256.length > 0 ? row.missing_sha256.join(",") : "-";
    const z = row.missing_size_bytes.length > 0 ? row.missing_size_bytes.join(",") : "-";
    console.log(`${row.provider_id}\t${t}\t${s}\t${z}`);
  }

  for (const warning of warnings) {
    console.warn(`warn: ${warning}`);
  }
  if (errors.length > 0) {
    for (const error of errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }
  console.log("ok: provider matrix archive gap report passed");
};

main();
