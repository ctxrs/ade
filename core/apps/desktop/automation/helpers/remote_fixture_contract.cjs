const fs = require("node:fs");
const path = require("node:path");
const { parseBoolish, resolveBoolishFlag } = require("../../../../scripts/lib/boolish.cjs");

const normalizeText = (value) => String(value || "").trim();

const normalizeLowerText = (value) => normalizeText(value).toLowerCase();

const parseBoolean = (value) => parseBoolish(value) === true;

const parseDurationMs = (value, fallback) => {
  const text = normalizeText(value);
  if (!text) return fallback;
  const numeric = Number.parseInt(text, 10);
  return Number.isFinite(numeric) && numeric > 0 ? numeric : fallback;
};

const parsePort = (value, fallback = 0) => {
  const text = normalizeText(value);
  if (!text) return fallback;
  const numeric = Number.parseInt(text, 10);
  if (!Number.isFinite(numeric) || numeric < 1 || numeric > 65535) {
    return Number.NaN;
  }
  return numeric;
};

const firstPresentEnv = (env, names) => {
  for (const name of names) {
    const value = normalizeText(env[name]);
    if (value) {
      return { value, source: name };
    }
  }
  return { value: "", source: null };
};

const userHostParts = (hostInput) => {
  const raw = normalizeText(hostInput);
  if (!raw) return { user: "", host: "" };
  const atIndex = raw.lastIndexOf("@");
  if (atIndex <= 0 || atIndex >= raw.length - 1) {
    return { user: "", host: raw };
  }
  return {
    user: raw.slice(0, atIndex).trim(),
    host: raw.slice(atIndex + 1).trim(),
  };
};

const LANE_ENV_NAMES = {
  host: {
    host: ["CTX_AUTOMATION_REMOTE_HOST"],
    user: ["CTX_AUTOMATION_REMOTE_USER"],
    port: ["CTX_AUTOMATION_REMOTE_PORT"],
    dataDir: ["CTX_AUTOMATION_REMOTE_DATA_DIR"],
    password: ["CTX_AUTOMATION_REMOTE_PASSWORD"],
    passwordActual: ["CTX_AUTOMATION_REMOTE_PASSWORD_ACTUAL", "CTX_AUTOMATION_REMOTE_PASSWORD"],
    authMode: ["CTX_AUTOMATION_REMOTE_AUTH_MODE"],
    sshKeyPath: ["CTX_AUTOMATION_REMOTE_SSH_KEY_PATH", "CTX_UPDATER_E2E_SSH_KEY_PATH"],
    sshConfigPath: ["CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG"],
    sshPort: ["CTX_AUTOMATION_REMOTE_SSH_PORT"],
  },
  sandbox: {
    host: ["CTX_AUTOMATION_REMOTE_CONTAINER_HOST", "CTX_AUTOMATION_REMOTE_HOST"],
    user: ["CTX_AUTOMATION_REMOTE_CONTAINER_USER", "CTX_AUTOMATION_REMOTE_USER"],
    port: ["CTX_AUTOMATION_REMOTE_CONTAINER_PORT", "CTX_AUTOMATION_REMOTE_PORT"],
    dataDir: ["CTX_AUTOMATION_REMOTE_CONTAINER_DATA_DIR", "CTX_AUTOMATION_REMOTE_DATA_DIR"],
    password: ["CTX_AUTOMATION_REMOTE_CONTAINER_PASSWORD", "CTX_AUTOMATION_REMOTE_PASSWORD"],
    passwordActual: [
      "CTX_AUTOMATION_REMOTE_CONTAINER_PASSWORD_ACTUAL",
      "CTX_AUTOMATION_REMOTE_PASSWORD_ACTUAL",
      "CTX_AUTOMATION_REMOTE_CONTAINER_PASSWORD",
      "CTX_AUTOMATION_REMOTE_PASSWORD",
    ],
    authMode: ["CTX_AUTOMATION_REMOTE_CONTAINER_AUTH_MODE", "CTX_AUTOMATION_REMOTE_AUTH_MODE"],
    sshKeyPath: [
      "CTX_AUTOMATION_REMOTE_CONTAINER_SSH_KEY_PATH",
      "CTX_AUTOMATION_REMOTE_SSH_KEY_PATH",
      "CTX_UPDATER_E2E_SSH_KEY_PATH",
    ],
    sshConfigPath: [
      "CTX_AUTOMATION_REMOTE_CONTAINER_FIXTURE_SSH_CONFIG",
      "CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG",
    ],
    sshPort: ["CTX_AUTOMATION_REMOTE_CONTAINER_SSH_PORT", "CTX_AUTOMATION_REMOTE_SSH_PORT"],
  },
};

