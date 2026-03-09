#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const providerAuthMatrixPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "fixtures",
  "provider_auth_matrix.json",
);
const macosBreakMatrixPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "fixtures",
  "macos_break_matrix.json",
);
const defaultReportPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "docs",
  "ci_preflight_redaction_contract.md",
);
const defaultAllowlistPath = path.join(
  coreRoot,
  "apps",
  "desktop",
  "automation",
  "fixtures",
  "artifact_redaction_allowlist.tsv",
);
const defaultOpenRouterBaseUrl = "https://openrouter.ai/api/v1";

const secretPlaceholderValues = new Set([
  "",
  "changeme",
  "dummy",
  "example",
  "placeholder",
  "replace-me",
  "replace_me",
  "test",
  "test-key",
  "test_key",
  "todo",
  "your-key",
  "your-key-here",
  "your-token",
  "your-token-here",
  "your_api_key",
  "your_api_key_here",
  "your_key",
  "your_key_here",
]);

const envSpecs = {
  CN_API_KEY: {
    kind: "secret",
    description: "CrabNebula API key used by macOS desktop WebDriver automation.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
  OPENROUTER_API_KEY: {
    kind: "secret",
    description: "OpenRouter API key used by real provider endpoint auth and first-turn checks.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
  OPENROUTER_BASE_URL: {
    kind: "config",
    description: "OpenRouter-compatible base URL for endpoint auth suites.",
    validation: [
      "must be an https URL when provided",
      `defaults to ${defaultOpenRouterBaseUrl}`,
    ],
  },
  CTX_E2E_CODEX_OAUTH_EMAIL: {
    kind: "config",
    description: "Codex OAuth account email for real browser-backed subscription login coverage.",
    validation: [
      "trimmed, non-empty email address",
      "must contain @",
      "must not contain whitespace",
    ],
    format: "email",
  },
  CTX_E2E_CODEX_OAUTH_PASSWORD: {
    kind: "secret",
    description: "Codex OAuth account password for real browser-backed subscription login coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "minimum length 8",
    ],
    minLength: 8,
    allowWhitespace: true,
  },
  CTX_E2E_CODEX_OAUTH_TOTP_SECRET: {
    kind: "secret",
    description: "Codex OAuth authenticator secret used to generate TOTP codes during real login coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must decode as base32 after removing spaces, hyphens, and = padding",
      "minimum normalized length 16",
    ],
    minLength: 16,
    allowWhitespace: true,
    format: "base32",
  },
  CTX_E2E_CURSOR_API_KEY: {
    kind: "secret",
    description: "Cursor provider API key for real provider-api-auth Playwright coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
  CTX_E2E_CURSOR_EMAIL: {
    kind: "config",
    description: "Cursor account email attached to managed subscription auth upserts.",
    validation: [
      "trimmed, non-empty email address when provided",
      "must contain @",
      "must not contain whitespace",
    ],
    format: "email",
  },
  CTX_E2E_GEMINI_API_KEY: {
    kind: "secret",
    description: "Gemini provider API key for real provider-api-auth Playwright coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
  AWS_ACCESS_KEY_ID: {
    kind: "secret",
    description: "AWS access key ID used to provision real remote CI fixture hosts.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 16",
    ],
    minLength: 16,
  },
  AWS_SECRET_ACCESS_KEY: {
    kind: "secret",
    description: "AWS secret access key paired with AWS_ACCESS_KEY_ID for remote CI fixture provisioning.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 20",
    ],
    minLength: 20,
  },
  AWS_SESSION_TOKEN: {
    kind: "secret",
    description: "Optional AWS STS session token for temporary credentials in remote CI fixture provisioning.",
    validation: [
      "trimmed, non-empty secret value when provided",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 20",
    ],
    minLength: 20,
  },
  AWS_REGION: {
    kind: "config",
    description: "AWS region used for remote CI fixture provisioning.",
    validation: [
      "trimmed, non-empty region identifier",
      "must match /^[a-z]{2}-[a-z]+-[0-9]+$/",
      "must not contain whitespace",
    ],
    format: "aws_region",
  },
  HETZNER_API_TOKEN: {
    kind: "secret",
    description: "Hetzner API token used to provision remote real-CI fixture VMs.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
};

Object.assign(envSpecs, {
  CTX_E2E_CLAUDE_SETUP_TOKEN: {
    kind: "secret",
    description: "Claude setup token used by deferred provider-auth matrix managed subscription upserts.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
  CTX_E2E_CLAUDE_SETUP_TOKEN_PATH: {
    kind: "config",
    description: "Path to a file containing the Claude setup token for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty file path when provided",
      "file must exist and be readable",
    ],
  },
  CTX_E2E_GEMINI_OAUTH_CREDS_JSON: {
    kind: "secret",
    description: "Gemini managed OAuth credentials JSON for deferred provider-auth matrix subscription coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "minimum length 20",
    ],
    minLength: 20,
    allowWhitespace: true,
  },
  CTX_E2E_GEMINI_OAUTH_CREDS_PATH: {
    kind: "config",
    description: "Path to a file containing Gemini managed OAuth credentials JSON for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty file path when provided",
      "file must exist and be readable",
    ],
  },
  CTX_E2E_QWEN_OAUTH_CREDS_JSON: {
    kind: "secret",
    description: "Qwen managed OAuth credentials JSON for deferred provider-auth matrix subscription coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "minimum length 20",
    ],
    minLength: 20,
    allowWhitespace: true,
  },
  CTX_E2E_QWEN_OAUTH_CREDS_PATH: {
    kind: "config",
    description: "Path to a file containing Qwen managed OAuth credentials JSON for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty file path when provided",
      "file must exist and be readable",
    ],
  },
  CTX_E2E_KIMI_CREDENTIALS_JSON: {
    kind: "secret",
    description: "Kimi credentials JSON for deferred provider-auth matrix subscription coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "minimum length 20",
    ],
    minLength: 20,
    allowWhitespace: true,
  },
  CTX_E2E_KIMI_CREDENTIALS_PATH: {
    kind: "config",
    description: "Path to a file containing Kimi credentials JSON for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty file path when provided",
      "file must exist and be readable",
    ],
  },
  CTX_E2E_COPILOT_TOKEN: {
    kind: "secret",
    description: "Copilot token used by deferred provider-auth matrix managed subscription upserts.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "must not contain whitespace",
      "minimum length 10",
    ],
    minLength: 10,
  },
  CTX_E2E_COPILOT_TOKEN_PATH: {
    kind: "config",
    description: "Path to a file containing the Copilot token for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty file path when provided",
      "file must exist and be readable",
    ],
  },
  CTX_E2E_AMP_SECRETS_JSON: {
    kind: "secret",
    description: "Amp secrets JSON used by deferred provider-auth matrix subscription coverage.",
    validation: [
      "trimmed, non-empty secret value",
      "must not be a placeholder example",
      "minimum length 20",
    ],
    minLength: 20,
    allowWhitespace: true,
  },
  CTX_E2E_AMP_SECRETS_PATH: {
    kind: "config",
    description: "Path to a file containing Amp secrets JSON for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty file path when provided",
      "file must exist and be readable",
    ],
  },
  CTX_E2E_AMP_HOME_SEED_DIR: {
    kind: "config",
    carriesSecretPayload: true,
    description: "Path to a seeded Amp runtime home directory for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty directory path when provided",
      "directory must exist",
    ],
  },
  CTX_E2E_MISTRAL_HOME_SEED_DIR: {
    kind: "config",
    carriesSecretPayload: true,
    description: "Path to a seeded Mistral runtime home directory for deferred provider-auth matrix coverage.",
    validation: [
      "trimmed, non-empty directory path when provided",
      "directory must exist",
    ],
  },
});

