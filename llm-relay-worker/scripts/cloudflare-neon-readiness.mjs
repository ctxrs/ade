#!/usr/bin/env node

import childProcess from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const PACKAGE_ROOT = path.resolve(SCRIPT_DIR, "..");
const REPO_ROOT = path.resolve(PACKAGE_ROOT, "..");
const CORE_ROOT = path.join(REPO_ROOT, "core");
const DEFAULT_WRANGLER_CONFIG = path.join(PACKAGE_ROOT, "wrangler.toml");
const DEFAULT_INFISICAL_ENV = "prod";
const DEFAULT_INFISICAL_PATH = "/";
const CLOUDFLARE_API_BASE = "https://api.cloudflare.com/client/v4";
const NEON_API_BASE = "https://console.neon.tech/api/v2";

const COMMON_REQUIRED_ENV = [
  {
    group: "cloudflare_operator",
    name: "CLOUDFLARE_ACCOUNT_ID",
    aliases: ["CF_ACCOUNT_ID"],
    secret: false,
  },
  {
    group: "cloudflare_operator",
    name: "CLOUDFLARE_API_TOKEN",
    aliases: ["CF_API_TOKEN"],
    secret: true,
  },
  {
    group: "neon_operator",
    name: "NEON_API_KEY",
    aliases: ["CTX_NEON_API_KEY"],
    secret: true,
  },
  {
    group: "neon_operator",
    name: "NEON_PROJECT_ID",
    aliases: ["CTX_NEON_PROJECT_ID"],
    secret: false,
  },
];

const RELAY_REQUIRED_ENV = [
  ...COMMON_REQUIRED_ENV,
  {
    group: "relay_authority",
    name: "RELAY_AUTHORITY_DATABASE_URL",
    aliases: ["CTX_RELAY_AUTHORITY_DATABASE_URL", "CTX_NEON_PROD_RELAY_DATABASE_URL", "NEON_DATABASE_URL"],
    secret: true,
  },
  {
    group: "relay_authority",
    name: "RELAY_AUTHORITY_BEARER_TOKEN",
    aliases: [],
    secret: true,
  },
  {
    group: "relay_authority",
    name: "RELAY_AUTHORITY_CONTROL_PLANE_JWKS",
    aliases: [],
    secret: true,
  },
  {
    group: "worker_runtime",
    name: "AUTHORITY_BEARER_TOKEN",
    aliases: [],
    secret: true,
  },
  {
    group: "worker_runtime",
    name: "CONTROL_PLANE_JWKS",
    aliases: [],
    secret: true,
  },
  {
    group: "worker_runtime",
    name: "OPENAI_API_KEY",
    aliases: [],
    secret: true,
  },
];

const RELEASE_API_REQUIRED_ENV = [
  ...COMMON_REQUIRED_ENV,
  {
    group: "release_storage",
    name: "CTX_RELEASES_R2_BUCKET",
    aliases: ["RELEASE_STORAGE_BUCKET", "CTX_RELEASE_R2_BUCKET"],
    secret: false,
  },
  {
    group: "release_storage",
    name: "CTX_RELEASE_STAGING_R2_BUCKET",
    aliases: [],
    secret: false,
  },
  {
    group: "release_storage",
    name: "CTX_RELEASE_STORAGE_BACKEND",
    aliases: ["RELEASE_STORAGE_PROVIDER"],
    secret: false,
  },
];

const TELEMETRY_REQUIRED_ENV = [
  ...COMMON_REQUIRED_ENV,
  {
    group: "telemetry_storage",
    name: "TELEMETRY_DATABASE_URL",
    aliases: ["CTX_TELEMETRY_DATABASE_URL", "CTX_NEON_PROD_TELEMETRY_DATABASE_URL"],
    secret: true,
  },
  {
    group: "telemetry_storage",
    name: "INSTALL_ID_HASH_SALT",
    aliases: [],
    secret: true,
  },
  {
    group: "telemetry_mirror",
    name: "POSTHOG_CANARY_PROJECT_API_KEY",
    aliases: [],
    secret: true,
  },
];

const RELAY_REQUIRED_WORKER_VARS = [
  "AUTHORITY_BASE_URL",
  "ENVIRONMENT",
  "GRANT_VERIFICATION_MODE",
  "OPENAI_RESPONSES_URL",
];

const RELEASE_API_REQUIRED_WORKER_VARS = [
  "RELEASE_ARTIFACT_REDIRECT_BASE_URL",
];

const TELEMETRY_REQUIRED_WORKER_VARS = [
  "POSTHOG_HOST",
];

const RELAY_REQUIRED_WORKER_SECRETS = [
  "AUTHORITY_BEARER_TOKEN",
  "CONTROL_PLANE_JWKS",
  "OPENAI_API_KEY",
];

