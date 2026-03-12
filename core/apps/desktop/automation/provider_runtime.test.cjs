const test = require("node:test");
const assert = require("node:assert/strict");

const HELPER_PATH = require.resolve("./specs/helpers/provider_runtime.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadHelper = (daemonJson) => {
  delete require.cache[HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  require.cache[DAEMON_HELPER_PATH] = {
    id: DAEMON_HELPER_PATH,
    filename: DAEMON_HELPER_PATH,
    loaded: true,
    exports: { daemonJson },
  };
  return require(HELPER_PATH);
};

test.afterEach(() => {
  delete require.cache[HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  delete process.env.OPENROUTER_API_KEY;
  delete process.env.OPENROUTER_BASE_URL;
  delete process.env.CTX_E2E_OPENROUTER_MODEL_OVERRIDE;
  delete global.browser;
});

test("readOpenRouterEnv uses provider-specific cheap defaults for write smoke", () => {
  const { readOpenRouterEnv } = loadHelper(async () => {
    throw new Error("daemonJson should not be called");
  });

  const codexEnv = readOpenRouterEnv("codex");
  const qwenEnv = readOpenRouterEnv("qwen");
  const piEnv = readOpenRouterEnv("pi");

  assert.equal(codexEnv.modelOverride, "openai/gpt-4.1-mini");
  assert.equal(qwenEnv.modelOverride, "openai/gpt-4.1-nano");
  assert.equal(piEnv.modelOverride, "google/gemini-3-flash-preview");
});

test("getProviderStatus requests the requested install target", async () => {
  const calls = [];
  const { getProviderStatus } = loadHelper(async (method, requestPath) => {
    calls.push({ method, requestPath });
    return {
      status: 200,
      payload: {
        provider_id: "codex",
        installed: true,
        health: "ok",
        diagnostics: ["ready"],
        details: {
          install_target: "container",
          managed_target: "container",
        },
      },
    };
  });

  const status = await getProviderStatus("codex", "container");

  assert.deepEqual(calls, [
    {
      method: "GET",
      requestPath: "/api/providers/codex?target=container",
    },
  ]);
  assert.equal(status.installed, true);
  assert.equal(status.health, "ok");
  assert.deepEqual(status.diagnostics, ["ready"]);
  assert.equal(status.details.install_target, "container");
  assert.equal(status.details.managed_target, "container");
});

test("ensureCodexOpenRouterWorkspaceReady checks install status against the requested target", async () => {
  process.env.OPENROUTER_API_KEY = "openrouter-key";
  const calls = [];
  const { ensureCodexOpenRouterWorkspaceReady } = loadHelper(async (method, requestPath, body) => {
    calls.push({ method, requestPath, body });
    if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
      return {
        status: 200,
        payload: {
          provider_id: "codex",
          installed: true,
          health: "ok",
          diagnostics: [],
          details: {
            install_target: "container",
            managed_target: "container",
          },
        },
      };
    }
    if (method === "POST" && requestPath === "/api/providers/codex/harness_config/endpoints") {
      return {
        status: 200,
        payload: {
          selected_endpoint_id: "endpoint-1",
          endpoints: [
            {
              id: "endpoint-1",
              name: "codex-openrouter-desktop-smoke",
            },
          ],
        },
      };
    }
    if (method === "POST" && requestPath === "/api/providers/codex/harness_config/select") {
      return {
        status: 200,
        payload: { ok: true },
      };
    }
    if (method === "POST" && requestPath === "/api/workspaces/ws-1/providers/codex/verify") {
      return {
        status: 200,
        payload: { status: "ok" },
      };
    }
    if (method === "GET" && requestPath === "/api/workspaces/ws-1/providers/codex/options") {
      return {
        status: 200,
        payload: {
          models: {
            current_model_id: "openai/gpt-4.1-mini",
          },
        },
      };
    }
    throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
  });

  const result = await ensureCodexOpenRouterWorkspaceReady("ws-1", {
    installTarget: "container",
  });

  assert.equal(result.endpointId, "endpoint-1");
  assert.equal(result.modelId, "openai/gpt-4.1-mini");
  assert.equal(calls[0].requestPath, "/api/providers/codex?target=container");
  assert.equal(
    calls.some((entry) => entry.requestPath.includes("/api/providers/codex/install?target=")),
    false,
  );
});