const providerAuthEnvAlternatives = {
  CTX_E2E_CLAUDE_SETUP_TOKEN: [
    { envName: "CTX_E2E_CLAUDE_SETUP_TOKEN", source: "env" },
    { envName: "CTX_E2E_CLAUDE_SETUP_TOKEN_PATH", source: "file" },
  ],
  CTX_E2E_GEMINI_OAUTH_CREDS_JSON: [
    { envName: "CTX_E2E_GEMINI_OAUTH_CREDS_JSON", source: "env" },
    { envName: "CTX_E2E_GEMINI_OAUTH_CREDS_PATH", source: "file" },
  ],
  CTX_E2E_QWEN_OAUTH_CREDS_JSON: [
    { envName: "CTX_E2E_QWEN_OAUTH_CREDS_JSON", source: "env" },
    { envName: "CTX_E2E_QWEN_OAUTH_CREDS_PATH", source: "file" },
  ],
  CTX_E2E_KIMI_CREDENTIALS_JSON: [
    { envName: "CTX_E2E_KIMI_CREDENTIALS_JSON", source: "env" },
    { envName: "CTX_E2E_KIMI_CREDENTIALS_PATH", source: "file" },
  ],
  CTX_E2E_COPILOT_TOKEN: [
    { envName: "CTX_E2E_COPILOT_TOKEN", source: "env" },
    { envName: "CTX_E2E_COPILOT_TOKEN_PATH", source: "file" },
  ],
  CTX_E2E_AMP_SECRETS_JSON: [
    { envName: "CTX_E2E_AMP_HOME_SEED_DIR", source: "dir" },
    { envName: "CTX_E2E_AMP_SECRETS_JSON", source: "env" },
    { envName: "CTX_E2E_AMP_SECRETS_PATH", source: "file" },
  ],
  CTX_E2E_MISTRAL_HOME_SEED_DIR: [
    { envName: "CTX_E2E_MISTRAL_HOME_SEED_DIR", source: "dir" },
  ],
};

const cloneRequirementAlternatives = (envName, source = "env") => {
  if (source !== "env") {
    return [{ envName, source }];
  }
  const alternatives = providerAuthEnvAlternatives[envName];
  if (!Array.isArray(alternatives) || alternatives.length === 0) {
    return [{ envName, source: "env" }];
  }
  return alternatives.map((entry) => ({ ...entry }));
};

const suiteIds = [
  "provider-auth-matrix-required",
  "provider-auth-matrix-nightly",
  "providers-tokens",
  "providers-endpoint-ui",
  "providers-provider-api-auth",
  "macos-break-matrix",
  "mac-webdriver-quick-smoke",
  "desktop-remote-real-ci",
];

const normalizeText = (value) => (typeof value === "string" ? value.trim() : "");
const envCarriesSecretPayload = (envName) =>
  envSpecs[envName]?.kind === "secret" || envSpecs[envName]?.carriesSecretPayload === true;

const readJson = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));