const envHint = (names) => names.join(" or ");

const resolveRemoteProofScope = ({
  lane = "host",
  fixtureClass = "",
} = {}) => {
  const normalizedLane = lane === "sandbox" ? "sandbox" : "host";
  const normalizedFixtureClass = normalizeLowerText(fixtureClass);
  if (normalizedFixtureClass === "docker-ssh") {
    return normalizedLane === "sandbox"
      ? "docker_warm_remote_sandbox"
      : "docker_warm_remote_host";
  }
  return normalizedLane === "sandbox" ? "remote_sandbox" : "remote_host";
};

const resolveRemotePerformanceBudgets = ({
  fixture = {},
  env = process.env,
} = {}) => {
  const fixtureClass = normalizeLowerText(fixture.fixtureClass);
  const dockerFixture = fixtureClass === "docker-ssh";
  return {
    cold_connect_ms: parseDurationMs(
      env.CTX_AUTOMATION_REMOTE_COLD_CONNECT_MAX_MS,
      dockerFixture ? 180_000 : 240_000,
    ),
    warm_connect_ms: parseDurationMs(
      env.CTX_AUTOMATION_REMOTE_WARM_CONNECT_MAX_MS,
      dockerFixture ? 60_000 : 120_000,
    ),
    daemon_restart_connect_ms: parseDurationMs(
      env.CTX_AUTOMATION_REMOTE_DAEMON_RESTART_CONNECT_MAX_MS,
      dockerFixture ? 120_000 : 180_000,
    ),
    provider_ready_ms: parseDurationMs(
      env.CTX_AUTOMATION_REMOTE_PROVIDER_READY_MAX_MS,
      dockerFixture ? 45_000 : 90_000,
    ),
    terminal_ready_ms: parseDurationMs(
      env.CTX_AUTOMATION_REMOTE_TERMINAL_READY_MAX_MS,
      dockerFixture ? 30_000 : 60_000,
    ),
    task_create_ms: parseDurationMs(
      env.CTX_AUTOMATION_REMOTE_TASK_CREATE_MAX_MS,
      dockerFixture ? 20_000 : 45_000,
    ),
  };
};

const summarizeFixtureForReport = (fixture) => ({
  lane: fixture.lane,
  host: fixture.host || null,
  host_input: fixture.hostInput || null,
  user: fixture.user || null,
  target: fixture.target || null,
  wizard_host_input: fixture.wizardHostInput || null,
  remote_port: Number.isFinite(fixture.port) && fixture.port > 0 ? fixture.port : null,
  remote_data_dir: fixture.dataDir || null,
  auth_mode: fixture.authMode || null,
  ssh_key_path: fixture.sshKeyPath || null,
  ssh_config_path: fixture.sshConfigPath || null,
  ssh_port: Number.isFinite(fixture.sshPort) && fixture.sshPort > 0 ? fixture.sshPort : null,
  strict_required: fixture.strictRequired,
  allow_skip: fixture.allowSkip,
  ready: fixture.ready,
  missing_requirements: fixture.missingRequirements,
  host_mode: fixture.hostMode || null,
  fixture_class: fixture.fixtureClass || null,
  sandbox_runtime: fixture.sandboxRuntime || null,
  proof_scope: fixture.proofScope || null,
  env_sources: fixture.sources,
});

