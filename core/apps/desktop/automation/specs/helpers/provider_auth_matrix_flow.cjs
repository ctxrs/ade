const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { daemonJson } = require("./daemon.cjs");
const { selectSubscriptionSource } = require("./provider_runtime.cjs");

const trimText = (value) => String(value || "").trim();

const normalizeProviderLabel = (providerId) => {
  const normalized = trimText(providerId);
  if (!normalized) return "Provider";
  return normalized
    .split(/[-_]/g)
    .filter(Boolean)
    .map((token) => token.charAt(0).toUpperCase() + token.slice(1))
    .join(" ");
};

const readOptionalFile = (filePath, label) => {
  const resolved = trimText(filePath);
  if (!resolved) return { ok: false, reason: `${label} path is empty` };
  if (!fs.existsSync(resolved)) {
    return { ok: false, reason: `${label} file not found: ${resolved}` };
  }
  return {
    ok: true,
    value: fs.readFileSync(resolved, "utf8"),
    path: resolved,
  };
};

const readRequiredText = ({ rawEnv, pathEnv, label, env = process.env }) => {
  const raw = Object.prototype.hasOwnProperty.call(env, rawEnv) ? String(env[rawEnv] || "") : "";
  if (trimText(raw)) {
    return {
      ok: true,
      value: raw.trim(),
      source: "env",
      source_ref: rawEnv,
    };
  }
  const filePath = trimText(env[pathEnv]);
  if (filePath) {
    const file = readOptionalFile(filePath, label);
    if (!file.ok) return file;
    return {
      ok: true,
      value: trimText(file.value),
      source: "file",
      source_ref: file.path,
    };
  }
  return {
    ok: false,
    reason: `missing ${label}; set ${rawEnv} or ${pathEnv}`,
  };
};

const readOptionalText = ({ rawEnv, pathEnv, env = process.env }) => {
  const raw = Object.prototype.hasOwnProperty.call(env, rawEnv) ? String(env[rawEnv] || "") : "";
  if (trimText(raw)) {
    return {
      ok: true,
      value: raw.trim(),
      source: "env",
      source_ref: rawEnv,
    };
  }
  const filePath = trimText(env[pathEnv]);
  if (!filePath) return null;
  const file = readOptionalFile(filePath, rawEnv);
  if (!file.ok) return file;
  return {
    ok: true,
    value: trimText(file.value),
    source: "file",
    source_ref: file.path,
  };
};

const readSeedDirectory = ({ envName, label, env = process.env }) => {
  const dirPath = trimText(env[envName]);
  if (!dirPath) {
    return {
      ok: false,
      reason: `missing ${label}; set ${envName}`,
    };
  }
  if (!fs.existsSync(dirPath)) {
    return {
      ok: false,
      reason: `${label} directory not found: ${dirPath}`,
    };
  }
  const stat = fs.statSync(dirPath);
  if (!stat.isDirectory()) {
    return {
      ok: false,
      reason: `${label} is not a directory: ${dirPath}`,
    };
  }
  return {
    ok: true,
    path: dirPath,
    source: "dir",
    source_ref: dirPath,
  };
};

const resolveCodexSource = ({ env = process.env, homeDir = os.homedir() }) => {
  const explicitPath = trimText(env.CTX_CODEX_HOST_AUTH_PATH);
  const authPath = explicitPath || path.join(homeDir, ".codex", "auth.json");
  if (!fs.existsSync(authPath)) {
    return {
      status: "skip",
      reason: explicitPath
        ? `codex host auth file not found at ${authPath}`
        : `codex host auth file not found at ${authPath}; sign into Codex locally or set CTX_CODEX_HOST_AUTH_PATH`,
    };
  }
  return {
    status: "ready",
    plan: {
      providerId: "codex",
      strategy: "codex_host_import",
      hostAuthPath: authPath,
      label: `Codex Matrix ${Date.now()}`,
    },
  };
};