const isProbablyTextBuffer = (buffer) => {
  const limit = Math.min(buffer.length, 4096);
  for (let index = 0; index < limit; index += 1) {
    if (buffer[index] === 0) return false;
  }
  return true;
};

const collectDirectorySecretPayloads = (rootDir, payloads = [], totalSize = { value: 0 }) => {
  if (payloads.length >= 32 || totalSize.value >= 262144) {
    return payloads;
  }
  let entries = [];
  try {
    entries = fs.readdirSync(rootDir, { withFileTypes: true });
  } catch {
    return payloads;
  }
  entries.sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    if (payloads.length >= 32 || totalSize.value >= 262144) break;
    const nextPath = path.join(rootDir, entry.name);
    if (entry.isDirectory()) {
      collectDirectorySecretPayloads(nextPath, payloads, totalSize);
      continue;
    }
    if (!entry.isFile()) continue;
    let buffer;
    try {
      buffer = fs.readFileSync(nextPath);
    } catch {
      continue;
    }
    if (!isProbablyTextBuffer(buffer)) continue;
    totalSize.value += buffer.length;
    const value = normalizeText(buffer.toString("utf8"));
    if (value) payloads.push(value);
  }
  return payloads;
};

const commandExists = (command, cwd = repoRoot) => {
  const result = childProcess.spawnSync("bash", ["-lc", `command -v ${command}`], {
    cwd,
    encoding: "utf8",
    stdio: "ignore",
  });
  return result.status === 0;
};

const getDataRoot = (env) => {
  const value = normalizeText(env.CTX_DATA_ROOT || "");
  if (value) return value;
  const home = normalizeText(env.HOME || "") || os.homedir();
  return path.join(home, ".ctx");
};

const getTitleGenerationSettings = (env) => {
  const settingsPath = path.join(getDataRoot(env), "settings.json");
  if (!fs.existsSync(settingsPath)) {
    return { settingsPath, value: "", present: false };
  }
  try {
    const parsed = readJson(settingsPath);
    const value = normalizeText(parsed?.title_generation?.api_key || "");
    return { settingsPath, value, present: Boolean(value) };
  } catch {
    return { settingsPath, value: "", present: false };
  }
};

const validateSecretValue = (envName, value) => {
  const spec = envSpecs[envName];
  const trimmed = normalizeText(value);
  const errors = [];
  if (!trimmed) {
    errors.push("is empty");
    return errors;
  }
  if (trimmed !== value) {
    errors.push("must not include leading or trailing whitespace");
  }
  if (!spec?.allowWhitespace && /\s/.test(trimmed)) {
    errors.push("must not contain whitespace");
  }
  if (secretPlaceholderValues.has(trimmed.toLowerCase())) {
    errors.push("must not be a placeholder example value");
  }
  const minLength = Number(spec?.minLength || 0);
  if (minLength > 0 && trimmed.length < minLength) {
    errors.push(`must be at least ${minLength} characters`);
  }
  if (spec?.format === "base32") {
    const normalized = trimmed.toUpperCase().replace(/[\s=-]+/g, "");
    if (!normalized) {
      errors.push("must decode as base32 after removing separators");
    } else if (!/^[A-Z2-7]+$/.test(normalized)) {
      errors.push("must decode as base32 after removing separators");
    } else if (normalized.length < minLength) {
      errors.push(`must be at least ${minLength} characters after removing separators`);
    }
  }
  return errors;
};

const validateConfigValue = (envName, value) => {
  const trimmed = normalizeText(value);
  if (envName === "OPENROUTER_BASE_URL") {
    if (!trimmed) return [];
    try {
      const parsed = new URL(trimmed);
      if (parsed.protocol !== "https:") {
        return ["must use https"];
      }
    } catch {
      return ["must be a valid URL"];
    }
    return [];
  }
  if (envSpecs[envName]?.format === "email") {
    if (!trimmed) return ["is empty"];
    if (trimmed !== value) return ["must not include leading or trailing whitespace"];
    if (/\s/.test(trimmed)) return ["must not contain whitespace"];
    if (!trimmed.includes("@")) return ["must contain @"];
    return [];
  }
  if (envSpecs[envName]?.format === "aws_region") {
    if (!trimmed) return ["is empty"];
    if (trimmed !== value) return ["must not include leading or trailing whitespace"];
    if (/\s/.test(trimmed)) return ["must not contain whitespace"];
    if (!/^[a-z]{2}-[a-z]+-[0-9]+$/.test(trimmed)) return ["must be a valid AWS region (for example us-east-1)"];
    return [];
  }
  return [];
};

const validateResolvedValue = (envName, value) => {
  const spec = envSpecs[envName];
  if (!spec) return [];
  if (spec.kind === "secret") return validateSecretValue(envName, value);
  return validateConfigValue(envName, value);
};