const RELEASE_API_REQUIRED_WORKER_SECRETS = [];

const TELEMETRY_REQUIRED_WORKER_SECRETS = [
  "INSTALL_ID_HASH_SALT",
  "POSTHOG_CANARY_PROJECT_API_KEY",
  "TELEMETRY_DATABASE_URL",
];

const AUTHORITY_TABLES = [
  "billing_spend_limits",
  "credit_grants",
  "credit_ledger_events",
  "model_prices",
  "pricing_catalog_versions",
  "request_state_events",
  "route_configs",
  "route_policy_versions",
  "usage_ledger_events",
  "usage_reservation_allocations",
  "usage_reservations",
];

function usage() {
  return `usage: node scripts/cloudflare-neon-readiness.mjs [options]

Safe Cloudflare + Neon migration readiness check for the LLM relay.

Default behavior is read-only:
  - verify required env/Infisical keys are present without printing values
  - parse wrangler.toml for required Worker vars
  - inspect Cloudflare Worker and script-level secret names when credentials exist
  - inspect Neon project metadata when credentials exist
  - skip live Postgres role/schema checks unless explicitly requested

Options:
  --environment <name>       Worker environment to evaluate (repeatable; default: staging, prod)
  --profile <name>           Readiness profile: relay, release-api, or telemetry (default: relay)
  --worker-name <name>       Cloudflare Worker script name (default: wrangler.toml name)
  --wrangler-config <path>   Wrangler config path (default: llm-relay-worker/wrangler.toml)
  --infisical-env <env>      Infisical environment for local fallback (default: INFISICAL_ENV or prod)
  --infisical-path <path>    Infisical path for local fallback (default: INFISICAL_PATH or /)
  --infisical-project-dir <path>
                             Directory containing .infisical.json (default: core/)
  --no-infisical             Do not query Infisical for missing env values
  --skip-cloudflare          Do not call Cloudflare APIs
  --skip-neon-api            Do not call Neon APIs
  --check-neon-roles         Verify Postgres role attributes and authority tables with psql
  --ensure-neon-roles        Create/update the runtime DB role from RELAY_AUTHORITY_DATABASE_URL
  --mutate                   Required with --ensure-neon-roles; never implied
  --help                     Show this help
`;
}