const resolveSubscriptionAuthPlan = (providerId, env = process.env, options = {}) => {
  const providerLabel = normalizeProviderLabel(providerId);
  switch (providerId) {
    case "codex":
      return resolveCodexSource({ env, homeDir: options.homeDir || os.homedir() });
    case "claude-crp": {
      const setupToken = readRequiredText({
        rawEnv: "CTX_E2E_CLAUDE_SETUP_TOKEN",
        pathEnv: "CTX_E2E_CLAUDE_SETUP_TOKEN_PATH",
        label: "Claude setup token",
        env,
      });
      if (!setupToken.ok) return { status: "skip", reason: setupToken.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "managed_upsert",
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            setup_token: setupToken.value,
          },
          sources: {
            setup_token: setupToken.source_ref,
          },
        },
      };
    }
    case "gemini": {
      const oauthCreds = readRequiredText({
        rawEnv: "CTX_E2E_GEMINI_OAUTH_CREDS_JSON",
        pathEnv: "CTX_E2E_GEMINI_OAUTH_CREDS_PATH",
        label: "Gemini oauth creds JSON",
        env,
      });
      if (!oauthCreds.ok) return { status: "skip", reason: oauthCreds.reason };
      const googleAccounts = readOptionalText({
        rawEnv: "CTX_E2E_GEMINI_GOOGLE_ACCOUNTS_JSON",
        pathEnv: "CTX_E2E_GEMINI_GOOGLE_ACCOUNTS_PATH",
        env,
      });
      if (googleAccounts && !googleAccounts.ok) return { status: "skip", reason: googleAccounts.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "managed_upsert",
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            oauth_creds_json: oauthCreds.value,
            google_accounts_json: googleAccounts ? googleAccounts.value : null,
            email: trimText(env.CTX_E2E_GEMINI_EMAIL) || null,
          },
          sources: {
            oauth_creds_json: oauthCreds.source_ref,
            google_accounts_json: googleAccounts ? googleAccounts.source_ref : null,
          },
        },
      };
    }
    case "qwen": {
      const oauthCreds = readRequiredText({
        rawEnv: "CTX_E2E_QWEN_OAUTH_CREDS_JSON",
        pathEnv: "CTX_E2E_QWEN_OAUTH_CREDS_PATH",
        label: "Qwen oauth creds JSON",
        env,
      });
      if (!oauthCreds.ok) return { status: "skip", reason: oauthCreds.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "managed_upsert",
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            oauth_creds_json: oauthCreds.value,
            email: trimText(env.CTX_E2E_QWEN_EMAIL) || null,
          },
          sources: {
            oauth_creds_json: oauthCreds.source_ref,
          },
        },
      };
    }
    case "kimi": {
      const credentials = readRequiredText({
        rawEnv: "CTX_E2E_KIMI_CREDENTIALS_JSON",
        pathEnv: "CTX_E2E_KIMI_CREDENTIALS_PATH",
        label: "Kimi credentials JSON",
        env,
      });
      if (!credentials.ok) return { status: "skip", reason: credentials.reason };
      const configToml = readOptionalText({
        rawEnv: "CTX_E2E_KIMI_CONFIG_TOML",
        pathEnv: "CTX_E2E_KIMI_CONFIG_TOML_PATH",
        env,
      });
      if (configToml && !configToml.ok) return { status: "skip", reason: configToml.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "managed_upsert",
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            provider: trimText(env.CTX_E2E_KIMI_PROVIDER) || null,
            credentials_json: credentials.value,
            config_toml: configToml ? configToml.value : null,
            email: trimText(env.CTX_E2E_KIMI_EMAIL) || null,
          },
          sources: {
            credentials_json: credentials.source_ref,
            config_toml: configToml ? configToml.source_ref : null,
          },
        },
      };
    }
    case "copilot": {
      const token = readRequiredText({
        rawEnv: "CTX_E2E_COPILOT_TOKEN",
        pathEnv: "CTX_E2E_COPILOT_TOKEN_PATH",
        label: "Copilot token",
        env,
      });
      if (!token.ok) return { status: "skip", reason: token.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "managed_upsert",
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            token: token.value,
            email: trimText(env.CTX_E2E_COPILOT_EMAIL) || null,
          },
          sources: {
            token: token.source_ref,
          },
        },
      };
    }
    case "cursor": {
      const apiKey = readRequiredText({
        rawEnv: "CTX_E2E_CURSOR_API_KEY",
        pathEnv: "CTX_E2E_CURSOR_API_KEY_PATH",
        label: "Cursor API key",
        env,
      });
      if (!apiKey.ok) return { status: "skip", reason: apiKey.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "managed_upsert",
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            token: apiKey.value,
            email: trimText(env.CTX_E2E_CURSOR_EMAIL) || null,
          },
          sources: {
            token: apiKey.source_ref,
          },
        },
      };
    }
    case "amp": {
      const homeSeed = trimText(env.CTX_E2E_AMP_HOME_SEED_DIR);
      if (homeSeed) {
        const resolved = readSeedDirectory({
          envName: "CTX_E2E_AMP_HOME_SEED_DIR",
          label: "Amp home seed",
          env,
        });
        if (!resolved.ok) return { status: "skip", reason: resolved.reason };
        return {
          status: "ready",
          plan: {
            providerId,
            strategy: "stage_amp_home",
            homeSeedDir: resolved.path,
            endpoint: `/api/providers/${providerId}/accounts`,
            body: {
              label: `${providerLabel} Matrix`,
              email: trimText(env.CTX_E2E_AMP_EMAIL) || null,
            },
            sources: {
              home_seed_dir: resolved.source_ref,
            },
          },
        };
      }
      const secrets = readRequiredText({
        rawEnv: "CTX_E2E_AMP_SECRETS_JSON",
        pathEnv: "CTX_E2E_AMP_SECRETS_PATH",
        label: "Amp secrets JSON",
        env,
      });
      if (!secrets.ok) return { status: "skip", reason: `${secrets.reason}; or set CTX_E2E_AMP_HOME_SEED_DIR` };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "stage_amp_secrets",
          secretsJson: secrets.value,
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            email: trimText(env.CTX_E2E_AMP_EMAIL) || null,
          },
          sources: {
            secrets_json: secrets.source_ref,
          },
        },
      };
    }
    case "mistral": {
      const homeSeed = readSeedDirectory({
        envName: "CTX_E2E_MISTRAL_HOME_SEED_DIR",
        label: "Mistral home seed",
        env,
      });
      if (!homeSeed.ok) return { status: "skip", reason: homeSeed.reason };
      return {
        status: "ready",
        plan: {
          providerId,
          strategy: "stage_mistral_home",
          homeSeedDir: homeSeed.path,
          endpoint: `/api/providers/${providerId}/accounts`,
          body: {
            label: `${providerLabel} Matrix`,
            email: trimText(env.CTX_E2E_MISTRAL_EMAIL) || null,
          },
          sources: {
            home_seed_dir: homeSeed.source_ref,
          },
        },
      };
    }
    default:
      return {
        status: "skip",
        reason: `subscription auth matrix flow is not implemented for provider ${providerId}`,
      };
  }
};