const resolveRequirementAlternative = (alternative, env) => {
  const envName = normalizeText(alternative?.envName || "");
  const source = normalizeText(alternative?.source || "env");
  if (!envName) {
    return { status: "missing" };
  }

  if (source === "env") {
    const value = normalizeText(env[envName] || "");
    if (!value) {
      return { status: "missing" };
    }
    return {
      status: "present",
      value,
      valueSource: `env:${envName}`,
      secretValues: envCarriesSecretPayload(envName) && value ? [value] : [],
    };
  }

  if (source === "file") {
    const filePath = normalizeText(env[envName] || "");
    if (!filePath) {
      return { status: "missing" };
    }
    if (!fs.existsSync(filePath)) {
      return {
        status: "invalid",
        valueSource: `env:${envName}`,
        errors: [`${envName} file not found: ${filePath}`],
      };
    }
    let stat;
    try {
      stat = fs.statSync(filePath);
    } catch (error) {
      const code = normalizeText(error?.code || "");
      return {
        status: "invalid",
        valueSource: `env:${envName}`,
        errors: [`${envName} file could not be inspected: ${filePath}${code ? ` (${code})` : ""}`],
      };
    }
    if (!stat.isFile()) {
      return {
        status: "invalid",
        valueSource: `env:${envName}`,
        errors: [`${envName} is not a file: ${filePath}`],
      };
    }
    let rawValue = "";
    try {
      rawValue = fs.readFileSync(filePath, "utf8");
    } catch (error) {
      const code = normalizeText(error?.code || "");
      return {
        status: "invalid",
        valueSource: `env:${envName}`,
        errors: [`${envName} file could not be read: ${filePath}${code ? ` (${code})` : ""}`],
      };
    }
    const value = normalizeText(rawValue);
    return {
      status: "present",
      value,
      valueSource: `file:${envName}`,
      secretValues: envCarriesSecretPayload(envName) && value ? [value] : [],
    };
  }

  if (source === "dir") {
    const dirPath = normalizeText(env[envName] || "");
    if (!dirPath) {
      return { status: "missing" };
    }
    if (!fs.existsSync(dirPath)) {
      return {
        status: "invalid",
        valueSource: `env:${envName}`,
        errors: [`${envName} directory not found: ${dirPath}`],
      };
    }
    const stat = fs.statSync(dirPath);
    if (!stat.isDirectory()) {
      return {
        status: "invalid",
        valueSource: `env:${envName}`,
        errors: [`${envName} is not a directory: ${dirPath}`],
      };
    }
    return {
      status: "present",
      value: "",
      valueSource: `dir:${envName}`,
      skipValidation: true,
      secretValues: envCarriesSecretPayload(envName) ? collectDirectorySecretPayloads(dirPath) : [],
    };
  }

  throw new Error(`unsupported requirement alternative source '${source}'`);
};

const resolveRequirement = (requirement, context) => {
  const env = context.env || process.env;
  const platform = normalizeText(context.platform || process.platform);
  const envName = requirement.envName;
  const fromEnv = normalizeText(env[envName] || "");
  const base = {
    envName,
    source: requirement.source,
    applies: requirement.applies !== false,
    required: Boolean(requirement.required),
    condition: normalizeText(requirement.condition || ""),
    note: normalizeText(requirement.note || ""),
    derivedFrom: Array.isArray(requirement.derivedFrom) ? requirement.derivedFrom.slice() : [],
    value: "",
    valueSource: "",
    errors: [],
    status: "missing",
    alternatives: Array.isArray(requirement.alternatives) ? requirement.alternatives.slice() : [],
    secretValues: [],
  };

  const finish = (status, value, valueSource, extra = {}) => {
    const next = { ...base, status, value: value || "", valueSource: valueSource || "" };
    next.secretValues = Array.isArray(extra.secretValues)
      ? extra.secretValues.map((entry) => normalizeText(entry)).filter(Boolean)
      : (value ? [value] : []);
    if (Array.isArray(extra.errors) && extra.errors.length > 0) {
      next.errors = extra.errors.slice();
      next.status = "invalid";
      return next;
    }
    if (value && !extra.skipValidation) {
      next.errors = validateResolvedValue(extra.validationEnvName || envName, value);
      if (next.errors.length > 0) {
        next.status = "invalid";
      }
    }
    return next;
  };

  if (!base.applies) {
    return finish("skipped", "", "");
  }

  if (Array.isArray(requirement.alternatives) && requirement.alternatives.length > 0 && requirement.source === "env") {
    for (const alternative of requirement.alternatives) {
      const alternativeResolution = resolveRequirementAlternative(alternative, env);
      if (alternativeResolution.status === "present") {
        return finish(
          "present",
          alternativeResolution.value,
          alternativeResolution.valueSource,
          {
            validationEnvName: envName,
            skipValidation: Boolean(alternativeResolution.skipValidation),
            secretValues: alternativeResolution.secretValues,
          },
        );
      }
      if (alternativeResolution.status === "invalid") {
        return finish("invalid", "", alternativeResolution.valueSource, {
          errors: alternativeResolution.errors,
          skipValidation: true,
        });
      }
    }
    return finish("missing", "", "");
  }

  if (fromEnv) {
    return finish("present", fromEnv, "env");
  }

  switch (requirement.source) {
    case "env":
      return finish("missing", "", "");
    case "env_or_default":
      return finish("default", normalizeText(requirement.defaultValue || ""), "default");
    case "env_or_title_generation_settings": {
      const settings = getTitleGenerationSettings(env);
      if (settings.present) {
        return finish("present", settings.value, "settings:title_generation.api_key");
      }
      return finish("missing", "", "");
    }
    case "env_or_infisical_darwin": {
      if (platform !== "darwin") {
        return finish("skipped", "", "");
      }
      const infisicalConfigPath = path.join(coreRoot, ".infisical.json");
      if (fs.existsSync(infisicalConfigPath) && commandExists("infisical", repoRoot)) {
        return finish("available", "", "infisical");
      }
      return finish("missing", "", "");
    }
    default:
      throw new Error(`unsupported requirement source '${requirement.source}'`);
  }
};