function parseArgs(argv) {
  const options = {
    environments: [],
    profile: "relay",
    workerName: "",
    wranglerConfig: DEFAULT_WRANGLER_CONFIG,
    infisicalEnv: "",
    infisicalPath: "",
    infisicalProjectDir: CORE_ROOT,
    useInfisical: true,
    inspectCloudflare: true,
    inspectNeonApi: true,
    checkNeonRoles: false,
    ensureNeonRoles: false,
    mutate: false,
    help: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    if (arg === "--environment") {
      options.environments.push(requireArg(argv, ++index, arg));
      continue;
    }
    if (arg === "--profile") {
      options.profile = requireArg(argv, ++index, arg);
      continue;
    }
    if (arg === "--worker-name") {
      options.workerName = requireArg(argv, ++index, arg);
      continue;
    }
    if (arg === "--wrangler-config") {
      options.wranglerConfig = path.resolve(requireArg(argv, ++index, arg));
      continue;
    }
    if (arg === "--infisical-env") {
      options.infisicalEnv = requireArg(argv, ++index, arg);
      continue;
    }
    if (arg === "--infisical-path") {
      options.infisicalPath = requireArg(argv, ++index, arg);
      continue;
    }
    if (arg === "--infisical-project-dir") {
      options.infisicalProjectDir = path.resolve(requireArg(argv, ++index, arg));
      continue;
    }
    if (arg === "--no-infisical") {
      options.useInfisical = false;
      continue;
    }
    if (arg === "--skip-cloudflare") {
      options.inspectCloudflare = false;
      continue;
    }
    if (arg === "--skip-neon-api") {
      options.inspectNeonApi = false;
      continue;
    }
    if (arg === "--check-neon-roles") {
      options.checkNeonRoles = true;
      continue;
    }
    if (arg === "--ensure-neon-roles") {
      options.ensureNeonRoles = true;
      options.checkNeonRoles = true;
      continue;
    }
    if (arg === "--mutate") {
      options.mutate = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  options.environments = normalizeList(options.environments.length > 0 ? options.environments : ["staging", "prod"]);
  options.infisicalEnv = options.infisicalEnv || process.env.INFISICAL_ENV || DEFAULT_INFISICAL_ENV;
  options.infisicalPath = options.infisicalPath || process.env.INFISICAL_PATH || DEFAULT_INFISICAL_PATH;
  if (!["relay", "release-api", "telemetry"].includes(options.profile)) {
    throw new Error(`unsupported --profile '${options.profile}'`);
  }
  if (options.ensureNeonRoles && !options.mutate) {
    throw new Error("--ensure-neon-roles requires --mutate");
  }
  return options;
}

function requireArg(argv, index, flag) {
  const value = argv[index];
  if (!value || value.startsWith("--")) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

function normalizeList(values) {
  return Array.from(new Set(values.map((value) => value.trim()).filter(Boolean))).sort();
}

async function buildReadinessReport({ argv = [], env = process.env, fetchImpl = globalThis.fetch, spawnSync = childProcess.spawnSync } = {}) {
  const options = parseArgs(argv);
  const profile = readinessProfile(options.profile);
  const wrangler = readWranglerConfig(options.wranglerConfig);
  const workerName = options.workerName || wrangler.name || "ctx-llm-relay";
  const envResolution = resolveRequiredEnv({
    env,
    requiredEnv: profile.requiredEnv,
    useInfisical: options.useInfisical,
    infisicalEnv: options.infisicalEnv,
    infisicalPath: options.infisicalPath,
    infisicalProjectDir: options.infisicalProjectDir,
    spawnSync,
  });
  const values = envResolution.values;
  const wranglerReadiness = buildWranglerReadiness(
    wrangler,
    options.environments,
    profile.requiredWorkerVars,
    profile.requiredWorkerSecrets,
  );
  const cloudflare = options.inspectCloudflare
    ? await inspectCloudflare({
        accountId: getResolvedValue(values, "CLOUDFLARE_ACCOUNT_ID"),
        apiToken: getResolvedValue(values, "CLOUDFLARE_API_TOKEN"),
        requiredWorkerSecrets: profile.requiredWorkerSecrets,
        workerName,
        fetchImpl,
      })
    : skipped("disabled_by_flag");
  const neonApi = options.inspectNeonApi
    ? await inspectNeonApi({
        apiKey: getResolvedValue(values, "NEON_API_KEY"),
        projectId: getResolvedValue(values, "NEON_PROJECT_ID"),
        fetchImpl,
      })
    : skipped("disabled_by_flag");
  const neonDatabase = profile.inspectNeonDatabase
    ? inspectNeonDatabase({
        authorityDatabaseUrl: getResolvedValue(values, "RELAY_AUTHORITY_DATABASE_URL"),
        adminDatabaseUrl: firstEnvValue(env, ["NEON_ADMIN_DATABASE_URL", "RELAY_AUTHORITY_ADMIN_DATABASE_URL"]),
        checkRoles: options.checkNeonRoles,
        ensureRoles: options.ensureNeonRoles,
        mutate: options.mutate,
        spawnSync,
      })
    : skipped("disabled_by_profile");

  const unresolved = [
    ...wranglerReadiness.unresolved,
    ...cloudflare.unresolved,
    ...neonApi.unresolved,
    ...neonDatabase.unresolved,
  ].sort();
  const failures = [
    ...envResolution.report.missing.map((name) => `missing_env:${name}`),
    ...wranglerReadiness.failures,
    ...cloudflare.failures,
    ...neonApi.failures,
    ...neonDatabase.failures,
  ].sort();

  return {
    schema_version: 1,
    ok: failures.length === 0,
    failures,
    mode: {
      cloudflare: options.inspectCloudflare ? "read_only" : "skipped",
      mutate: options.mutate,
      neon_api: options.inspectNeonApi ? "read_only" : "skipped",
      neon_roles: options.ensureNeonRoles
        ? "mutating_ensure"
        : options.checkNeonRoles
          ? "read_only"
          : "skipped",
      profile: options.profile,
    },
    env: envResolution.report,
    wrangler: wranglerReadiness.report,
    cloudflare: cloudflare.report,
    neon: {
      api: neonApi.report,
      database: neonDatabase.report,
    },
    unresolved,
  };
}

function readinessProfile(profile) {
  if (profile === "release-api") {
    return {
      inspectNeonDatabase: false,
      requiredEnv: RELEASE_API_REQUIRED_ENV,
      requiredWorkerSecrets: RELEASE_API_REQUIRED_WORKER_SECRETS,
      requiredWorkerVars: RELEASE_API_REQUIRED_WORKER_VARS,
    };
  }
  if (profile === "telemetry") {
    return {
      inspectNeonDatabase: false,
      requiredEnv: TELEMETRY_REQUIRED_ENV,
      requiredWorkerSecrets: TELEMETRY_REQUIRED_WORKER_SECRETS,
      requiredWorkerVars: TELEMETRY_REQUIRED_WORKER_VARS,
    };
  }
  return {
    inspectNeonDatabase: true,
    requiredEnv: RELAY_REQUIRED_ENV,
    requiredWorkerSecrets: RELAY_REQUIRED_WORKER_SECRETS,
    requiredWorkerVars: RELAY_REQUIRED_WORKER_VARS,
  };
}

function resolveRequiredEnv({ env, requiredEnv, useInfisical, infisicalEnv, infisicalPath, infisicalProjectDir, spawnSync }) {
  const values = new Map();
  const required = [];
  const missing = [];
  const lookups = [];
  for (const spec of requiredEnv) {
    const resolution = resolveEnvSpec(spec, {
      env,
      useInfisical,
      infisicalEnv,
      infisicalPath,
      infisicalProjectDir,
      spawnSync,
    });
    if (resolution.value) {
      values.set(spec.name, resolution.value);
    } else {
      missing.push(spec.name);
    }
    required.push({
      aliases: spec.aliases,
      group: spec.group,
      name: spec.name,
      present: Boolean(resolution.value),
      secret: spec.secret,
      source: resolution.source,
    });
    if (resolution.lookup) {
      lookups.push(resolution.lookup);
    }
  }
  const equality = compareResolvedSecrets(values);
  return {
    values,
    report: {
      infisical: {
        enabled: useInfisical,
        env: infisicalEnv,
        path: infisicalPath,
        project_dir: relativize(infisicalProjectDir),
      },
      required,
      missing: missing.sort(),
      equality,
      lookups: lookups.sort((left, right) => left.name.localeCompare(right.name)),
    },
  };
}

function resolveEnvSpec(spec, { env, useInfisical, infisicalEnv, infisicalPath, infisicalProjectDir, spawnSync }) {
  for (const key of [spec.name, ...spec.aliases]) {
    const value = String(env[key] || "").trim();
    if (value) {
      return { value, source: key === spec.name ? "env" : `env:${key}` };
    }
  }
  if (!useInfisical) {
    return { value: "", source: "missing" };
  }
  if (isBuildkiteCi(env)) {
    return {
      value: "",
      source: "missing",
      lookup: {
        name: spec.name,
        source: "infisical",
        status: "skipped_in_buildkite",
      },
    };
  }
  let lastLookup = null;
  for (const key of [spec.name, ...spec.aliases]) {
    const lookup = readInfisicalSecret(key, {
      env,
      infisicalEnv,
      infisicalPath,
      infisicalProjectDir,
      spawnSync,
    });
    lastLookup = lookup;
    if (lookup.value) {
      return {
        value: lookup.value,
        source: key === spec.name ? "infisical" : `infisical:${key}`,
        lookup: {
          name: key,
          source: "infisical",
          status: "present",
        },
      };
    }
  }
  return {
    value: "",
    source: "missing",
    lookup: {
      name: spec.name,
      source: "infisical",
      status: lastLookup?.status || "missing",
    },
  };
}

function readInfisicalSecret(name, { env, infisicalEnv, infisicalPath, infisicalProjectDir, spawnSync }) {
  const args = ["secrets", "get", name, "--plain", "--env", infisicalEnv];
  if (infisicalPath) {
    args.push("--path", infisicalPath);
  }
  const result = spawnSync("infisical", args, {
    cwd: infisicalProjectDir,
    encoding: "utf8",
    env,
  });
  if (result.error) {
    return { value: "", status: "cli_unavailable" };
  }
  if (result.status !== 0) {
    return { value: "", status: "lookup_failed" };
  }
  const value = String(result.stdout || "").trim();
  return { value, status: value ? "present" : "empty" };
}

function isBuildkiteCi(env) {
  return Boolean(String(env.BUILDKITE || env.BUILDKITE_BUILD_ID || "").trim());
}

function compareResolvedSecrets(values) {
  return [
    compareSecretPair(values, "AUTHORITY_BEARER_TOKEN", "RELAY_AUTHORITY_BEARER_TOKEN"),
    compareSecretPair(values, "CONTROL_PLANE_JWKS", "RELAY_AUTHORITY_CONTROL_PLANE_JWKS"),
  ].sort((left, right) => left.names.join(":").localeCompare(right.names.join(":")));
}

function compareSecretPair(values, left, right) {
  const leftValue = getResolvedValue(values, left);
  const rightValue = getResolvedValue(values, right);
  return {
    names: [left, right].sort(),
    comparable: Boolean(leftValue && rightValue),
    same_value: Boolean(leftValue && rightValue && leftValue === rightValue),
  };
}

function getResolvedValue(values, name) {
  return values.get(name) || "";
}

function firstEnvValue(env, names) {
  for (const name of names) {
    const value = String(env[name] || "").trim();
    if (value) return value;
  }
  return "";
}

function readWranglerConfig(configPath) {
  const text = fs.readFileSync(configPath, "utf8");
  return parseWranglerToml(text, configPath);
}

function parseWranglerToml(text, configPath = "wrangler.toml") {
  const root = { name: "", vars: {}, env: {} };
  let section = [];
  for (const rawLine of text.split(/\r?\n/u)) {
    const line = stripTomlComment(rawLine).trim();
    if (!line) continue;
    const sectionMatch = line.match(/^\[([^\]]+)\]$/u);
    if (sectionMatch) {
      section = sectionMatch[1].split(".");
      continue;
    }
    const assignment = line.match(/^([A-Za-z0-9_-]+)\s*=\s*(.+)$/u);
    if (!assignment) continue;
    const key = assignment[1];
    const value = parseTomlScalar(assignment[2].trim());
    if (section.length === 0) {
      if (key === "name" && typeof value === "string") {
        root.name = value;
      }
      continue;
    }
    if (section.length === 1 && section[0] === "vars") {
      root.vars[key] = value;
      continue;
    }
    if (section.length === 3 && section[0] === "env" && section[2] === "vars") {
      const envName = section[1];
      root.env[envName] = root.env[envName] || { vars: {} };
      root.env[envName].vars[key] = value;
    }
  }
  return { ...root, path: configPath };
}

function stripTomlComment(line) {
  let inString = false;
  let quote = "";
  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];
    if ((char === "\"" || char === "'") && line[index - 1] !== "\\") {
      if (!inString) {
        inString = true;
        quote = char;
      } else if (quote === char) {
        inString = false;
        quote = "";
      }
    }
    if (char === "#" && !inString) {
      return line.slice(0, index);
    }
  }
  return line;
}

