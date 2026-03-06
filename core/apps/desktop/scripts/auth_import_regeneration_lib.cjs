const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const SUPPORTED_PROVIDER_IDS = ["codex", "gemini", "qwen", "opencode", "amp"];
const ACCEPTABLE_IMPORT_STATUSES = new Set(["imported", "updated", "already_imported"]);
const SIGNAL_SCORES = {
  strong: 4,
  medium: 3,
  weak: 2,
  legacy: 1,
};
const CONFIDENCE_SCORES = {
  high: 4,
  medium: 3,
  "low-medium": 2,
  low: 1,
  legacy: 0,
};

const normalizeText = (value) =>
  String(value || "")
    .replace(/\s+/g, " ")
    .trim();

const asRecord = (value) =>
  value && typeof value === "object" && !Array.isArray(value) ? value : {};

const asArray = (value) => (Array.isArray(value) ? value : []);

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const splitCsv = (raw) =>
  String(raw || "")
    .split(",")
    .map((entry) => normalizeText(entry).toLowerCase())
    .filter(Boolean);

const ensureDir = (dirPath) => {
  fs.mkdirSync(dirPath, { recursive: true });
  return dirPath;
};

const defaultReportPath = () =>
  path.join(os.tmpdir(), `ctx-auth-import-regeneration-${Date.now()}.json`);

const resolvePath = (rawPath, cwd) => {
  const text = normalizeText(rawPath);
  if (!text) return "";
  return path.isAbsolute(text) ? text : path.resolve(cwd, text);
};