const buildRequirement = (envName, extra = {}) => ({
  envName,
  required: extra.required !== false,
  applies: extra.applies !== false,
  source: extra.source || "env",
  condition: extra.condition || "",
  note: extra.note || "",
  defaultValue: extra.defaultValue || "",
  derivedFrom: Array.isArray(extra.derivedFrom) ? extra.derivedFrom.slice() : [],
  alternatives: Array.isArray(extra.alternatives)
    ? extra.alternatives.map((entry) => ({ ...entry }))
    : cloneRequirementAlternatives(envName, extra.source || "env"),
});

const addDerivedCellIds = (map, cell, id) => {
  const prerequisites = Array.isArray(cell.prerequisites) ? cell.prerequisites : [];
  for (const entry of prerequisites) {
    const envName = normalizeText(entry);
    if (!envName) continue;
    const existing = map.get(envName) || [];
    existing.push(id);
    map.set(envName, existing);
  }
};

const resolveProviderAuthDynamic = ({ lane, cellIds = [], env = process.env }) => {
  const manifestPath = normalizeText(env.CTX_PROVIDER_AUTH_MATRIX_FIXTURE || "") || providerAuthMatrixPath;
  const manifest = readJson(manifestPath);
  const cells = Array.isArray(manifest.cells) ? manifest.cells : [];
  const selectedIds = cellIds.length > 0 ? new Set(cellIds.map((value) => normalizeText(value)).filter(Boolean)) : null;
  const unknownCellIds = selectedIds
    ? Array.from(selectedIds).filter((cellId) => !cells.some((cell) => normalizeText(cell?.id) === cellId))
    : [];
  if (unknownCellIds.length > 0) {
    throw new Error(`unknown provider auth matrix cell(s): ${unknownCellIds.join(", ")}`);
  }

  const filtered = cells.filter((cell) => {
    const id = normalizeText(cell?.id);
    if (!id) return false;
    if (selectedIds && !selectedIds.has(id)) return false;
    return normalizeText(cell?.lane) === lane;
  });
  const supported = filtered.filter((cell) => normalizeText(cell?.support) === "supported");
  const deferredRunnerBacked = filtered.filter(
    (cell) => normalizeText(cell?.support) === "deferred" && normalizeText(cell?.runner?.kind) !== "none",
  );
  const prerequisiteMap = new Map();
  for (const cell of supported) {
    addDerivedCellIds(prerequisiteMap, cell, normalizeText(cell.id));
  }
  const deferredPrerequisiteMap = new Map();
  for (const cell of deferredRunnerBacked) {
    addDerivedCellIds(deferredPrerequisiteMap, cell, normalizeText(cell.id));
  }
  return {
    lane,
    selectedCellIds: filtered.map((cell) => normalizeText(cell.id)).filter(Boolean),
    supportedCellIds: supported.map((cell) => normalizeText(cell.id)).filter(Boolean),
    deferredRunnerCellIds: deferredRunnerBacked.map((cell) => normalizeText(cell.id)).filter(Boolean),
    prerequisiteMap,
    deferredPrerequisiteMap,
  };
};

const resolveBreakMatrixDynamic = ({ caseIds = [], env = process.env }) => {
  const manifestPath = normalizeText(env.CTX_MACOS_BREAK_MATRIX_FIXTURE || "") || macosBreakMatrixPath;
  const manifest = readJson(manifestPath);
  const cases = Array.isArray(manifest.cases) ? manifest.cases : [];
  const selectedIds = caseIds.length > 0 ? new Set(caseIds.map((value) => normalizeText(value)).filter(Boolean)) : null;
  const unknownCaseIds = selectedIds
    ? Array.from(selectedIds).filter((caseId) => !cases.some((row) => normalizeText(row?.id) === caseId))
    : [];
  if (unknownCaseIds.length > 0) {
    throw new Error(`unknown macOS break-matrix case(s): ${unknownCaseIds.join(", ")}`);
  }

  const selectedCases = cases.filter((row) => {
    const id = normalizeText(row?.id);
    if (!id) return false;
    return selectedIds ? selectedIds.has(id) : true;
  });

  const prerequisiteMap = new Map();
  for (const row of selectedCases) {
    const id = normalizeText(row.id);
    const preconditions = Array.isArray(row.preconditions) ? row.preconditions : [];
    for (const precondition of preconditions) {
      const match = normalizeText(precondition).match(/\b([A-Z0-9_]+)\s+set\b/);
      if (!match) continue;
      const envName = normalizeText(match[1]);
      if (!envName) continue;
      const existing = prerequisiteMap.get(envName) || [];
      existing.push(id);
      prerequisiteMap.set(envName, existing);
    }
  }

  return {
    selectedCaseIds: selectedCases.map((row) => normalizeText(row.id)).filter(Boolean),
    prerequisiteMap,
  };
};

const uniq = (values) => Array.from(new Set(values.filter(Boolean)));

const collectSecretEnvNames = (requirements) =>
  uniq(
    (Array.isArray(requirements) ? requirements : [])
      .map((requirement) => normalizeText(requirement?.envName))
      .filter((envName) => envCarriesSecretPayload(envName)),
  );