function parseTomlScalar(value) {
  if ((value.startsWith("\"") && value.endsWith("\"")) || (value.startsWith("'") && value.endsWith("'"))) {
    return value.slice(1, -1);
  }
  if (value === "true") return true;
  if (value === "false") return false;
  return value;
}

function buildWranglerReadiness(wrangler, environments, requiredWorkerVars, requiredWorkerSecrets) {
  const failures = [];
  const unresolved = [];
  const envReports = environments.map((environment) => {
    const vars = wrangler.env[environment]?.vars || {};
    const missing = requiredWorkerVars.filter((name) => !(name in vars)).sort();
    if (missing.length > 0) {
      failures.push(`wrangler_missing_vars:${environment}:${missing.join(",")}`);
    }
    return {
      name: environment,
      present_vars: Object.keys(vars).sort(),
      missing_vars: missing,
      required_vars: [...requiredWorkerVars].sort(),
    };
  });

  if (requiredWorkerSecrets.length > 0) {
    unresolved.push(
      "worker runtime secrets are inspected by Cloudflare script-level secret names; per-environment secret binding inspection still needs release-process confirmation",
    );
  }
  return {
    failures,
    unresolved,
    report: {
      path: relativize(wrangler.path),
      worker_name: wrangler.name || "",
      root_vars: Object.keys(wrangler.vars).sort(),
      environments: envReports,
      required_secrets: [...requiredWorkerSecrets].sort(),
    },
  };
}

