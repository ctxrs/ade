#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultLockPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles", "runtime_lock.v2.json");
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
    .map((entry) => entry.trim())
    .filter(Boolean);

const parseArgs = (argv) => {
  const opts = {
    lock: defaultLockPath,
    matrix: defaultMatrixPath,
    targets: [...defaultTargets],
    includeKinds: [],
    includeProviders: [],
    strictMissingManagedInstall: true,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--lock") {
      opts.lock = resolveInputPath(argv[i + 1]);
      i += 1;
      continue;
    }
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
    if (arg === "--include-kinds") {
      opts.includeKinds = parseCsv(argv[i + 1]).map((value) => value.toLowerCase());
      i += 1;
      continue;
    }
    if (arg === "--include-providers") {
      opts.includeProviders = parseCsv(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--allow-missing-managed-install") {
      opts.strictMissingManagedInstall = false;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }
  opts.targets = Array.from(new Set(opts.targets.map((value) => value.trim()).filter(Boolean)));
  if (opts.targets.length === 0) {
    opts.targets = [...defaultTargets];
  }
  return opts;
};

const readJson = (filePath, label) => {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse ${label} at ${filePath}: ${error?.message || String(error)}`);
  }
};

const normalizeTargetToken = (value) => String(value || "").trim().toLowerCase().replace(/-/g, "/");
const normalizeTargetOs = (raw) => {
  const value = String(raw || "").trim().toLowerCase();
  if (value === "macos" || value === "darwin") return "darwin";
  if (value === "win32" || value === "windows_nt") return "windows";
  return value;
};

const splitTarget = (value) => {
  const normalized = normalizeTargetToken(value);
  const [osRaw, arch] = normalized.split("/");
  const os = normalizeTargetOs(osRaw);
  if (!os || !arch) return null;
  return { os, arch, normalized: `${os}/${arch}` };
};

const normalizeTargetMap = (targets) => {
  const out = new Set();
  if (!targets || typeof targets !== "object") return out;
  for (const key of Object.keys(targets)) {
    const parsed = splitTarget(key);
    if (!parsed) continue;
    out.add(parsed.normalized);
  }
  return out;
};

const printHelp = () => {
  console.log(
    [
      "Usage: node core/scripts/provider_matrix_required_targets_gate.cjs [options]",
      "",
      "Options:",
      "  --lock <path>                  Runtime lock path",
      "  --matrix <path>                Provider matrix path",
      "  --target <os/arch>             Add required target (repeatable)",
      "  --targets <csv>                Comma-separated required targets",
      "  --include-kinds <csv>          Only validate managed_install kinds (archive,npm,python,...)",
      "  --include-providers <csv>      Only validate specific provider IDs",
      "  --allow-missing-managed-install  Skip hard fail when managed_install is absent",
      "  --help                         Show help",
    ].join("\n"),
  );
};

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    return;
  }

  const lock = readJson(opts.lock, "runtime lock");
  const matrix = readJson(opts.matrix, "provider matrix");
  const requiredProviderIds = Array.isArray(lock?.required?.provider_ids) ? lock.required.provider_ids : [];
  const requiredTargets = opts.targets
    .map((value) => splitTarget(value))
    .filter(Boolean)
    .map((value) => value.normalized);

  const providerById = new Map(
    (Array.isArray(matrix?.providers) ? matrix.providers : [])
      .filter((entry) => entry && typeof entry === "object" && typeof entry.id === "string")
      .map((entry) => [entry.id, entry]),
  );

  const includeProviderSet = new Set(opts.includeProviders);
  const includeKindsSet = new Set(opts.includeKinds);
  const errors = [];
  let checked = 0;

  for (const providerId of requiredProviderIds) {
    if (includeProviderSet.size > 0 && !includeProviderSet.has(providerId)) {
      continue;
    }
    const entry = providerById.get(providerId);
    if (!entry) {
      errors.push(`missing provider matrix entry: ${providerId}`);
      continue;
    }
    const managedInstall =
      entry.managed_install && typeof entry.managed_install === "object" ? entry.managed_install : null;
    if (!managedInstall) {
      if (opts.strictMissingManagedInstall) {
        errors.push(`provider missing managed_install: ${providerId}`);
      }
      continue;
    }
    const kind = String(managedInstall.kind || "").trim().toLowerCase();
    if (includeKindsSet.size > 0 && !includeKindsSet.has(kind)) {
      continue;
    }
    checked += 1;

    const targetSet = normalizeTargetMap(managedInstall.targets);
    const missing = requiredTargets.filter((target) => !targetSet.has(target));
    if (missing.length > 0) {
      errors.push(
        `provider ${providerId} (${kind || "unknown"}) missing target(s): ${missing.join(", ")}`,
      );
    }
  }

  if (errors.length > 0) {
    for (const error of errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }

  console.log(`ok: provider matrix required target gate passed (${checked} providers checked)`);
};

main();