const ensureLocalDaemonDataDir = () => {
  const dataDir = trimText(process.env.CTX_DESKTOP_DAEMON_DATA_DIR || process.env.CTX_AUTOMATION_INTERNAL_DAEMON_DATA_DIR);
  if (!dataDir) {
    throw new Error("desktop daemon data dir is not available in automation env");
  }
  return dataDir;
};

const copyDirectory = (sourceDir, targetDir) => {
  fs.rmSync(targetDir, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(targetDir), { recursive: true });
  fs.cpSync(sourceDir, targetDir, { recursive: true });
};

const writeTextFile = (filePath, contents) => {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, String(contents || ""), "utf8");
};

const stageAmpRuntimeAuth = (plan, dataRoot) => {
  const ampHome = path.join(dataRoot, "providers", "amp", "home");
  if (plan.strategy === "stage_amp_home") {
    copyDirectory(plan.homeSeedDir, ampHome);
    return {
      amp_home: ampHome,
      staged_from: plan.homeSeedDir,
    };
  }

  const secretsPath = path.join(
    ampHome,
    ".local",
    "share",
    "amp",
    "secrets.json",
  );
  writeTextFile(secretsPath, plan.secretsJson);
  return {
    amp_home: ampHome,
    secrets_path: secretsPath,
  };
};

const stageMistralRuntimeHome = (plan, dataRoot) => {
  const mistralHome = path.join(dataRoot, "providers", "mistral", "home");
  copyDirectory(plan.homeSeedDir, mistralHome);
  return {
    mistral_home: mistralHome,
    staged_from: plan.homeSeedDir,
  };
};

const postJson = async (apiPath, body) => {
  const response = await daemonJson("POST", apiPath, body);
  if (response.status !== 200) {
    throw new Error(
      `POST ${apiPath} failed (${response.status}): ${JSON.stringify(response.payload || null)}`,
    );
  }
  return response.payload || null;
};

const getJson = async (apiPath) => {
  const response = await daemonJson("GET", apiPath);
  if (response.status !== 200) {
    throw new Error(
      `GET ${apiPath} failed (${response.status}): ${JSON.stringify(response.payload || null)}`,
    );
  }
  return response.payload || null;
};

const prepareSubscriptionAuth = async ({ providerId, envTarget }) => {
  const resolved = resolveSubscriptionAuthPlan(providerId);
  if (resolved.status !== "ready") {
    return resolved;
  }

  const plan = resolved.plan;
  const artifacts = {
    auth_plan: {
      provider_id: providerId,
      strategy: plan.strategy,
      sources: plan.sources || {},
    },
  };

  if (envTarget !== "local_host" && envTarget !== "local_container") {
    return {
      status: "skip",
      reason: `subscription auth matrix automation currently supports local env targets only (got ${envTarget})`,
      artifacts,
    };
  }

  if (plan.strategy === "codex_host_import") {
    const probe = await getJson("/api/providers/codex/import/host");
    artifacts.codex_import_probe = probe;
    if (probe.available !== true) {
      return {
        status: "skip",
        reason: trimText(probe.error) || "codex host import probe reported no usable auth candidate",
        artifacts,
      };
    }
    artifacts.account_response = await postJson("/api/providers/codex/import/host", {
      label: plan.label,
    });
  } else if (plan.strategy === "stage_amp_home" || plan.strategy === "stage_amp_secrets") {
    const dataRoot = ensureLocalDaemonDataDir();
    artifacts.runtime_stage = stageAmpRuntimeAuth(plan, dataRoot);
    artifacts.account_response = await postJson(plan.endpoint, plan.body);
  } else if (plan.strategy === "stage_mistral_home") {
    const dataRoot = ensureLocalDaemonDataDir();
    artifacts.runtime_stage = stageMistralRuntimeHome(plan, dataRoot);
    artifacts.account_response = await postJson(plan.endpoint, plan.body);
  } else {
    artifacts.account_response = await postJson(plan.endpoint, plan.body);
  }

  artifacts.subscription_source = await selectSubscriptionSource(providerId);
  artifacts.accounts_after_auth = await getJson(`/api/providers/${providerId}/accounts`);
  return {
    status: "ready",
    artifacts,
  };
};

module.exports = {
  resolveSubscriptionAuthPlan,
  prepareSubscriptionAuth,
};