async function inspectCloudflare({ accountId, apiToken, requiredWorkerSecrets, workerName, fetchImpl }) {
  const report = {
    checked: false,
    worker_name: workerName,
    worker_exists: null,
    script_secrets: {
      checked: false,
      present: [],
      missing: [...requiredWorkerSecrets].sort(),
      required: [...requiredWorkerSecrets].sort(),
    },
  };
  const failures = [];
  const unresolved = [];
  if (!accountId || !apiToken) {
    report.skip_reason = "missing_cloudflare_credentials";
    unresolved.push("Cloudflare API inspection requires CLOUDFLARE_ACCOUNT_ID and CLOUDFLARE_API_TOKEN");
    return { report, failures, unresolved };
  }
  report.checked = true;
  const scripts = await cloudflareGet(`/accounts/${encodeURIComponent(accountId)}/workers/scripts`, apiToken, fetchImpl);
  if (!scripts.ok) {
    failures.push(`cloudflare_workers_list:${scripts.status}`);
    report.error = scripts.error;
    return { report, failures, unresolved };
  }
  const scriptNames = parseCloudflareWorkerNames(scripts.result);
  report.worker_exists = scriptNames.includes(workerName);
  if (!report.worker_exists) {
    failures.push(`cloudflare_worker_missing:${workerName}`);
  }
  if (requiredWorkerSecrets.length === 0) {
    report.script_secrets.checked = false;
    return { report, failures, unresolved };
  }

  const secrets = await cloudflareGet(
    `/accounts/${encodeURIComponent(accountId)}/workers/scripts/${encodeURIComponent(workerName)}/secrets`,
    apiToken,
    fetchImpl,
  );
  report.script_secrets.checked = true;
  if (!secrets.ok) {
    failures.push(`cloudflare_worker_secrets:${secrets.status}`);
    report.script_secrets.error = secrets.error;
    return { report, failures, unresolved };
  }
  const secretNames = parseCloudflareSecretNames(secrets.result);
  const present = requiredWorkerSecrets.filter((name) => secretNames.includes(name)).sort();
  const missing = requiredWorkerSecrets.filter((name) => !secretNames.includes(name)).sort();
  report.script_secrets.present = present;
  report.script_secrets.missing = missing;
  for (const name of missing) {
    failures.push(`cloudflare_worker_secret_missing:${name}`);
  }
  return { report, failures, unresolved };
}

