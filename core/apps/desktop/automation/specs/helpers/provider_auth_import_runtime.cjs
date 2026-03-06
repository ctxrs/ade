const fs = require("node:fs");
const path = require("node:path");

const { daemonJson } = require("./daemon.cjs");

const normalizeText = (value) =>
  String(value || "")
    .replace(/\s+/g, " ")
    .trim();

const asRecord = (value) =>
  value && typeof value === "object" && !Array.isArray(value) ? value : {};

const asArray = (value) => (Array.isArray(value) ? value : []);

const envPath = (key, fallbackPath) => {
  const raw = normalizeText(process.env[key] || "");
  return raw || fallbackPath;
};

const decodeBase64Utf8 = (key) => {
  const raw = normalizeText(process.env[key] || "");
  if (!raw) return "";
  return Buffer.from(raw, "base64").toString("utf8");
};

const readUtf8Payload = (base64Key, plainKey) => {
  const fromBase64 = decodeBase64Utf8(base64Key);
  if (fromBase64) return fromBase64;
  return String(process.env[plainKey] || "");
};

const writeUtf8 = (targetPath, contents) => {
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, contents, "utf8");
  return targetPath;
};

const stageCodexImportFixture = () => {
  const authJson = readUtf8Payload(
    "CTX_E2E_AUTH_IMPORT_CODEX_AUTH_JSON_B64",
    "CTX_E2E_AUTH_IMPORT_CODEX_AUTH_JSON",
  );
  if (!normalizeText(authJson)) {
    throw new Error("CTX_E2E_AUTH_IMPORT_CODEX_AUTH_JSON_B64 is required for codex auth-import coverage");
  }
  const codexHome = envPath("CTX_PROVIDER_AUTH_IMPORT_CODEX_HOME", path.join(process.cwd(), ".tmp-codex-home"));
  const fixturePath = writeUtf8(path.join(codexHome, "auth.json"), authJson);
  return { providerId: "codex", kind: "auth_file", expectedPath: fixturePath };
};

const stageGeminiImportFixture = () => {
  const geminiHome = envPath("CTX_PROVIDER_AUTH_IMPORT_HOME", path.join(process.cwd(), ".tmp-auth-import-home"));
  const oauthJson = readUtf8Payload(
    "CTX_E2E_AUTH_IMPORT_GEMINI_OAUTH_JSON_B64",
    "CTX_E2E_AUTH_IMPORT_GEMINI_OAUTH_JSON",
  );
  if (normalizeText(oauthJson)) {
    const fixturePath = writeUtf8(path.join(geminiHome, ".gemini", "oauth_creds.json"), oauthJson);
    const googleAccountsJson = readUtf8Payload(
      "CTX_E2E_AUTH_IMPORT_GEMINI_GOOGLE_ACCOUNTS_JSON_B64",
      "CTX_E2E_AUTH_IMPORT_GEMINI_GOOGLE_ACCOUNTS_JSON",
    );
    if (normalizeText(googleAccountsJson)) {
      writeUtf8(path.join(geminiHome, ".gemini", "google_accounts.json"), googleAccountsJson);
    }
    return { providerId: "gemini", kind: "auth_file", expectedPath: fixturePath };
  }

  const envContents = readUtf8Payload(
    "CTX_E2E_AUTH_IMPORT_GEMINI_ENV_B64",
    "CTX_E2E_AUTH_IMPORT_GEMINI_ENV",
  );
  if (!normalizeText(envContents)) {
    throw new Error(
      "CTX_E2E_AUTH_IMPORT_GEMINI_ENV_B64 is required unless Gemini OAuth fixture env vars are supplied",
    );
  }
  const fixturePath = writeUtf8(path.join(geminiHome, ".gemini", ".env"), envContents);
  return { providerId: "gemini", kind: "env_file", expectedPath: fixturePath };
};

const stageQwenImportFixture = () => {
  const envContents = readUtf8Payload(
    "CTX_E2E_AUTH_IMPORT_QWEN_ENV_B64",
    "CTX_E2E_AUTH_IMPORT_QWEN_ENV",
  );
  if (!normalizeText(envContents)) {
    throw new Error("CTX_E2E_AUTH_IMPORT_QWEN_ENV_B64 is required for qwen auth-import coverage");
  }
  const importHome = envPath("CTX_PROVIDER_AUTH_IMPORT_HOME", path.join(process.cwd(), ".tmp-auth-import-home"));
  const fixturePath = writeUtf8(path.join(importHome, ".qwen", ".env"), envContents);
  return { providerId: "qwen", kind: "env_file", expectedPath: fixturePath };
};