const resolveRemoteFixtureEnv = ({
  lane = "host",
  env = process.env,
  defaultPort = 44099,
} = {}) => {
  const laneKey = lane === "host" ? "host" : "sandbox";
  const names = LANE_ENV_NAMES[laneKey];
  const hostMatch = firstPresentEnv(env, names.host);
  const userMatch = firstPresentEnv(env, names.user);
  const portMatch = firstPresentEnv(env, names.port);
  const dataDirMatch = firstPresentEnv(env, names.dataDir);
  const passwordMatch = firstPresentEnv(env, names.password);
  const passwordActualMatch = firstPresentEnv(env, names.passwordActual);
  const authModeMatch = firstPresentEnv(env, names.authMode);
  const sshKeyPathMatch = firstPresentEnv(env, names.sshKeyPath);
  const sshConfigPathMatch = firstPresentEnv(env, names.sshConfigPath);
  const sshPortMatch = firstPresentEnv(env, names.sshPort);

  const parsedHost = userHostParts(hostMatch.value);
  const host = parsedHost.host;
  const user = userMatch.value || parsedHost.user;
  const port = parsePort(portMatch.value, defaultPort);
  const sshPort = parsePort(sshPortMatch.value, 0);
  const strictRequested = resolveBoolishFlag(
    env.CTX_AUTOMATION_REMOTE_STRICT,
    false,
    "CTX_AUTOMATION_REMOTE_STRICT",
  );
  const allowSkip = resolveBoolishFlag(
    env.CTX_AUTOMATION_REMOTE_ALLOW_SKIP,
    false,
    "CTX_AUTOMATION_REMOTE_ALLOW_SKIP",
  );

  const missingRequirements = [];
  if (!host) {
    missingRequirements.push(envHint(names.host));
  }
  if (!user) {
    missingRequirements.push(`${envHint(names.user)} or embed user in ${names.host[0]}`);
  }
  if (!dataDirMatch.value) {
    missingRequirements.push(envHint(names.dataDir));
  }
  if (Number.isNaN(port)) {
    missingRequirements.push(`${envHint(names.port)} must be a valid TCP port`);
  }
  if (Number.isNaN(sshPort)) {
    missingRequirements.push(`${envHint(names.sshPort)} must be a valid TCP port`);
  }

  const wizardHostInput = host
    ? (parsedHost.user ? hostMatch.value : `${user}@${host}`)
    : "";
  const target = host && user ? `${user}@${host}` : "";
  const hostMode = normalizeLowerText(env.CTX_AUTOMATION_REMOTE_FIXTURE_HOST_MODE) || "fresh-install";
  const fixtureClass = normalizeLowerText(env.CTX_AUTOMATION_REMOTE_FIXTURE_CLASS);
  const sandboxRuntime = normalizeLowerText(env.CTX_AUTOMATION_REMOTE_FIXTURE_SANDBOX_RUNTIME);
  const proofScope = resolveRemoteProofScope({
    lane: laneKey,
    fixtureClass,
  });

  return {
    lane: laneKey,
    hostInput: hostMatch.value,
    host,
    user,
    target,
    wizardHostInput,
    port: Number.isNaN(port) ? 0 : port,
    dataDir: dataDirMatch.value,
    password: passwordMatch.value,
    passwordActual: passwordActualMatch.value,
    authMode: normalizeLowerText(authModeMatch.value),
    sshKeyPath: sshKeyPathMatch.value,
    sshConfigPath: sshConfigPathMatch.value,
    sshPort: Number.isNaN(sshPort) ? 0 : sshPort,
    ready: missingRequirements.length === 0,
    strictRequired: strictRequested && !allowSkip,
    allowSkip,
    missingRequirements,
    hostMode,
    fixtureClass,
    sandboxRuntime,
    proofScope,
    preflightMessage:
      missingRequirements.length === 0
        ? ""
        : `${laneKey} remote fixture is missing required configuration: ${missingRequirements.join(", ")}`,
    sources: {
      host: hostMatch.source,
      user: userMatch.source,
      port: portMatch.source,
      data_dir: dataDirMatch.source,
      password: passwordMatch.source,
      password_actual: passwordActualMatch.source,
      auth_mode: authModeMatch.source,
      ssh_key_path: sshKeyPathMatch.source,
      ssh_config_path: sshConfigPathMatch.source,
      ssh_port: sshPortMatch.source,
    },
  };
};

