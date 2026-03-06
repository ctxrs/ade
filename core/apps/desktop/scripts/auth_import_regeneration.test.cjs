const test = require("node:test");
const assert = require("node:assert/strict");

const {
  parseArgs,
  selectImportCandidates,
  runRegeneration,
} = require("./auth_import_regeneration_lib.cjs");

const createFakeClient = (handlers) => ({
  async request(method, apiPath, body) {
    const key = `${method} ${apiPath}`;
    const handler = handlers[key];
    if (!handler) {
      throw new Error(`unexpected request: ${key} body=${JSON.stringify(body || null)}`);
    }
    const result = typeof handler === "function" ? await handler(body) : handler;
    return {
      status: result.status,
      payload: result.payload,
    };
  },
});

test("parseArgs falls back to canonical providers", () => {
  const opts = parseArgs([], {}, process.cwd());
  assert.deepEqual(opts.providerIds, ["codex", "gemini", "qwen", "opencode", "amp"]);
  assert.equal(opts.skipRuntime, false);
});

test("selectImportCandidates prefers the strongest parsed candidate per provider", () => {
  const selection = selectImportCandidates(
    [
      {
        id: "gemini-env",
        provider_id: "gemini",
        kind: "env_file",
        path: "/tmp/.gemini/.env",
        parse_status: "parsed",
        signal_strength: "strong",
        confidence: "medium",
        last_modified: "2026-03-05T11:00:00Z",
      },
      {
        id: "gemini-oauth",
        provider_id: "gemini",
        kind: "auth_file",
        path: "/tmp/.gemini/oauth_creds.json",
        parse_status: "parsed",
        signal_strength: "weak",
        confidence: "high",
        last_modified: "2026-03-05T12:00:00Z",
      },
    ],
    {
      providerIds: ["gemini"],
      candidateIds: [],
    },
  );

  assert.equal(selection.selected.length, 1);
  assert.equal(selection.selected[0].id, "gemini-env");
  assert.equal(selection.warnings.length, 1);
});

test("runRegeneration validates account-backed imports via provider accounts", async () => {
  const client = createFakeClient({
    "GET /api/providers/auth/import/candidates": {
      status: 200,
      payload: {
        candidates: [
          {
            id: "codex-candidate",
            provider_id: "codex",
            kind: "auth_file",
            path: "/tmp/.codex/auth.json",
            parse_status: "parsed",
            signal_strength: "strong",
            confidence: "high",
          },
        ],
      },
    },
    "POST /api/providers/auth/import": {
      status: 200,
      payload: {
        results: [
          {
            candidate_id: "codex-candidate",
            provider_id: "codex",
            status: "imported",
            profile_id: "acct-1",
          },
        ],
      },
    },
    "GET /api/providers/auth/import/profiles": {
      status: 200,
      payload: {
        profiles: [
          {
            id: "acct-1",
            provider_id: "codex",
            source_path: "/tmp/.codex/auth.json",
            source_kind: "auth_file",
            secret_fingerprint: "fp-1",
          },
        ],
      },
    },
    "GET /api/providers/codex/accounts": {
      status: 200,
      payload: {
        active_account_id: "acct-1",
        accounts: [{ id: "acct-1", label: "Imported Codex profile" }],
      },
    },
  });

  const report = await runRegeneration(client, {
    baseUrl: "http://127.0.0.1:4399",
    token: "token",
    providerIds: ["codex"],
    candidateIds: [],
    reportPath: "/tmp/report.json",
    workspaceRoot: "",
    listOnly: false,
    skipRuntime: true,
    installTarget: "host",
    environment: "host",
    networkMode: "all",
    firstTurnTimeoutMs: 1000,
  });

  assert.equal(report.result, "pass");
  assert.equal(report.providers.length, 1);
  assert.equal(report.providers[0].active_state.mode, "account");
  assert.deepEqual(report.providers[0].checks, ["profile_metadata", "active_profile"]);
});

test("runRegeneration validates endpoint-backed imports via harness config", async () => {
  const client = createFakeClient({
    "GET /api/providers/auth/import/candidates": {
      status: 200,
      payload: {
        candidates: [
          {
            id: "qwen-candidate",
            provider_id: "qwen",
            kind: "env_file",
            path: "/tmp/.qwen/.env",
            parse_status: "parsed",
            signal_strength: "strong",
            confidence: "high",
          },
        ],
      },
    },
    "POST /api/providers/auth/import": {
      status: 200,
      payload: {
        results: [
          {
            candidate_id: "qwen-candidate",
            provider_id: "qwen",
            status: "already_imported",
            profile_id: "endpoint-1",
          },
        ],
      },
    },
    "GET /api/providers/auth/import/profiles": {
      status: 200,
      payload: {
        profiles: [
          {
            id: "endpoint-1",
            provider_id: "qwen",
            source_path: "/tmp/.qwen/.env",
            source_kind: "env_file",
            secret_fingerprint: "fp-2",
          },
        ],
      },
    },
    "GET /api/providers/qwen/harness_config": {
      status: 200,
      payload: {
        selected_source_kind: "endpoint",
        selected_endpoint_id: "endpoint-1",
        endpoints: [{ id: "endpoint-1", has_api_key: true }],
      },
    },
  });

  const report = await runRegeneration(client, {
    baseUrl: "http://127.0.0.1:4399",
    token: "token",
    providerIds: ["qwen"],
    candidateIds: [],
    reportPath: "/tmp/report.json",
    workspaceRoot: "",
    listOnly: false,
    skipRuntime: true,
    installTarget: "host",
    environment: "host",
    networkMode: "all",
    firstTurnTimeoutMs: 1000,
  });

  assert.equal(report.result, "pass");
  assert.equal(report.providers.length, 1);
  assert.equal(report.providers[0].active_state.mode, "endpoint");
  assert.deepEqual(report.providers[0].checks, ["profile_metadata", "active_profile"]);
});