async function cloudflareGet(pathname, apiToken, fetchImpl) {
  try {
    const response = await fetchImpl(`${CLOUDFLARE_API_BASE}${pathname}`, {
      headers: {
        authorization: `Bearer ${apiToken}`,
        "content-type": "application/json",
      },
      method: "GET",
    });
    const body = await safeJson(response);
    if (!response.ok || body?.success === false) {
      return {
        ok: false,
        status: response.status,
        error: summarizeApiErrors(body),
      };
    }
    return { ok: true, status: response.status, result: body?.result ?? body };
  } catch {
    return { ok: false, status: "network_error", error: "request_failed" };
  }
}

function parseCloudflareWorkerNames(result) {
  const entries = Array.isArray(result) ? result : Array.isArray(result?.items) ? result.items : [];
  return entries
    .map((item) => String(item?.id || item?.name || "").trim())
    .filter(Boolean)
    .sort();
}

function parseCloudflareSecretNames(result) {
  const entries = Array.isArray(result) ? result : Array.isArray(result?.items) ? result.items : [];
  return entries
    .map((item) => String(item?.name || "").trim())
    .filter(Boolean)
    .sort();
}

async function inspectNeonApi({ apiKey, projectId, fetchImpl }) {
  const report = {
    checked: false,
    project_id_present: Boolean(projectId),
    project_exists: null,
  };
  const failures = [];
  const unresolved = [];
  if (!apiKey || !projectId) {
    report.skip_reason = "missing_neon_api_credentials";
    unresolved.push("Neon API inspection requires NEON_API_KEY and NEON_PROJECT_ID");
    return { report, failures, unresolved };
  }
  report.checked = true;
  try {
    const response = await fetchImpl(`${NEON_API_BASE}/projects/${encodeURIComponent(projectId)}`, {
      headers: {
        accept: "application/json",
        authorization: `Bearer ${apiKey}`,
      },
      method: "GET",
    });
    const body = await safeJson(response);
    if (!response.ok) {
      failures.push(`neon_project:${response.status}`);
      report.error = summarizeApiErrors(body);
      return { report, failures, unresolved };
    }
    report.project_exists = Boolean(body?.project?.id || body?.id || body?.project);
    if (!report.project_exists) {
      failures.push("neon_project_missing");
    }
  } catch {
    failures.push("neon_project:network_error");
    report.error = "request_failed";
  }
  return { report, failures, unresolved };
}

function inspectNeonDatabase({ authorityDatabaseUrl, adminDatabaseUrl, checkRoles, ensureRoles, mutate, spawnSync }) {
  const failures = [];
  const unresolved = [];
  const report = {
    checked: checkRoles,
    authority_database_url_present: Boolean(authorityDatabaseUrl),
    admin_database_url_present: Boolean(adminDatabaseUrl),
    authority_database: summarizeDatabaseUrl(authorityDatabaseUrl),
    roles: [],
    tables: [],
  };

  if (!authorityDatabaseUrl) {
    failures.push("missing_env:RELAY_AUTHORITY_DATABASE_URL");
    return { report, failures, unresolved };
  }

  const authorityUrl = parseDatabaseUrl(authorityDatabaseUrl);
  if (!authorityUrl.ok) {
    failures.push("relay_authority_database_url_invalid");
    report.authority_database = { valid: false };
    return { report, failures, unresolved };
  }

  const runtimeRole = authorityUrl.username;
  if (!runtimeRole) {
    failures.push("relay_authority_database_url_missing_username");
    return { report, failures, unresolved };
  }
  if (["neondb_owner", "postgres"].includes(runtimeRole)) {
    failures.push(`neon_runtime_role_not_scoped:${runtimeRole}`);
  }

  if (!checkRoles) {
    unresolved.push("run with --check-neon-roles to verify Postgres role attributes and authority tables");
    report.roles = [
      {
        name: runtimeRole,
        source: "RELAY_AUTHORITY_DATABASE_URL username",
        checked: false,
      },
    ];
    return { report, failures, unresolved };
  }

  if (ensureRoles) {
    if (!mutate) {
      failures.push("neon_role_ensure_without_mutate");
      return { report, failures, unresolved };
    }
    if (!adminDatabaseUrl) {
      failures.push("missing_env:NEON_ADMIN_DATABASE_URL");
      unresolved.push("NEON_ADMIN_DATABASE_URL or RELAY_AUTHORITY_ADMIN_DATABASE_URL is required to create/update Neon roles");
      return { report, failures, unresolved };
    }
    const adminUrl = parseDatabaseUrl(adminDatabaseUrl);
    if (!adminUrl.ok) {
      failures.push("neon_admin_database_url_invalid");
      return { report, failures, unresolved };
    }
    const ensure = ensureRuntimeRole({ adminUrl, runtimeUrl: authorityUrl, spawnSync });
    if (!ensure.ok) {
      failures.push(`neon_role_ensure_failed:${runtimeRole}`);
      report.role_ensure_error = ensure.error;
      return { report, failures, unresolved };
    }
    report.role_ensured = runtimeRole;
  }

  const roleCheck = queryRoleAttributes({ databaseUrl: authorityUrl, roleNames: [runtimeRole], spawnSync });
  if (!roleCheck.ok) {
    failures.push("neon_role_check_failed");
    report.role_check_error = roleCheck.error;
  } else {
    report.roles = roleCheck.roles;
    for (const role of roleCheck.roles) {
      if (!role.exists) failures.push(`neon_role_missing:${role.name}`);
      if (role.exists && !role.can_login) failures.push(`neon_role_no_login:${role.name}`);
      if (role.exists && !role.scoped) failures.push(`neon_role_not_scoped:${role.name}`);
    }
  }

  const tableCheck = queryAuthorityTables({ databaseUrl: authorityUrl, spawnSync });
  if (!tableCheck.ok) {
    failures.push("neon_table_check_failed");
    report.table_check_error = tableCheck.error;
  } else {
    report.tables = tableCheck.tables;
    for (const table of tableCheck.tables) {
      if (!table.exists) failures.push(`neon_table_missing:${table.name}`);
    }
  }

  unresolved.push("authority table grants for the scoped runtime role are not created by this readiness script");
  return { report, failures, unresolved };
}