const parseArgs = (argv, env = process.env, cwd = process.cwd()) => {
  const opts = {
    baseUrl: normalizeText(env.CTX_DAEMON_URL || ""),
    token: normalizeText(env.CTX_DAEMON_AUTH_TOKEN || ""),
    providerIds: splitCsv(env.CTX_AUTH_IMPORT_PROVIDERS || "").length
      ? splitCsv(env.CTX_AUTH_IMPORT_PROVIDERS || "")
      : [...SUPPORTED_PROVIDER_IDS],
    candidateIds: splitCsv(env.CTX_AUTH_IMPORT_CANDIDATE_IDS || ""),
    reportPath: resolvePath(env.CTX_AUTH_IMPORT_REGEN_REPORT || "", cwd) || defaultReportPath(),
    workspaceRoot: resolvePath(env.CTX_AUTH_IMPORT_WORKSPACE_ROOT || "", cwd),
    listOnly: false,
    skipRuntime: ["1", "true", "yes"].includes(normalizeText(env.CTX_AUTH_IMPORT_SKIP_RUNTIME || "").toLowerCase()),
    installTarget: normalizeText(env.CTX_AUTH_IMPORT_INSTALL_TARGET || "host").toLowerCase() || "host",
    environment: normalizeText(env.CTX_AUTH_IMPORT_ENVIRONMENT || "host").toLowerCase() || "host",
    networkMode: normalizeText(env.CTX_AUTH_IMPORT_NETWORK_MODE || "all").toLowerCase() || "all",
    firstTurnTimeoutMs: parsePositiveInt(env.CTX_AUTH_IMPORT_FIRST_TURN_TIMEOUT_MS || "240000", 240000),
    help: false,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--base-url") {
      opts.baseUrl = normalizeText(argv[i + 1] || "");
      i += 1;
      continue;
    }
    if (arg === "--token") {
      opts.token = normalizeText(argv[i + 1] || "");
      i += 1;
      continue;
    }
    if (arg === "--provider") {
      opts.providerIds = splitCsv(argv[i + 1] || "");
      i += 1;
      continue;
    }
    if (arg === "--candidate-id") {
      opts.candidateIds.push(...splitCsv(argv[i + 1] || ""));
      i += 1;
      continue;
    }
    if (arg === "--report") {
      opts.reportPath = resolvePath(argv[i + 1] || "", cwd) || defaultReportPath();
      i += 1;
      continue;
    }
    if (arg === "--workspace-root") {
      opts.workspaceRoot = resolvePath(argv[i + 1] || "", cwd);
      i += 1;
      continue;
    }
    if (arg === "--install-target") {
      opts.installTarget = normalizeText(argv[i + 1] || "").toLowerCase();
      i += 1;
      continue;
    }
    if (arg === "--environment") {
      opts.environment = normalizeText(argv[i + 1] || "").toLowerCase();
      i += 1;
      continue;
    }
    if (arg === "--network-mode") {
      opts.networkMode = normalizeText(argv[i + 1] || "").toLowerCase();
      i += 1;
      continue;
    }
    if (arg === "--first-turn-timeout-ms") {
      opts.firstTurnTimeoutMs = parsePositiveInt(argv[i + 1] || "", opts.firstTurnTimeoutMs);
      i += 1;
      continue;
    }
    if (arg === "--list") {
      opts.listOnly = true;
      continue;
    }
    if (arg === "--skip-runtime") {
      opts.skipRuntime = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  if (!opts.providerIds.length && !opts.candidateIds.length) {
    opts.providerIds = [...SUPPORTED_PROVIDER_IDS];
  }

  return opts;
};

const usage = () => `Usage:
  node apps/desktop/scripts/auth_import_regeneration.cjs [options]

Options:
  --base-url URL              Daemon base URL. Default: CTX_DAEMON_URL
  --token TOKEN               Daemon bearer token. Default: CTX_DAEMON_AUTH_TOKEN
  --provider ID[,ID...]       Provider ids to evaluate. Default: codex,gemini,qwen,opencode,amp
  --candidate-id ID[,ID...]   Explicit candidate ids to import
  --report PATH               Output JSON report path
  --workspace-root DIR        Root directory for verification workspaces
  --install-target TARGET     Provider install target (host|container). Default: host
  --environment VALUE         Workspace execution environment. Default: host
  --network-mode VALUE        Workspace network mode. Default: all
  --first-turn-timeout-ms MS  First-turn timeout. Default: 240000
  --list                      List selected candidates and exit
  --skip-runtime              Skip install/verify/model/first-turn checks
  --help                      Show this help
`;

const compareCandidates = (left, right) => {
  const leftSignal = SIGNAL_SCORES[normalizeText(left.signal_strength).toLowerCase()] || 0;
  const rightSignal = SIGNAL_SCORES[normalizeText(right.signal_strength).toLowerCase()] || 0;
  if (leftSignal !== rightSignal) return rightSignal - leftSignal;

  const leftConfidence = CONFIDENCE_SCORES[normalizeText(left.confidence).toLowerCase()] || 0;
  const rightConfidence = CONFIDENCE_SCORES[normalizeText(right.confidence).toLowerCase()] || 0;
  if (leftConfidence !== rightConfidence) return rightConfidence - leftConfidence;

  const leftModified = Date.parse(left.last_modified || "") || 0;
  const rightModified = Date.parse(right.last_modified || "") || 0;
  if (leftModified !== rightModified) return rightModified - leftModified;

  return normalizeText(left.path).localeCompare(normalizeText(right.path));
};

const selectImportCandidates = (candidates, { providerIds, candidateIds }) => {
  const rows = asArray(candidates).map(asRecord);
  const selected = [];
  const warnings = [];
  const missing = [];

  if (candidateIds.length > 0) {
    for (const candidateId of candidateIds) {
      const found = rows.find((entry) => normalizeText(entry.id).toLowerCase() === candidateId);
      if (!found) {
        missing.push(`candidate:${candidateId}`);
        continue;
      }
      selected.push(found);
    }
    return { selected, warnings, missing };
  }

  for (const providerId of providerIds) {
    const parsed = rows
      .filter((entry) => normalizeText(entry.provider_id).toLowerCase() === providerId)
      .filter((entry) => normalizeText(entry.parse_status).toLowerCase() === "parsed")
      .sort(compareCandidates);
    if (!parsed.length) {
      missing.push(`provider:${providerId}`);
      continue;
    }
    selected.push(parsed[0]);
    if (parsed.length > 1) {
      warnings.push(
        `provider ${providerId}: selected ${normalizeText(parsed[0].path)} from ${parsed.length} parsed candidates`,
      );
    }
  }

  return { selected, warnings, missing };
};

const createDaemonClient = ({ baseUrl, token, fetchImpl = global.fetch }) => {
  const normalizedBaseUrl = normalizeText(baseUrl);
  const normalizedToken = normalizeText(token);
  if (!normalizedBaseUrl) {
    throw new Error("daemon base URL is required (use --base-url or CTX_DAEMON_URL)");
  }
  if (!normalizedToken) {
    throw new Error("daemon auth token is required (use --token or CTX_DAEMON_AUTH_TOKEN)");
  }
  if (typeof fetchImpl !== "function") {
    throw new Error("global fetch is not available in this Node runtime");
  }

  return {
    async request(method, apiPath, body) {
      const response = await fetchImpl(new URL(apiPath, normalizedBaseUrl), {
        method,
        headers: {
          authorization: `Bearer ${normalizedToken}`,
          "content-type": "application/json",
        },
        body: typeof body === "undefined" ? undefined : JSON.stringify(body),
      });
      const raw = await response.text();
      let payload = null;
      if (raw.trim()) {
        try {
          payload = JSON.parse(raw);
        } catch {
          payload = { raw };
        }
      }
      return { status: response.status, payload };
    },
  };
};

const requestJson = async (client, method, apiPath, body) => {
  const response = await client.request(method, apiPath, body);
  return {
    status: Number(response.status || 0),
    payload: response.payload,
  };
};

const expectOk = async (client, method, apiPath, body, actionLabel) => {
  const response = await requestJson(client, method, apiPath, body);
  if (response.status !== 200) {
    throw new Error(`${actionLabel} failed (${response.status}): ${JSON.stringify(response.payload || null)}`);
  }
  return response.payload;
};

const run = (cmd, args, opts = {}) => {
  const result = childProcess.spawnSync(cmd, args, { encoding: "utf8", ...opts });
  if (result.status !== 0) {
    throw new Error(
      [
        `command failed: ${cmd} ${args.join(" ")}`,
        normalizeText(result.stderr),
        normalizeText(result.stdout),
      ]
        .filter(Boolean)
        .join("\n"),
    );
  }
  return String(result.stdout || "");
};

const initGitRepo = (dir, name) => {
  ensureDir(dir);
  run("git", ["init", "--", dir]);
  run("git", ["-C", dir, "config", "user.email", "ctx-auth-import@example.com"]);
  run("git", ["-C", dir, "config", "user.name", "ctx-auth-import"]);
  fs.writeFileSync(path.join(dir, "README.md"), `# ${name}\n`, "utf8");
  run("git", ["-C", dir, "add", "README.md"]);
  run("git", ["-C", dir, "commit", "-m", "init"]);
};

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const ensureProviderInstalled = async (client, providerId, installTarget, timeoutMs = 600000) => {
  const providers = asArray(await expectOk(client, "GET", "/api/providers", undefined, "provider list"));
  const provider = providers.find((entry) => normalizeText(entry.provider_id) === providerId);
  if (provider?.installed === true) {
    return { installed: true, alreadyInstalled: true };
  }

  const started = await expectOk(
    client,
    "POST",
    `/api/providers/${providerId}/install?target=${encodeURIComponent(installTarget)}`,
    {},
    `provider install start for ${providerId}`,
  );
  const installId = normalizeText(started.install_id);
  if (!installId) {
    throw new Error(`provider install start missing install_id for ${providerId}`);
  }

  const startedAt = Date.now();
  let lastState = null;
  while (Date.now() - startedAt < timeoutMs) {
    const poll = await expectOk(
      client,
      "GET",
      `/api/providers/install/${encodeURIComponent(installId)}`,
      undefined,
      `provider install poll for ${providerId}`,
    );
    lastState = asRecord(poll);
    const state = normalizeText(poll.state).toLowerCase();
    if (state === "succeeded") {
      return { installed: true, alreadyInstalled: false, installId };
    }
    if (state === "failed" || state === "cancelled") {
      throw new Error(`provider install ${state} for ${providerId}: ${JSON.stringify(poll)}`);
    }
    await sleep(1000);
  }

  throw new Error(`provider install timed out for ${providerId}: ${JSON.stringify(lastState || null)}`);
};

const createWorkspaceAndLaunchExecution = async (
  client,
  { rootPath, name, environment, networkMode, timeoutMs = 900000 },
) => {
  initGitRepo(rootPath, name);
  const created = await expectOk(client, "POST", "/api/workspaces", {
    root_path: rootPath,
    name,
  }, "workspace create");
  const workspaceId = normalizeText(created.id);
  if (!workspaceId) {
    throw new Error(`workspace create response missing id: ${JSON.stringify(created || null)}`);
  }

  await expectOk(
    client,
    "POST",
    `/api/workspaces/${workspaceId}/execution_config`,
    {
      environment,
      network_mode: networkMode,
    },
    `execution config update for workspace ${workspaceId}`,
  );

  const launch = await expectOk(
    client,
    "POST",
    "/api/execution/launch/start",
    { workspace_id: workspaceId },
    `execution launch start for workspace ${workspaceId}`,
  );
  const jobId = normalizeText(launch.job_id);
  if (!jobId) {
    throw new Error(`execution launch response missing job_id: ${JSON.stringify(launch || null)}`);
  }

  const startedAt = Date.now();
  let lastStatus = null;
  while (Date.now() - startedAt < timeoutMs) {
    const status = await expectOk(
      client,
      "GET",
      `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`,
      undefined,
      `execution launch status for workspace ${workspaceId}`,
    );
    lastStatus = status;
    const state = normalizeText(status.state).toLowerCase();
    if (state === "ready") {
      return {
        workspaceId,
        launchJobId: jobId,
        launchStatus: status,
      };
    }
    if (state === "error") {
      throw new Error(`execution launch failed for workspace ${workspaceId}: ${JSON.stringify(status)}`);
    }
    await sleep(1000);
  }

  throw new Error(`execution launch timed out for workspace ${workspaceId}: ${JSON.stringify(lastStatus || null)}`);
};

const verifyProviderForWorkspace = async (client, workspaceId, providerId) => {
  const payload = await expectOk(
    client,
    "POST",
    `/api/workspaces/${workspaceId}/providers/${providerId}/verify`,
    {},
    `provider verify for ${providerId}`,
  );
  const status = normalizeText(payload.status).toLowerCase();
  if (status !== "ok") {
    throw new Error(`provider verify returned status=${status} for ${providerId}: ${JSON.stringify(payload)}`);
  }
  return payload;
};

const resolveWorkspaceProviderModelId = async (
  client,
  workspaceId,
  providerId,
  { timeoutMs = 90000, pollMs = 3000 } = {},
) => {
  const startedAt = Date.now();
  let lastOptions = null;
  while (Date.now() - startedAt < timeoutMs) {
    const payload = await expectOk(
      client,
      "GET",
      `/api/workspaces/${workspaceId}/providers/${providerId}/options`,
      undefined,
      `provider options for ${providerId}`,
    );
    lastOptions = payload;
    const models = asRecord(payload.models);
    const currentModelId = normalizeText(models.current_model_id || models.currentModelId || "");
    if (currentModelId) {
      return { modelId: currentModelId, options: payload };
    }
    const modelId = asArray(models.models)
      .map(asRecord)
      .map((entry) => normalizeText(entry.id || entry.model_id || entry.modelId || entry.name || ""))
      .find(Boolean);
    if (modelId) {
      return { modelId, options: payload };
    }
    await sleep(pollMs);
  }
  throw new Error(`provider models not populated for ${providerId}: ${JSON.stringify(lastOptions || null)}`);
};

const runProviderFirstTurnApiSmoke = async (
  client,
  workspaceId,
  providerId,
  modelId,
  timeoutMs,
) => {
  const task = await expectOk(client, "POST", `/api/workspaces/${workspaceId}/tasks`, {
    title: `auth-import-${providerId}-${Date.now()}`,
    description: "Auth import regeneration verification",
    create_default_session: false,
  }, `task create for ${providerId}`);
  const taskId = normalizeText(task.id);
  if (!taskId) {
    throw new Error(`task create response missing id for ${providerId}: ${JSON.stringify(task || null)}`);
  }

  const session = await expectOk(client, "POST", `/api/tasks/${taskId}/sessions`, {
    provider_id: providerId,
    model_id: modelId,
    env_target: "worktree",
  }, `session create for ${providerId}`);
  const sessionId = normalizeText(session.id);
  if (!sessionId) {
    throw new Error(`session create response missing id for ${providerId}: ${JSON.stringify(session || null)}`);
  }

  await expectOk(client, "POST", `/api/sessions/${sessionId}/messages`, {
    content: `auth-import-regeneration-${providerId}-${Date.now()}: reply with exactly pong`,
    delivery: "immediate",
    attachments: [],
  }, `message post for ${providerId}`);

  const startedAt = Date.now();
  let lastHistory = null;
  while (Date.now() - startedAt < timeoutMs) {
    const history = await expectOk(
      client,
      "GET",
      `/api/sessions/${sessionId}/history?limit=200`,
      undefined,
      `session history for ${providerId}`,
    );
    lastHistory = history;
    const assistantMessage = asArray(history.messages)
      .filter((entry) => normalizeText(entry.role).toLowerCase() === "assistant")
      .map((entry) => normalizeText(entry.content))
      .find(Boolean);
    if (assistantMessage) {
      return {
        taskId,
        sessionId,
        assistantMessage,
      };
    }

    const latestTurn = asArray(history.turns).slice(-1)[0];
    const latestStatus = normalizeText(latestTurn?.status).toLowerCase();
    if (latestStatus === "failed" || latestStatus === "cancelled") {
      throw new Error(`first turn failed for ${providerId}: ${JSON.stringify(latestTurn || null)}`);
    }
    await sleep(500);
  }

  throw new Error(`first turn timed out for ${providerId}: ${JSON.stringify(lastHistory || null)}`);
};

const isAccountBackedImport = (providerId, candidateKind) => {
  const provider = normalizeText(providerId);
  if (provider === "codex") return true;
  if (provider === "gemini" && normalizeText(candidateKind) === "auth_file") return true;
  return false;
};

const assertImportedProfileMetadata = ({ profiles, candidate, result }) => {
  const profileId = normalizeText(result.profile_id);
  if (!profileId) {
    throw new Error(`import result missing profile_id: ${JSON.stringify(result || null)}`);
  }
  const profile = asArray(profiles).find((entry) => normalizeText(entry.id) === profileId);
  if (!profile) {
    throw new Error(`profile metadata missing for profile_id=${profileId}`);
  }
  if (normalizeText(profile.provider_id) !== normalizeText(candidate.provider_id)) {
    throw new Error(
      `profile metadata provider mismatch for ${profileId}: expected=${candidate.provider_id} actual=${profile.provider_id}`,
    );
  }
  if (normalizeText(profile.source_path) !== normalizeText(candidate.path)) {
    throw new Error(
      `profile metadata source path mismatch for ${profileId}: expected=${candidate.path} actual=${profile.source_path}`,
    );
  }
  if (normalizeText(profile.source_kind) !== normalizeText(candidate.kind)) {
    throw new Error(
      `profile metadata source kind mismatch for ${profileId}: expected=${candidate.kind} actual=${profile.source_kind}`,
    );
  }
  return asRecord(profile);
};

const assertImportedProfileActive = async (client, { providerId, candidate, result }) => {
  const profileId = normalizeText(result.profile_id);
  if (isAccountBackedImport(providerId, candidate.kind)) {
    const accounts = asRecord(await expectOk(
      client,
      "GET",
      `/api/providers/${providerId}/accounts`,
      undefined,
      `provider accounts for ${providerId}`,
    ));
    const activeAccountId = normalizeText(accounts.active_account_id);
    if (activeAccountId !== profileId) {
      throw new Error(
        `active account mismatch for ${providerId}: expected=${profileId} actual=${activeAccountId || "<none>"}`,
      );
    }
    return {
      mode: "account",
      active_account_id: activeAccountId,
      accounts,
    };
  }

  const config = asRecord(await expectOk(
    client,
    "GET",
    `/api/providers/${providerId}/harness_config`,
    undefined,
    `provider harness config for ${providerId}`,
  ));
  const selectedEndpointId = normalizeText(config.selected_endpoint_id);
  if (normalizeText(config.selected_source_kind).toLowerCase() !== "endpoint") {
    throw new Error(`expected endpoint source for ${providerId}: ${JSON.stringify(config)}`);
  }
  if (selectedEndpointId !== profileId) {
    throw new Error(
      `selected endpoint mismatch for ${providerId}: expected=${profileId} actual=${selectedEndpointId || "<none>"}`,
    );
  }
  return {
    mode: "endpoint",
    selected_endpoint_id: selectedEndpointId,
    config,
  };
};

const createWorkspaceRoot = (workspaceRoot, providerId) => {
  if (workspaceRoot) {
    ensureDir(workspaceRoot);
    return path.join(workspaceRoot, `${providerId}-${Date.now()}`);
  }
  return fs.mkdtempSync(path.join(os.tmpdir(), `ctx-auth-import-${providerId}-`));
};

const runRegeneration = async (client, options) => {
  const startedAt = new Date().toISOString();
  const candidatesPayload = asArray(asRecord(await expectOk(
    client,
    "GET",
    "/api/providers/auth/import/candidates",
    undefined,
    "auth import candidate list",
  )).candidates);
  const selection = selectImportCandidates(candidatesPayload, {
    providerIds: options.providerIds,
    candidateIds: options.candidateIds,
  });
  if (!selection.selected.length) {
    throw new Error(
      `no importable auth candidates matched selection; missing=${selection.missing.join(",") || "<none>"}`,
    );
  }

  const report = {
    schema_version: 1,
    started_at: startedAt,
    completed_at: null,
    base_url: options.baseUrl,
    selected_candidates: selection.selected.map((entry) => ({
      id: entry.id,
      provider_id: entry.provider_id,
      kind: entry.kind,
      path: entry.path,
      parse_status: entry.parse_status,
      signal_strength: entry.signal_strength,
      confidence: entry.confidence,
      last_modified: entry.last_modified || null,
    })),
    warnings: selection.warnings,
    missing: selection.missing,
    list_only: Boolean(options.listOnly),
    skip_runtime: Boolean(options.skipRuntime),
    providers: [],
    result: "pass",
  };

  if (options.listOnly) {
    report.completed_at = new Date().toISOString();
    return report;
  }

  const importResults = asArray(asRecord(await expectOk(
    client,
    "POST",
    "/api/providers/auth/import",
    { candidate_ids: selection.selected.map((entry) => entry.id) },
    "auth import request",
  )).results);
  const profiles = asArray(asRecord(await expectOk(
    client,
    "GET",
    "/api/providers/auth/import/profiles",
    undefined,
    "auth import profile list",
  )).profiles);

  for (const candidate of selection.selected) {
    const result = asRecord(importResults.find((entry) => normalizeText(entry.candidate_id) === normalizeText(candidate.id)));
    const providerReport = {
      provider_id: normalizeText(candidate.provider_id),
      candidate_id: normalizeText(candidate.id),
      candidate_kind: normalizeText(candidate.kind),
      candidate_path: normalizeText(candidate.path),
      import_result: result,
      checks: [],
      status: "pass",
    };

    if (!ACCEPTABLE_IMPORT_STATUSES.has(normalizeText(result.status).toLowerCase())) {
      providerReport.status = "fail";
      providerReport.error = normalizeText(result.message || `unexpected import status: ${result.status}`);
      report.providers.push(providerReport);
      report.result = "fail";
      continue;
    }

    const profile = assertImportedProfileMetadata({ profiles, candidate, result });
    providerReport.imported_profile = profile;
    providerReport.checks.push("profile_metadata");

    const activeState = await assertImportedProfileActive(client, {
      providerId: providerReport.provider_id,
      candidate,
      result,
    });
    providerReport.active_state = activeState;
    providerReport.checks.push("active_profile");

    if (!options.skipRuntime) {
      const install = await ensureProviderInstalled(client, providerReport.provider_id, options.installTarget);
      providerReport.install = install;
      providerReport.checks.push("install");

      const workspaceRoot = createWorkspaceRoot(options.workspaceRoot, providerReport.provider_id);
      providerReport.workspace_root = workspaceRoot;
      const workspace = await createWorkspaceAndLaunchExecution(client, {
        rootPath: workspaceRoot,
        name: `auth-import-${providerReport.provider_id}-${Date.now()}`,
        environment: options.environment,
        networkMode: options.networkMode,
      });
      providerReport.workspace = workspace;
      providerReport.checks.push("workspace_launch");

      const verify = await verifyProviderForWorkspace(client, workspace.workspaceId, providerReport.provider_id);
      providerReport.verify = verify;
      providerReport.checks.push("verify");

      const model = await resolveWorkspaceProviderModelId(
        client,
        workspace.workspaceId,
        providerReport.provider_id,
      );
      providerReport.model = model;
      providerReport.checks.push("models");

      const firstTurn = await runProviderFirstTurnApiSmoke(
        client,
        workspace.workspaceId,
        providerReport.provider_id,
        model.modelId,
        options.firstTurnTimeoutMs,
      );
      providerReport.first_turn = firstTurn;
      providerReport.checks.push("first_turn");
    }

    report.providers.push(providerReport);
  }

  report.completed_at = new Date().toISOString();
  if (report.providers.some((entry) => entry.status !== "pass")) {
    report.result = "fail";
  }
  return report;
};

const writeReport = (reportPath, report) => {
  ensureDir(path.dirname(reportPath));
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8");
};

module.exports = {
  SUPPORTED_PROVIDER_IDS,
  parseArgs,
  usage,
  selectImportCandidates,
  createDaemonClient,
  runRegeneration,
  writeReport,
};
