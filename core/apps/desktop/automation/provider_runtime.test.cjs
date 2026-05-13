const test = require("node:test");
const assert = require("node:assert/strict");

const HELPER_PATH = require.resolve("./specs/helpers/provider_runtime.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadHelper = (daemonHelper) => {
  delete require.cache[HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
  const helperExports = typeof daemonHelper === "function"
    ? { daemonJson: daemonHelper }
    : daemonHelper;
  require.cache[DAEMON_HELPER_PATH] = {
    id: DAEMON_HELPER_PATH,
    filename: DAEMON_HELPER_PATH,
    loaded: true,
    exports: helperExports,
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

  assert.equal(codexEnv.modelOverride, "google/gemini-2.5-flash");
  assert.equal(qwenEnv.modelOverride, "google/gemini-2.5-flash");
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
            current_model_id: "google/gemini-2.5-flash",
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
  assert.equal(result.modelId, "google/gemini-2.5-flash");
  assert.equal(calls[0].requestPath, "/api/providers/codex?target=container");
  assert.equal(
    calls.some((entry) => entry.requestPath.includes("/api/providers/codex/install?target=")),
    false,
  );
});

test("ensureCodexOpenRouterWorkspaceReady waits for target readiness before workspace verify", async () => {
  process.env.OPENROUTER_API_KEY = "openrouter-key";
  global.browser = { pause: async () => {} };
  const calls = [];
  const statuses = [
    {
      provider_id: "codex",
      installed: true,
      health: "ok",
      diagnostics: ["provider is not ready until required dependencies are installed: codex-cli"],
      details: {
        install_supported: "true",
        ready_for_use: "false",
        required_dependency_ids: "codex-cli",
        pending_dependency_ids: "codex-cli",
      },
    },
    {
      provider_id: "codex",
      installed: true,
      detected_path: "/tmp/ctx/codex-crp",
      health: "ok",
      diagnostics: [],
      details: {
        install_supported: "true",
        ready_for_use: "true",
      },
    },
  ];

  const { ensureCodexOpenRouterWorkspaceReady } = loadHelper({
    daemonJson: async (method, requestPath, body) => {
      calls.push({ method, requestPath, body });
      if (method === "GET" && requestPath === "/api/health") {
        return {
          status: 200,
          payload: {
            daemon_version: "0.62.59",
            pid: 101,
            daemon_url: "http://127.0.0.1:64000",
            data_root: "/home/example-user/.ctx",
            compatibility: { desktop_build_id: "build-a" },
          },
        };
      }
      if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
        return { status: 200, payload: statuses.shift() || statuses.at(-1) };
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
        return { status: 200, payload: { ok: true } };
      }
      if (method === "POST" && requestPath === "/api/workspaces/ws-1/providers/codex/verify") {
        return { status: 200, payload: { status: "ok" } };
      }
      if (method === "GET" && requestPath === "/api/workspaces/ws-1/providers/codex/options") {
        return {
          status: 200,
          payload: {
            models: {
              current_model_id: "google/gemini-2.5-flash",
            },
          },
        };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
    },
    getDesktopConnection: async () => ({
      kind: "ssh",
      base_url: "http://127.0.0.1:47001",
      browser_query_secret: "browser-secret",
    }),
  });

  await ensureCodexOpenRouterWorkspaceReady("ws-1", {
    installTarget: "container",
    timeoutMs: 1000,
    pollMs: 1,
  });

  const providerStatusCalls = calls
    .map((entry, index) => ({ ...entry, index }))
    .filter((entry) => entry.requestPath === "/api/providers/codex?target=container");
  const verifyCallIndex = calls.findIndex(
    (entry) => entry.requestPath === "/api/workspaces/ws-1/providers/codex/verify",
  );

  assert.equal(providerStatusCalls.length, 2);
  assert.ok(verifyCallIndex > providerStatusCalls.at(-1).index);
  assert.equal(
    calls.some((entry) => entry.requestPath.includes("/api/providers/codex/install?target=")),
    false,
  );
});

test("ensureCodexOpenRouterWorkspaceReady can repair a lost nonblocking install kickoff", async () => {
  process.env.OPENROUTER_API_KEY = "openrouter-key";
  global.browser = { pause: async () => {} };
  const calls = [];
  const installedStatus = {
    provider_id: "codex",
    installed: true,
    detected_path: "/tmp/ctx/codex-crp",
    health: "ok",
    diagnostics: [],
    details: {
      install_supported: "true",
      ready_for_use: "true",
    },
  };
  const statuses = [
    {
      provider_id: "codex",
      installed: false,
      health: "missing",
      diagnostics: [
        "runtime command is not configured for provider 'codex'",
        "provider is not ready until required dependencies are installed: codex-cli",
      ],
      details: {
        install_supported: "true",
        ready_for_use: "false",
        required_dependency_ids: "codex-cli",
        pending_dependency_ids: "codex-cli",
      },
    },
    installedStatus,
  ];

  const { ensureCodexOpenRouterWorkspaceReady } = loadHelper({
    daemonJson: async (method, requestPath, body) => {
      calls.push({ method, requestPath, body });
      if (method === "GET" && requestPath === "/api/health") {
        return {
          status: 200,
          payload: {
            daemon_version: "0.65.8",
            pid: 101,
            daemon_url: "http://127.0.0.1:64000",
            data_root: "/home/example-user/.ctx",
            compatibility: { desktop_build_id: "build-a" },
          },
        };
      }
      if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
        return { status: 200, payload: statuses.shift() || installedStatus };
      }
      if (method === "POST" && requestPath === "/api/providers/codex/install?target=container") {
        return { status: 200, payload: { install_id: "install-1", target: "container" } };
      }
      if (method === "GET" && requestPath === "/api/providers/install/install-1") {
        return { status: 200, payload: { state: "succeeded" } };
      }
      if (method === "POST" && requestPath === "/api/providers/codex/harness_config/endpoints") {
        return {
          status: 200,
          payload: {
            selected_endpoint_id: "endpoint-1",
            endpoints: [{ id: "endpoint-1", name: "codex-openrouter-desktop-smoke" }],
          },
        };
      }
      if (method === "POST" && requestPath === "/api/providers/codex/harness_config/select") {
        return { status: 200, payload: { ok: true } };
      }
      if (method === "POST" && requestPath === "/api/workspaces/ws-1/providers/codex/verify") {
        return { status: 200, payload: { status: "ok" } };
      }
      if (method === "GET" && requestPath === "/api/workspaces/ws-1/providers/codex/options") {
        return {
          status: 200,
          payload: {
            models: {
              current_model_id: "google/gemini-2.5-flash",
            },
          },
        };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
    },
    getDesktopConnection: async () => ({
      kind: "local",
      base_url: "http://127.0.0.1:47001",
      browser_query_secret: "browser-secret",
    }),
  });

  await ensureCodexOpenRouterWorkspaceReady("ws-1", {
    installTarget: "container",
    installTimeoutMs: 1000,
    pollMs: 1,
    allowInstall: true,
  });

  const installCallIndex = calls.findIndex(
    (entry) => entry.method === "POST" && entry.requestPath === "/api/providers/codex/install?target=container",
  );
  const verifyCallIndex = calls.findIndex(
    (entry) => entry.method === "POST" && entry.requestPath === "/api/workspaces/ws-1/providers/codex/verify",
  );
  assert.ok(installCallIndex >= 0, "expected a repair install after the missing dependency status");
  assert.ok(verifyCallIndex > installCallIndex, "workspace verify must wait for the repair install");
});

test("waitForProviderInstallCompletion keeps waiting while dependencies are pending", async () => {
  const calls = [];
  global.browser = { pause: async () => {} };
  const statuses = [
    {
      provider_id: "codex",
      installed: true,
      health: "ok",
      diagnostics: ["provider is not ready until required dependencies are installed: codex-cli"],
      details: {
        install_supported: "true",
        ready_for_use: "false",
        required_dependency_ids: "codex-cli",
        pending_dependency_ids: "codex-cli",
      },
    },
    {
      provider_id: "codex",
      installed: true,
      detected_path: "/tmp/ctx/codex-crp",
      health: "ok",
      diagnostics: [],
      details: {
        install_supported: "true",
        ready_for_use: "true",
      },
    },
  ];
  const { waitForProviderInstallCompletion } = loadHelper({
    daemonJson: async (method, requestPath) => {
      calls.push({ method, requestPath });
      if (method === "GET" && requestPath === "/api/health") {
        return {
          status: 200,
          payload: {
            daemon_version: "0.62.26",
            compatibility: { desktop_build_id: "build-a" },
          },
        };
      }
      if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
        return { status: 200, payload: statuses.shift() || statuses.at(-1) };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
    },
    getDesktopConnection: async () => ({
      kind: "local",
      base_url: "http://127.0.0.1:47001",
      browser_query_secret: "browser-secret",
    }),
  });

  const result = await waitForProviderInstallCompletion("codex", "container", {
    timeoutMs: 1000,
    pollMs: 1,
    settleMs: 0,
  });

  assert.equal(result.installed, true);
  assert.ok(
    calls.filter((entry) => entry.requestPath === "/api/providers/codex?target=container").length >= 2,
    "pending dependencies should keep the poll loop alive after settleMs",
  );
});

test("waitForProviderInstallCompletion fails fast when install is unsupported", async () => {
  global.browser = { pause: async () => {} };
  const { waitForProviderInstallCompletion } = loadHelper({
    daemonJson: async (method, requestPath) => {
      if (method === "GET" && requestPath === "/api/health") {
        return {
          status: 200,
          payload: {
            daemon_version: "0.62.26",
            compatibility: { desktop_build_id: "build-a" },
          },
        };
      }
      if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
        return {
          status: 200,
          payload: {
            provider_id: "codex",
            installed: false,
            health: "missing",
            diagnostics: ["no managed install metadata"],
            details: {
              install_supported: "false",
            },
          },
        };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
    },
    getDesktopConnection: async () => ({
      kind: "local",
      base_url: "http://127.0.0.1:47001",
      browser_query_secret: "browser-secret",
    }),
  });

  await assert.rejects(
    () => waitForProviderInstallCompletion("codex", "container", {
      timeoutMs: 1000,
      pollMs: 1,
      settleMs: 0,
    }),
    /install is unsupported.*install_supported/,
  );
});

test("waitForProviderInstallCompletion reports daemon changes during install polling", async () => {
  global.browser = { pause: async () => {} };
  let healthIndex = 0;
  const healthPayloads = [
    { daemon_version: "0.62.26", pid: 101, compatibility: { desktop_build_id: "build-a" } },
    { daemon_version: "0.62.26", pid: 202, compatibility: { desktop_build_id: "build-a" } },
  ];
  const { waitForProviderInstallCompletion } = loadHelper({
    daemonJson: async (method, requestPath) => {
      if (method === "GET" && requestPath === "/api/health") {
        const payload = healthPayloads[Math.min(healthIndex, healthPayloads.length - 1)];
        healthIndex += 1;
        return { status: 200, payload };
      }
      if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
        return {
          status: 200,
          payload: {
            provider_id: "codex",
            installed: false,
            health: "missing",
            diagnostics: ["provider install still running"],
            details: {
              install_supported: "true",
              install_running: "true",
              install_id: "install-1",
            },
          },
        };
      }
      if (method === "GET" && requestPath === "/api/providers/install/install-1") {
        return { status: 200, payload: { state: "running" } };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
    },
    getDesktopConnection: async () => ({
      kind: "local",
      base_url: "http://127.0.0.1:47001",
      browser_query_secret: "browser-secret",
    }),
  });

  await assert.rejects(
    () => waitForProviderInstallCompletion("codex", "container", {
      timeoutMs: 1000,
      pollMs: 1,
      settleMs: 0,
    }),
    /daemon identity changed while waiting.*daemon_change/,
  );
});

test("waitForProviderInstallCompletion ignores SSH tunnel URL churn when daemon identity is stable", async () => {
  global.browser = { pause: async () => {} };
  const statuses = [
    {
      provider_id: "codex",
      installed: false,
      health: "missing",
      diagnostics: ["provider install still running"],
      details: {
        install_supported: "true",
        install_running: "true",
        install_id: "install-1",
      },
    },
    {
      provider_id: "codex",
      installed: true,
      detected_path: "/tmp/ctx/codex-crp",
      health: "ok",
      diagnostics: [],
      details: {
        install_supported: "true",
        ready_for_use: "true",
      },
    },
  ];
  const tunnelUrls = [
    "http://127.0.0.1:47001",
    "http://127.0.0.1:47002",
    "http://127.0.0.1:47003",
  ];
  const { waitForProviderInstallCompletion } = loadHelper({
    daemonJson: async (method, requestPath) => {
      if (method === "GET" && requestPath === "/api/health") {
        return {
          status: 200,
          payload: {
            daemon_version: "0.62.26",
            pid: 101,
            daemon_url: "http://127.0.0.1:64000",
            data_root: "/home/example-user/.ctx",
            compatibility: { desktop_build_id: "build-a" },
          },
        };
      }
      if (method === "GET" && requestPath === "/api/providers/codex?target=container") {
        return { status: 200, payload: statuses.shift() || statuses.at(-1) };
      }
      if (method === "GET" && requestPath === "/api/providers/install/install-1") {
        return { status: 200, payload: { state: "running" } };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
    },
    getDesktopConnection: async () => ({
      kind: "ssh",
      base_url: tunnelUrls.shift() || "http://127.0.0.1:47004",
      browser_query_secret: "browser-secret",
    }),
  });

  const result = await waitForProviderInstallCompletion("codex", "container", {
    timeoutMs: 1000,
    pollMs: 1,
    settleMs: 0,
  });

  assert.equal(result.installed, true);
});