function parseDatabaseUrl(raw) {
  try {
    const parsed = new URL(raw);
    if (parsed.protocol !== "postgres:" && parsed.protocol !== "postgresql:") {
      return { ok: false };
    }
    return {
      ok: true,
      protocol: parsed.protocol,
      hostname: parsed.hostname,
      port: parsed.port || "5432",
      database: parsed.pathname.replace(/^\//u, ""),
      username: decodeURIComponent(parsed.username || ""),
      password: decodeURIComponent(parsed.password || ""),
      sslmode: parsed.searchParams.get("sslmode") || "",
    };
  } catch {
    return { ok: false };
  }
}

function summarizeDatabaseUrl(raw) {
  const parsed = parseDatabaseUrl(raw);
  if (!raw) {
    return { present: false };
  }
  if (!parsed.ok) {
    return { present: true, valid: false };
  }
  return {
    present: true,
    valid: true,
    database_present: Boolean(parsed.database),
    host_present: Boolean(parsed.hostname),
    sslmode: parsed.sslmode || "",
    username: parsed.username || "",
  };
}

function ensureRuntimeRole({ adminUrl, runtimeUrl, spawnSync }) {
  if (!isSafeIdentifier(runtimeUrl.username)) {
    return { ok: false, error: "runtime role name is not a safe Postgres identifier" };
  }
  if (!runtimeUrl.password) {
    return { ok: false, error: "runtime database URL must include a password for role creation" };
  }
  const sql = `
do $ctx$
begin
  if not exists (select 1 from pg_catalog.pg_roles where rolname = ${sqlLiteral(runtimeUrl.username)}) then
    execute format('create role %I login password %L nosuperuser nocreatedb nocreaterole noreplication nobypassrls', ${sqlLiteral(runtimeUrl.username)}, ${sqlLiteral(runtimeUrl.password)});
  else
    execute format('alter role %I with login password %L nosuperuser nocreatedb nocreaterole noreplication nobypassrls', ${sqlLiteral(runtimeUrl.username)}, ${sqlLiteral(runtimeUrl.password)});
  end if;
end
$ctx$;
`;
  return runPsql(adminUrl, sql, spawnSync);
}

function queryRoleAttributes({ databaseUrl, roleNames, spawnSync }) {
  const safeNames = roleNames.filter(isSafeIdentifier);
  if (safeNames.length !== roleNames.length) {
    return { ok: false, error: "role name is not a safe Postgres identifier" };
  }
  const literals = safeNames.map(sqlLiteral).join(",");
  const sql = `
select rolname, rolcanlogin, rolsuper, rolcreaterole, rolcreatedb, rolreplication, rolbypassrls
from pg_catalog.pg_roles
where rolname in (${literals})
order by rolname;
`;
  const result = runPsql(databaseUrl, sql, spawnSync);
  if (!result.ok) return result;
  const rows = parsePsqlRows(result.stdout, [
    "name",
    "can_login",
    "superuser",
    "create_role",
    "create_db",
    "replication",
    "bypass_rls",
  ]);
  const byName = new Map(rows.map((row) => [row.name, row]));
  const roles = safeNames.map((name) => {
    const row = byName.get(name);
    if (!row) {
      return { name, exists: false };
    }
    const overprivileged =
      row.superuser === "t" ||
      row.create_role === "t" ||
      row.create_db === "t" ||
      row.replication === "t" ||
      row.bypass_rls === "t";
    return {
      name,
      exists: true,
      bypass_rls: row.bypass_rls === "t",
      can_login: row.can_login === "t",
      create_db: row.create_db === "t",
      create_role: row.create_role === "t",
      replication: row.replication === "t",
      scoped: !overprivileged,
      superuser: row.superuser === "t",
    };
  });
  return { ok: true, roles };
}

function queryAuthorityTables({ databaseUrl, spawnSync }) {
  const checks = AUTHORITY_TABLES.map((table) => `(${sqlLiteral(table)}, to_regclass(${sqlLiteral(`public.${table}`)}) is not null)`).join(",");
  const sql = `
select name, exists
from (values ${checks}) as required(name, exists)
order by name;
`;
  const result = runPsql(databaseUrl, sql, spawnSync);
  if (!result.ok) return result;
  const rows = parsePsqlRows(result.stdout, ["name", "exists"]);
  return {
    ok: true,
    tables: rows.map((row) => ({
      name: row.name,
      exists: row.exists === "t",
    })),
  };
}

function runPsql(databaseUrl, sql, spawnSync) {
  const psqlEnv = postgresEnv(databaseUrl);
  if (!psqlEnv.ok) {
    return { ok: false, error: "invalid database URL" };
  }
  const result = spawnSync("psql", ["-X", "-A", "-t", "-v", "ON_ERROR_STOP=1"], {
    encoding: "utf8",
    env: {
      ...process.env,
      ...psqlEnv.env,
    },
    input: sql,
  });
  if (result.error) {
    return { ok: false, error: "psql_unavailable" };
  }
  if (result.status !== 0) {
    return { ok: false, error: "psql_failed" };
  }
  return { ok: true, stdout: String(result.stdout || "") };
}

function postgresEnv(databaseUrl) {
  if (!databaseUrl.ok) return { ok: false };
  return {
    ok: true,
    env: {
      PGDATABASE: databaseUrl.database,
      PGHOST: databaseUrl.hostname,
      PGPASSWORD: databaseUrl.password,
      PGPORT: databaseUrl.port,
      PGSSLMODE: databaseUrl.sslmode || "require",
      PGUSER: databaseUrl.username,
    },
  };
}

function parsePsqlRows(stdout, columns) {
  return String(stdout || "")
    .trim()
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const parts = line.split("|");
      const row = {};
      for (let index = 0; index < columns.length; index += 1) {
        row[columns[index]] = parts[index] ?? "";
      }
      return row;
    });
}