const resolveSuiteContract = (suiteId, options = {}) => {
  const platform = normalizeText(options.platform || process.platform);
  const env = options.env || process.env;
  const normalizedId = normalizeText(suiteId);

  switch (normalizedId) {
    case "provider-auth-matrix-required":
    case "provider-auth-matrix-nightly": {
      const lane = normalizedId.endsWith("required") ? "required" : "nightly";
      const includeDeferred = Boolean(options.includeDeferred);
      const dynamic = resolveProviderAuthDynamic({
        lane,
        cellIds: Array.isArray(options.cellIds) ? options.cellIds : [],
        env,
      });
      const requirements = [];
      for (const [envName, derivedFrom] of dynamic.prerequisiteMap.entries()) {
        requirements.push(buildRequirement(envName, {
          source: "env",
          derivedFrom,
          note: `derived from supported provider_auth_matrix ${lane} lane cells`,
        }));
      }
      const deferredOptionalRequirements = [];
      if (includeDeferred) {
        for (const [envName, derivedFrom] of dynamic.deferredPrerequisiteMap.entries()) {
          requirements.push(buildRequirement(envName, {
            source: "env",
            derivedFrom,
            note: `required when provider_auth_matrix ${lane} runs include deferred runner-backed cells`,
          }));
        }
      } else {
        for (const [envName, derivedFrom] of dynamic.deferredPrerequisiteMap.entries()) {
          deferredOptionalRequirements.push(buildRequirement(envName, {
            required: false,
            source: "env",
            derivedFrom,
            note: `accepted for provider_auth_matrix ${lane} include-deferred runs`,
          }));
        }
      }
      if (dynamic.supportedCellIds.length > 0 || (includeDeferred && dynamic.deferredRunnerCellIds.length > 0)) {
        requirements.push(buildRequirement("CN_API_KEY", {
          source: "env_or_infisical_darwin",
          applies: platform === "darwin",
          condition: "macOS only",
          note: "required when supported provider-auth matrix cells run through CrabNebula WebDriver",
        }));
      }
      const optionalRequirements = [
        ...deferredOptionalRequirements,
        ...((dynamic.prerequisiteMap.has("OPENROUTER_API_KEY") || dynamic.deferredPrerequisiteMap.has("OPENROUTER_API_KEY"))
          ? [
            buildRequirement("OPENROUTER_BASE_URL", {
              required: false,
              applies: true,
              source: "env_or_default",
              defaultValue: defaultOpenRouterBaseUrl,
              note: "OpenRouter base URL falls back to the canonical default when unset",
            }),
          ]
          : []),
      ];
      return {
        id: normalizedId,
        title: lane === "required" ? "Provider auth matrix required lane" : "Provider auth matrix nightly lane",
        description: "Preflight for desktop provider-auth matrix orchestration before any expensive WDIO bootstrap.",
        requirements,
        optionalRequirements,
        artifactRoots: ["core/apps/desktop/automation/artifacts/provider-auth-matrix/"],
        redactionEnvNames: collectSecretEnvNames([...requirements, ...optionalRequirements]),
        metadata: {
          lane,
          includeDeferred,
          selectedCellIds: dynamic.selectedCellIds,
          supportedCellIds: dynamic.supportedCellIds,
          deferredRunnerCellIds: dynamic.deferredRunnerCellIds,
        },
        notes: [
          ...(dynamic.supportedCellIds.length === 0
            ? ["No supported provider-auth matrix cells are currently selected for this lane."]
            : []),
          ...(!includeDeferred && dynamic.deferredRunnerCellIds.length > 0
            ? ["Deferred runner-backed nightly cells publish optional secret-contract entries here and become blocking only with --include-deferred."]
            : []),
        ],
        env,
        platform,
      };
    }
    case "providers-tokens":
      return {
        id: normalizedId,
        title: "Provider tokens suite",
        description: "Rust provider token e2e suite driven by scripts/providers_e2e.sh tokens.",
        requirements: [
          buildRequirement("OPENROUTER_API_KEY", {
            source: "env_or_title_generation_settings",
            note: "may be sourced from CTX_DATA_ROOT/settings.json title_generation.api_key for local runs",
          }),
        ],
        optionalRequirements: [
          buildRequirement("OPENROUTER_BASE_URL", {
            required: false,
            source: "env_or_default",
            defaultValue: defaultOpenRouterBaseUrl,
          }),
        ],
        artifactRoots: [],
        redactionEnvNames: ["OPENROUTER_API_KEY"],
        metadata: {},
        notes: [],
        env,
        platform,
      };
    case "providers-endpoint-ui":
      return {
        id: normalizedId,
        title: "Provider endpoint-ui suite",
        description: "Real Playwright endpoint-auth provider suite driven by scripts/providers_e2e.sh endpoint-ui.",
        requirements: [
          buildRequirement("OPENROUTER_API_KEY", {
            source: "env_or_title_generation_settings",
            note: "may be sourced from CTX_DATA_ROOT/settings.json title_generation.api_key for local runs",
          }),
        ],
        optionalRequirements: [
          buildRequirement("OPENROUTER_BASE_URL", {
            required: false,
            source: "env_or_default",
            defaultValue: defaultOpenRouterBaseUrl,
          }),
        ],
        artifactRoots: [],
        redactionEnvNames: ["OPENROUTER_API_KEY"],
        metadata: {},
        notes: [],
        env,
        platform,
      };
    case "providers-provider-api-auth":
      return {
        id: normalizedId,
        title: "Provider API-key auth suite",
        description: "Real Playwright suite validating provider-native API-key auth flows.",
        requirements: [
          buildRequirement("CTX_E2E_CURSOR_API_KEY"),
          buildRequirement("CTX_E2E_GEMINI_API_KEY"),
        ],
        optionalRequirements: [],
        artifactRoots: [],
        redactionEnvNames: ["CTX_E2E_CURSOR_API_KEY", "CTX_E2E_GEMINI_API_KEY"],
        metadata: {},
        notes: [],
        env,
        platform,
      };
    case "macos-break-matrix": {
      const dynamic = resolveBreakMatrixDynamic({
        caseIds: Array.isArray(options.caseIds) ? options.caseIds : [],
        env,
      });
      const requirements = [];
      if (dynamic.selectedCaseIds.length > 0) {
        requirements.push(buildRequirement("CN_API_KEY", {
          source: "env_or_infisical_darwin",
          applies: platform === "darwin",
          condition: "macOS only",
          note: "required because the break matrix executes WDIO through CrabNebula on macOS",
          derivedFrom: dynamic.selectedCaseIds,
        }));
      }
      for (const [envName, derivedFrom] of dynamic.prerequisiteMap.entries()) {
        requirements.push(buildRequirement(envName, {
          source: "env",
          derivedFrom,
          note: "derived from macos_break_matrix fixture preconditions",
        }));
      }
      const optionalRequirements = dynamic.prerequisiteMap.has("OPENROUTER_API_KEY")
        ? [
          buildRequirement("OPENROUTER_BASE_URL", {
            required: false,
            source: "env_or_default",
            defaultValue: defaultOpenRouterBaseUrl,
          }),
        ]
        : [];
      return {
        id: normalizedId,
        title: "macOS break matrix",
        description: "Targeted desktop break-matrix coverage for auth/runtime regression contracts.",
        requirements,
        optionalRequirements,
        artifactRoots: ["core/apps/desktop/automation/artifacts/macos-break-matrix/"],
        redactionEnvNames: collectSecretEnvNames([...requirements, ...optionalRequirements]),
        metadata: {
          selectedCaseIds: dynamic.selectedCaseIds,
        },
        notes: [],
        env,
        platform,
      };
    }
    case "mac-webdriver-quick-smoke":
      return {
        id: normalizedId,
        title: "macOS WebDriver quick smoke",
        description: "Updater/native smoke lane that boots the CrabNebula WebDriver stack directly in CI.",
        requirements: [
          buildRequirement("CN_API_KEY", {
            source: "env",
            applies: platform === "darwin",
            condition: "macOS only",
          }),
        ],
        optionalRequirements: [],
        artifactRoots: [],
        redactionEnvNames: ["CN_API_KEY"],
        metadata: {},
        notes: [
          "This auth contract intentionally excludes CTX_DESKTOP_UPDATER_PUBKEY because it is required config, not a secret.",
        ],
        env,
        platform,
      };
    case "desktop-remote-real-ci":
      {
        const remoteProvider = normalizeText(env.CTX_UPDATER_E2E_CLOUD_PROVIDER || "aws").toLowerCase();
        const usingAws = remoteProvider === "aws";
        const usingHetzner = remoteProvider === "hetzner";
        const usingLocal = remoteProvider === "local";
        const requirements = usingAws
          ? [
            buildRequirement("AWS_ACCESS_KEY_ID"),
            buildRequirement("AWS_SECRET_ACCESS_KEY"),
            buildRequirement("AWS_REGION"),
          ]
          : usingHetzner
            ? [
              buildRequirement("HETZNER_API_TOKEN"),
            ]
            : [];
        const optionalRequirements = usingAws
          ? [
            buildRequirement("AWS_SESSION_TOKEN", {
              required: false,
            }),
          ]
          : [];
        return {
          id: normalizedId,
          title: "Desktop remote real CI",
          description: "Remote fixture lane that provisions live hosts before desktop remote coverage runs.",
          requirements,
          optionalRequirements,
          artifactRoots: [],
          redactionEnvNames: collectSecretEnvNames([...requirements, ...optionalRequirements]),
          metadata: {
            remoteProvider,
          },
          notes: [
            usingAws
              ? "AWS mode validates access key + secret + region; optional session token is accepted."
              : usingHetzner
                ? "Hetzner mode validates HETZNER_API_TOKEN."
                : usingLocal
                  ? "Local mode uses Docker SSH fixtures and does not require cloud credentials."
                  : "Unknown provider; no cloud-specific preflight requirements were resolved.",
          ],
          env,
          platform,
        };
      }
    default:
      throw new Error(`unsupported suite id '${suiteId}'`);
  }
};