const stageOpencodeImportFixture = () => {
  const authJson = readUtf8Payload(
    "CTX_E2E_AUTH_IMPORT_OPENCODE_AUTH_JSON_B64",
    "CTX_E2E_AUTH_IMPORT_OPENCODE_AUTH_JSON",
  );
  if (!normalizeText(authJson)) {
    throw new Error("CTX_E2E_AUTH_IMPORT_OPENCODE_AUTH_JSON_B64 is required for opencode auth-import coverage");
  }
  const xdgDataHome = envPath(
    "CTX_PROVIDER_AUTH_IMPORT_XDG_DATA_HOME",
    path.join(process.cwd(), ".tmp-auth-import-data"),
  );
  const fixturePath = writeUtf8(path.join(xdgDataHome, "opencode", "auth.json"), authJson);
  return { providerId: "opencode", kind: "auth_file", expectedPath: fixturePath };
};

const stageAmpImportFixture = () => {
  const authJson = readUtf8Payload(
    "CTX_E2E_AUTH_IMPORT_AMP_AUTH_JSON_B64",
    "CTX_E2E_AUTH_IMPORT_AMP_AUTH_JSON",
  );
  if (!normalizeText(authJson)) {
    throw new Error("CTX_E2E_AUTH_IMPORT_AMP_AUTH_JSON_B64 is required for amp auth-import coverage");
  }
  const importHome = envPath("CTX_PROVIDER_AUTH_IMPORT_HOME", path.join(process.cwd(), ".tmp-auth-import-home"));
  const fixturePath = writeUtf8(path.join(importHome, ".amp", "oauth", "imported-profile.json"), authJson);
  return { providerId: "amp", kind: "auth_file", expectedPath: fixturePath };
};

const stageProviderAuthImportFixture = (providerId) => {
  switch (normalizeText(providerId)) {
    case "codex":
      return stageCodexImportFixture();
    case "gemini":
      return stageGeminiImportFixture();
    case "qwen":
      return stageQwenImportFixture();
    case "opencode":
      return stageOpencodeImportFixture();
    case "amp":
      return stageAmpImportFixture();
    default:
      throw new Error(`auth-import fixture staging is not implemented for provider_id=${providerId}`);
  }
};

const listProviderAuthImportCandidates = async () => {
  const response = await daemonJson("GET", "/api/providers/auth/import/candidates");
  if (response.status !== 200) {
    throw new Error(`auth import candidate list failed (${response.status}): ${JSON.stringify(response.payload || null)}`);
  }
  return asArray(response.payload?.candidates);
};

const importProviderAuthCandidates = async (candidateIds) => {
  const response = await daemonJson("POST", "/api/providers/auth/import", {
    candidate_ids: candidateIds,
  });
  if (response.status !== 200) {
    throw new Error(`auth import request failed (${response.status}): ${JSON.stringify(response.payload || null)}`);
  }
  return asArray(response.payload?.results);
};

const listProviderAuthImportProfiles = async () => {
  const response = await daemonJson("GET", "/api/providers/auth/import/profiles");
  if (response.status !== 200) {
    throw new Error(`auth import profile list failed (${response.status}): ${JSON.stringify(response.payload || null)}`);
  }
  return asArray(response.payload?.profiles);
};

const getProviderAccounts = async (providerId) => {
  const response = await daemonJson("GET", `/api/providers/${providerId}/accounts`);
  if (response.status !== 200) {
    throw new Error(`provider accounts read failed for ${providerId} (${response.status}): ${JSON.stringify(response.payload || null)}`);
  }
  return asRecord(response.payload);
};

const getProviderHarnessConfig = async (providerId) => {
  const response = await daemonJson("GET", `/api/providers/${providerId}/harness_config`);
  if (response.status !== 200) {
    throw new Error(`provider harness config read failed for ${providerId} (${response.status}): ${JSON.stringify(response.payload || null)}`);
  }
  return asRecord(response.payload);
};