function isSafeIdentifier(value) {
  return /^[A-Za-z_][A-Za-z0-9_]{0,62}$/u.test(value);
}

function sqlLiteral(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

async function safeJson(response) {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return { raw: "non_json_response" };
  }
}

function summarizeApiErrors(body) {
  if (Array.isArray(body?.errors) && body.errors.length > 0) {
    return body.errors
      .map((entry) => String(entry?.code || entry?.message || "api_error"))
      .filter(Boolean)
      .sort()
      .join(",");
  }
  if (typeof body?.message === "string") {
    return body.message;
  }
  if (body?.raw) {
    return body.raw;
  }
  return "api_error";
}

function skipped(reason) {
  return {
    report: {
      checked: false,
      skip_reason: reason,
    },
    failures: [],
    unresolved: [],
  };
}

function relativize(targetPath) {
  const relative = path.relative(REPO_ROOT, targetPath);
  if (!relative.startsWith("..") && !path.isAbsolute(relative)) {
    return relative || ".";
  }
  return targetPath;
}

function stableJson(value) {
  return `${JSON.stringify(sortJson(value), null, 2)}\n`;
}

function sortJson(value) {
  if (Array.isArray(value)) {
    return value.map(sortJson);
  }
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) {
      out[key] = sortJson(value[key]);
    }
    return out;
  }
  return value;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    process.stdout.write(usage());
    return;
  }
  const report = await buildReadinessReport({ argv: process.argv.slice(2) });
  process.stdout.write(stableJson(report));
  process.exitCode = report.ok ? 0 : 1;
}

export {
  buildReadinessReport,
  parseArgs,
  parseCloudflareSecretNames,
  parseCloudflareWorkerNames,
  parsePsqlRows,
  parseWranglerToml,
  stableJson,
};

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    process.stderr.write(`cloudflare-neon readiness failed: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exit(2);
  });
}