const resolveSuiteForReport = (suiteId) => {
  if (suiteId === "macos-break-matrix") {
    return resolveSuiteContract(suiteId, { platform: "darwin" });
  }
  if (suiteId === "mac-webdriver-quick-smoke") {
    return resolveSuiteContract(suiteId, { platform: "darwin" });
  }
  if (suiteId.startsWith("provider-auth-matrix-")) {
    return resolveSuiteContract(suiteId, { platform: "darwin" });
  }
  return resolveSuiteContract(suiteId, { platform: process.platform });
};

const sourceSummary = (source) => {
  switch (source) {
    case "env":
      return "Environment variable";
    case "env_or_default":
      return "Environment variable or suite default";
    case "env_or_title_generation_settings":
      return "Environment variable or CTX_DATA_ROOT/settings.json title_generation.api_key";
    case "env_or_infisical_darwin":
      return "Environment variable or Infisical access on macOS";
    case "file":
      return "Path to readable file containing the secret payload";
    case "dir":
      return "Directory path containing a staged runtime seed";
    default:
      return source;
  }
};

const requirementLabel = (requirement) => {
  let label = requirement.envName;
  if (requirement.condition) {
    label += ` (${requirement.condition})`;
  }
  return label;
};

const renderRequirementList = (requirements) => {
  if (!Array.isArray(requirements) || requirements.length === 0) return "None";
  return requirements
    .map((requirement) => {
      const pieces = [requirementLabel(requirement)];
      const alternatives = Array.isArray(requirement.alternatives)
        ? requirement.alternatives
          .map((entry) => normalizeText(entry?.envName))
          .filter((envName) => envName && envName !== requirement.envName)
        : [];
      if (alternatives.length > 0) {
        pieces.push(`accepted via ${alternatives.join(" or ")}`);
      }
      if (requirement.defaultValue) {
        pieces.push(`default: ${requirement.defaultValue}`);
      }
      if (requirement.note) {
        pieces.push(requirement.note);
      }
      if (Array.isArray(requirement.derivedFrom) && requirement.derivedFrom.length > 0) {
        pieces.push(`derived from ${requirement.derivedFrom.join(", ")}`);
      }
      return pieces.join("; ");
    })
    .join("<br>");
};