const findStagedCandidate = (candidates, stagedFixture) => {
  const match = asArray(candidates).find((candidate) => (
    normalizeText(candidate.provider_id) === stagedFixture.providerId
    && normalizeText(candidate.kind) === stagedFixture.kind
    && normalizeText(candidate.path) === normalizeText(stagedFixture.expectedPath)
  ));
  if (!match) {
    throw new Error(
      `expected staged auth import candidate was not detected: ${JSON.stringify(stagedFixture)}`,
    );
  }
  return match;
};

const assertImportedProfileMetadata = ({ profiles, candidate, result }) => {
  const profileId = normalizeText(result?.profile_id || "");
  if (!profileId) {
    throw new Error(`import result missing profile_id: ${JSON.stringify(result || null)}`);
  }
  const profile = asArray(profiles).find((entry) => normalizeText(entry.id) === profileId);
  if (!profile) {
    throw new Error(`imported profile metadata missing for profile_id=${profileId}`);
  }
  if (normalizeText(profile.provider_id) !== normalizeText(candidate.provider_id)) {
    throw new Error(
      `imported profile provider mismatch: expected=${candidate.provider_id} actual=${profile.provider_id}`,
    );
  }
  if (normalizeText(profile.source_path) !== normalizeText(candidate.path)) {
    throw new Error(
      `imported profile source path mismatch: expected=${candidate.path} actual=${profile.source_path}`,
    );
  }
  if (normalizeText(profile.source_kind) !== normalizeText(candidate.kind)) {
    throw new Error(
      `imported profile source kind mismatch: expected=${candidate.kind} actual=${profile.source_kind}`,
    );
  }
  if (!normalizeText(profile.secret_fingerprint)) {
    throw new Error(`imported profile metadata missing secret_fingerprint for profile_id=${profileId}`);
  }
  return profile;
};

const isAccountBackedImport = (providerId, candidateKind) => {
  const provider = normalizeText(providerId);
  if (provider === "codex") return true;
  if (provider === "gemini" && normalizeText(candidateKind) === "auth_file") return true;
  return false;
};

const assertImportedProfileActive = async ({ providerId, candidate, result }) => {
  const profileId = normalizeText(result?.profile_id || "");
  if (!profileId) {
    throw new Error(`import result missing profile_id for active-profile assertion: ${JSON.stringify(result || null)}`);
  }

  if (isAccountBackedImport(providerId, candidate.kind)) {
    const accounts = getProviderAccounts(providerId);
    const payload = await accounts;
    const activeAccountId = normalizeText(payload.active_account_id || "");
    if (activeAccountId !== profileId) {
      throw new Error(
        `active account mismatch for ${providerId}: expected=${profileId} actual=${activeAccountId || "<none>"}`,
      );
    }
    const account = asArray(payload.accounts).find((entry) => normalizeText(entry.id) === profileId);
    if (!account) {
      throw new Error(`active account ${profileId} missing from ${providerId} accounts payload`);
    }
    return {
      mode: "account",
      active_account_id: activeAccountId,
      account: asRecord(account),
      accounts: payload,
    };
  }

  const config = await getProviderHarnessConfig(providerId);
  const selectedSourceKind = normalizeText(config.selected_source_kind || "").toLowerCase();
  const selectedEndpointId = normalizeText(config.selected_endpoint_id || "");
  if (selectedSourceKind !== "endpoint") {
    throw new Error(`expected endpoint source for ${providerId}, got '${selectedSourceKind || "unknown"}'`);
  }
  if (selectedEndpointId !== profileId) {
    throw new Error(
      `selected endpoint mismatch for ${providerId}: expected=${profileId} actual=${selectedEndpointId || "<none>"}`,
    );
  }
  const endpoint = asArray(config.endpoints).find((entry) => normalizeText(entry.id) === profileId);
  if (!endpoint) {
    throw new Error(`selected endpoint ${profileId} missing from ${providerId} harness config`);
  }
  if (endpoint.has_api_key !== true) {
    throw new Error(`selected endpoint ${profileId} for ${providerId} is missing API key material`);
  }
  return {
    mode: "endpoint",
    selected_endpoint_id: selectedEndpointId,
    endpoint: asRecord(endpoint),
    config,
  };
};

module.exports = {
  normalizeText,
  stageProviderAuthImportFixture,
  listProviderAuthImportCandidates,
  importProviderAuthCandidates,
  listProviderAuthImportProfiles,
  findStagedCandidate,
  assertImportedProfileMetadata,
  assertImportedProfileActive,
};