const createRedactor = (secretValues = []) => {
  const uniqueSecrets = Array.from(
    new Set(
      secretValues
        .map((value) => normalizeText(value))
        .filter((value) => value.length > 0),
    ),
  ).sort((left, right) => right.length - left.length);

  const redactString = (value) => {
    let text = String(value || "");
    for (const secret of uniqueSecrets) {
      text = text.split(secret).join("[REDACTED]");
    }
    return text;
  };

  const redactValue = (value) => {
    if (typeof value === "string") {
      return redactString(value);
    }
    if (Array.isArray(value)) {
      return value.map((entry) => redactValue(entry));
    }
    if (!value || typeof value !== "object") {
      return value;
    }
    const out = {};
    for (const [key, entry] of Object.entries(value)) {
      out[key] = redactValue(entry);
    }
    return out;
  };

  return {
    redactString,
    redactValue,
  };
};

const writeJsonFile = (outputPath, payload) => {
  if (!outputPath) return;
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
};

const createRemoteContractRecorder = ({
  outputPath,
  suite,
  lane,
  fixture,
  secretValues = [],
} = {}) => {
  const startedAt = new Date().toISOString();
  const assertions = [];
  const artifacts = {};
  const sshTranscripts = [];
  const { redactValue } = createRedactor(secretValues);

  const recordAssertion = (name, status, detail = "", payload = null) => {
    assertions.push({
      name: normalizeText(name),
      status: normalizeLowerText(status) || "unknown",
      detail: normalizeText(detail),
      at: new Date().toISOString(),
      payload: redactValue(payload),
    });
  };

  const recordArtifact = (name, payload) => {
    artifacts[normalizeText(name)] = redactValue(payload);
  };

  const recordSshTranscript = (label, payload = {}) => {
    sshTranscripts.push({
      label: normalizeText(label),
      at: new Date().toISOString(),
      ...redactValue(payload),
    });
  };

  const finalize = ({ result, reason = "", error = "", extras = {} } = {}) => {
    const normalizedResult = normalizeLowerText(result) || "unknown";
    const report = {
      schema_version: 1,
      started_at: startedAt,
      completed_at: new Date().toISOString(),
      suite: normalizeText(suite),
      lane: normalizeText(lane),
      result: normalizedResult,
      skipped: normalizedResult === "skipped",
      skip_reason: normalizedResult === "skipped" ? redactValue(normalizeText(reason)) : "",
      reason: redactValue(normalizeText(reason)),
      error: redactValue(normalizeText(error)),
      fixture: redactValue(summarizeFixtureForReport(fixture || {})),
      assertions,
      artifacts,
      ssh_transcripts: sshTranscripts,
      extras: redactValue(extras),
    };
    writeJsonFile(outputPath, report);
    return report;
  };

  return {
    recordAssertion,
    recordArtifact,
    recordSshTranscript,
    finalize,
  };
};

module.exports = {
  createRemoteContractRecorder,
  createRedactor,
  normalizeText,
  parseBoolean,
  parseDurationMs,
  parsePort,
  resolveRemoteFixtureEnv,
  resolveRemotePerformanceBudgets,
  resolveRemoteProofScope,
  summarizeFixtureForReport,
  writeJsonFile,
};