const renderContractReport = () => {
  const suites = suiteIds.map((suiteId) => resolveSuiteForReport(suiteId));
  const sourceMap = new Map();
  for (const suite of suites) {
    for (const requirement of [...suite.requirements, ...suite.optionalRequirements]) {
      if (!requirement?.envName) continue;
      const existing = sourceMap.get(requirement.envName) || new Set();
      existing.add(sourceSummary(requirement.source));
      sourceMap.set(requirement.envName, existing);
      for (const alternative of Array.isArray(requirement.alternatives) ? requirement.alternatives : []) {
        const alternativeEnvName = normalizeText(alternative?.envName);
        if (!alternativeEnvName || alternativeEnvName === requirement.envName) continue;
        const alternativeExisting = sourceMap.get(alternativeEnvName) || new Set();
        alternativeExisting.add(sourceSummary(normalizeText(alternative?.source || "env")));
        sourceMap.set(alternativeEnvName, alternativeExisting);
      }
    }
  }
  const lines = [];
  lines.push("# CI Preflight + Artifact Redaction Contract");
  lines.push("");
  lines.push("Generated from `core/scripts/desktop_e2e_secret_contract.cjs`. Do not hand-edit this file.");
  lines.push("");
  lines.push("## Variables");
  lines.push("");
  lines.push("| Variable | Kind | Availability Source | Validation Rules |");
  lines.push("| --- | --- | --- | --- |");
  for (const [envName, spec] of Object.entries(envSpecs)) {
    const sources = Array.from(sourceMap.get(envName) || ["Suite-specific"]).join("<br>");
    lines.push(`| \`${envName}\` | ${spec.kind} | ${sources} | ${spec.validation.join("; ")} |`);
  }
  lines.push("");
  lines.push("## Suites");
  lines.push("");
  lines.push("| Suite | Required | Optional | Artifact Roots | Notes |");
  lines.push("| --- | --- | --- | --- | --- |");
  for (const suite of suites) {
    const notes = suite.notes.length > 0 ? suite.notes.join("<br>") : "";
    lines.push(`| \`${suite.id}\` | ${renderRequirementList(suite.requirements)} | ${renderRequirementList(suite.optionalRequirements)} | ${suite.artifactRoots.join("<br>") || "None"} | ${notes || " "} |`);
  }
  lines.push("");
  lines.push("## Dynamic Sources");
  lines.push("");
  const requiredLane = resolveSuiteForReport("provider-auth-matrix-required");
  const nightlyLane = resolveSuiteForReport("provider-auth-matrix-nightly");
  const breakMatrix = resolveSuiteForReport("macos-break-matrix");
  lines.push(`- Provider auth matrix required lane supported cells: ${requiredLane.metadata.supportedCellIds.join(", ") || "none"}`);
  lines.push(`- Provider auth matrix nightly lane supported cells: ${nightlyLane.metadata.supportedCellIds.join(", ") || "none"}`);
  lines.push(`- macOS break-matrix selected cases that currently imply auth secrets: ${breakMatrix.requirements.flatMap((requirement) => requirement.derivedFrom || []).filter(Boolean).join(", ") || "none"}`);
  lines.push("");
  lines.push("## Preflight Policy");
  lines.push("");
  lines.push("- Preflight is strict by default.");
  lines.push("- Local non-strict runs must opt in explicitly with `--allow-missing` (or the script env wrapper that passes it through).");
  lines.push("- Redaction scanning uses suite-specific secret env names plus named `KEY=value` and `Authorization: Bearer ...` leak patterns.");
  lines.push("");
  return `${lines.join("\n")}\n`;
};

module.exports = {
  buildRequirement,
  coreRoot,
  repoRoot,
  defaultAllowlistPath,
  defaultOpenRouterBaseUrl,
  defaultReportPath,
  envCarriesSecretPayload,
  envSpecs,
  renderContractReport,
  resolveRequirement,
  resolveSuiteContract,
  sourceSummary,
  suiteIds,
};
