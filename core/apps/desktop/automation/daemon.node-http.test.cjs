const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadDaemonHelper = () => {
  delete require.cache[DAEMON_HELPER_PATH];
  return require(DAEMON_HELPER_PATH);
};

test.afterEach(() => {
  delete global.browser;
  delete global.fetch;
  delete process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON;
  delete process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON_URL;
  delete process.env.CTX_AUTOMATION_REMOTE_HOST;
  delete process.env.CTX_AUTOMATION_REMOTE_PORT;
  delete process.env.CTX_AUTOMATION_REMOTE_CONTAINER_HOST;
  delete process.env.CTX_AUTOMATION_REMOTE_CONTAINER_PORT;
  delete process.env.CTX_AUTOMATION_EXPECT_DAEMON_VERSION;
  delete process.env.CTX_AUTOMATION_EXPECT_DAEMON_BUILD_ID;
  delete process.env.CTX_AUTOMATION_EXPECT_DAEMON_COMPATIBILITY_TOKEN;
  delete process.env.CTX_AUTOMATION_DAEMON_AUTH_PATH;
  delete process.env.CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR;
  delete require.cache[DAEMON_HELPER_PATH];
});

test("daemonJson uses host-side fetch and reuses the cached desktop connection", async () => {
  const executeCalls = [];
  const fetchCalls = [];

  global.browser = {
    execute: async (_fn, command) => {
      executeCalls.push(command || "desktop_get_connection");
      return {
        info: {
          kind: "local",
          base_url: "http://127.0.0.1:4399",
          browser_query_secret: "browser-secret-one",
        },
      };
    },
  };
  global.fetch = async (url, options) => {
    fetchCalls.push({
      url: String(url),
      method: options?.method || "GET",
      authorization: options?.headers?.authorization || "",
    });
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { daemonJson } = loadDaemonHelper();
  const first = await daemonJson("GET", "/api/health");
  const second = await daemonJson("GET", "/api/providers");

  assert.equal(first.status, 200);
  assert.equal(second.status, 200);
  assert.deepEqual(fetchCalls, [
    {
      url: "http://127.0.0.1:4399/api/health",
      method: "GET",
      authorization: "Bearer browser-secret-one",
    },
    {
      url: "http://127.0.0.1:4399/api/providers",
      method: "GET",
      authorization: "Bearer browser-secret-one",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection", "desktop_connect_local"]);
});

test("daemonJson refreshes the desktop connection after an auth failure", async () => {
  const executeCalls = [];
  const fetchCalls = [];
  let connectionIndex = 0;
  const connections = [
    {
      kind: "ssh",
      base_url: "http://127.0.0.1:4400",
      browser_query_secret: "browser-secret-old",
    },
    {
      kind: "ssh",
      base_url: "http://127.0.0.1:4400",
      browser_query_secret: "browser-secret-new",
    },
  ];

  global.browser = {
    execute: async (_fn, command) => {
      executeCalls.push(command || "desktop_get_connection");
      const current = connections[Math.min(connectionIndex, connections.length - 1)];
      connectionIndex += 1;
      return { info: current };
    },
  };
  global.fetch = async (url, options) => {
    fetchCalls.push({
      url: String(url),
      authorization: options?.headers?.authorization || "",
    });
    if (fetchCalls.length === 1) {
      return new Response("{}", { status: 401, headers: { "content-type": "application/json" } });
    }
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { daemonJson } = loadDaemonHelper();
  const resp = await daemonJson("POST", "/api/workspaces", { name: "workspace" });

  assert.equal(resp.status, 200);
  assert.deepEqual(fetchCalls, [
    {
      url: "http://127.0.0.1:4400/api/workspaces",
      authorization: "Bearer browser-secret-old",
    },
    {
      url: "http://127.0.0.1:4400/api/workspaces",
      authorization: "Bearer browser-secret-new",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection", "desktop_get_connection"]);
});

test("daemonJsonOnce refreshes the desktop connection once after auth failure", async () => {
  const executeCalls = [];
  const fetchCalls = [];
  let connectionIndex = 0;
  const connections = [
    {
      kind: "ssh",
      base_url: "http://127.0.0.1:4401",
      browser_query_secret: "browser-secret-old",
    },
    {
      kind: "ssh",
      base_url: "http://127.0.0.1:4401",
      browser_query_secret: "browser-secret-new",
    },
  ];

  global.browser = {
    execute: async (_fn, command) => {
      executeCalls.push(command || "desktop_get_connection");
      const current = connections[Math.min(connectionIndex, connections.length - 1)];
      connectionIndex += 1;
      return { info: current };
    },
  };
  global.fetch = async (url, options) => {
    fetchCalls.push({
      url: String(url),
      authorization: options?.headers?.authorization || "",
    });
    if (fetchCalls.length === 1) {
      return new Response("{}", { status: 401, headers: { "content-type": "application/json" } });
    }
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { daemonJsonOnce } = loadDaemonHelper();
  const resp = await daemonJsonOnce("GET", "/api/health");

  assert.equal(resp.status, 200);
  assert.deepEqual(fetchCalls, [
    {
      url: "http://127.0.0.1:4401/api/health",
      authorization: "Bearer browser-secret-old",
    },
    {
      url: "http://127.0.0.1:4401/api/health",
      authorization: "Bearer browser-secret-new",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection", "desktop_get_connection"]);
});

test("daemonJsonOnce refreshes the desktop connection once after transport failure", async () => {
  const executeCalls = [];
  const fetchCalls = [];
  let connectionIndex = 0;
  const connections = [
    {
      kind: "local",
      base_url: "http://127.0.0.1:4402",
      browser_query_secret: "browser-secret-old",
    },
    {
      kind: "local",
      base_url: "http://127.0.0.1:4402",
      browser_query_secret: "browser-secret-old",
    },
    {
      kind: "local",
      base_url: "http://127.0.0.1:4403",
      browser_query_secret: "browser-secret-new",
    },
    {
      kind: "local",
      base_url: "http://127.0.0.1:4403",
      browser_query_secret: "browser-secret-new",
    },
  ];

  global.browser = {
    execute: async (_fn, command) => {
      executeCalls.push(command || "desktop_get_connection");
      const current = connections[Math.min(connectionIndex, connections.length - 1)];
      connectionIndex += 1;
      return { info: current };
    },
  };
  global.fetch = async (url, options) => {
    fetchCalls.push({
      url: String(url),
      authorization: options?.headers?.authorization || "",
    });
    if (fetchCalls.length === 1) {
      throw new TypeError("fetch failed");
    }
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { daemonJsonOnce } = loadDaemonHelper();
  const resp = await daemonJsonOnce("GET", "/api/health");

  assert.equal(resp.status, 200);
  assert.deepEqual(fetchCalls, [
    {
      url: "http://127.0.0.1:4402/api/health",
      authorization: "Bearer browser-secret-old",
    },
    {
      url: "http://127.0.0.1:4403/api/health",
      authorization: "Bearer browser-secret-new",
    },
  ]);
  assert.deepEqual(executeCalls, [
    "desktop_get_connection",
    "desktop_connect_local",
    "desktop_get_connection",
    "desktop_connect_local",
  ]);
});

test("daemonJson uses the direct remote daemon URL for ssh connections when explicitly enabled", async () => {
  const executeCalls = [];
  const fetchCalls = [];
  process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON = "1";
  process.env.CTX_AUTOMATION_REMOTE_HOST = "127.0.0.1";
  process.env.CTX_AUTOMATION_REMOTE_PORT = "44099";

  global.browser = {
    execute: async (_fn, command) => {
      executeCalls.push(command || "desktop_get_connection");
      return {
        info: {
          kind: "ssh",
          base_url: "http://127.0.0.1:51575",
          browser_query_secret: "browser-secret-one",
          host: "127.0.0.1",
          remote_port: 44099,
          user: "ctxfixture",
        },
      };
    },
  };
  global.fetch = async (url, options) => {
    fetchCalls.push({
      url: String(url),
      method: options?.method || "GET",
      authorization: options?.headers?.authorization || "",
    });
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { daemonJson } = loadDaemonHelper();
  const resp = await daemonJson("GET", "/api/health");

  assert.equal(resp.status, 200);
  assert.deepEqual(fetchCalls, [
    {
      url: "http://127.0.0.1:44099/api/health",
      method: "GET",
      authorization: "Bearer browser-secret-one",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection"]);
});

test("daemonJson prefers an explicit direct remote daemon URL over the desktop tunnel URL", async () => {
  const fetchCalls = [];
  process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON = "1";
  process.env.CTX_AUTOMATION_REMOTE_DIRECT_DAEMON_URL = "http://127.0.0.1:47123";

  global.browser = {
    execute: async () => ({
      info: {
        kind: "ssh",
        base_url: "http://127.0.0.1:51575",
        browser_query_secret: "browser-secret-two",
      },
    }),
  };
  global.fetch = async (url, options) => {
    fetchCalls.push({
      url: String(url),
      authorization: options?.headers?.authorization || "",
    });
    return new Response(JSON.stringify({ ok: true }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { daemonJson } = loadDaemonHelper();
  const resp = await daemonJson("GET", "/api/health");

  assert.equal(resp.status, 200);
  assert.deepEqual(fetchCalls, [
    {
      url: "http://127.0.0.1:47123/api/health",
      authorization: "Bearer browser-secret-two",
    },
  ]);
});

test("daemonJson requires browser_query_secret rather than the raw desktop token", async () => {
  global.browser = {
    execute: async () => ({
      info: {
        kind: "ssh",
        base_url: "http://127.0.0.1:51575",
        token: "raw-token-must-not-be-used",
      },
    }),
  };
  global.fetch = async () => {
    throw new Error("fetch should not run without browser_query_secret");
  };

  const { daemonJson } = loadDaemonHelper();
  await assert.rejects(
    () => daemonJson("GET", "/api/health"),
    /desktop_get_connection missing base_url\/browser_query_secret/,
  );
});

test("assertExpectedDaemonIdentity passes when health compatibility matches env", async () => {
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_VERSION = "0.62.26";
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_BUILD_ID = "4c678fbec605";
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_COMPATIBILITY_TOKEN = "artifact-4c678fbec605";

  global.browser = {
    execute: async () => ({
      info: {
        kind: "ssh",
        base_url: "http://127.0.0.1:51575",
        browser_query_secret: "browser-secret",
      },
    }),
  };
  global.fetch = async () => new Response(JSON.stringify({
    daemon_version: "0.62.26",
    compatibility: {
      desktop_build_id: "4c678fbec605",
      protocol_compatibility_token: "artifact-4c678fbec605",
    },
  }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });

  const { assertExpectedDaemonIdentity } = loadDaemonHelper();
  const result = await assertExpectedDaemonIdentity({ label: "identity-test" });

  assert.equal(result.skipped, false);
  assert.deepEqual(result.mismatches, []);
  assert.equal(result.actual.version, "0.62.26");
});

test("assertExpectedDaemonIdentity uses raw daemon auth when browser health omits compatibility token", async () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-daemon-auth-"));
  const authPath = path.join(tempDir, "daemon_auth.json");
  fs.writeFileSync(authPath, JSON.stringify({
    daemon_url: "http://127.0.0.1:51575",
    token: "raw-daemon-token",
  }));
  process.env.CTX_AUTOMATION_DAEMON_AUTH_PATH = authPath;
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_VERSION = "0.62.26";
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_BUILD_ID = "4c678fbec605";
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_COMPATIBILITY_TOKEN = "artifact-4c678fbec605";

  global.browser = {
    execute: async () => ({
      info: {
        kind: "local",
        base_url: "http://127.0.0.1:51575",
        browser_query_secret: "browser-secret",
      },
    }),
  };
  const authorizations = [];
  global.fetch = async (_url, init) => {
    authorizations.push(String(init?.headers?.authorization || ""));
    const rawAuth = authorizations.at(-1) === "Bearer raw-daemon-token";
    return new Response(JSON.stringify({
      daemon_version: "0.62.26",
      compatibility: {
        desktop_build_id: "4c678fbec605",
        protocol_compatibility_token: rawAuth ? "artifact-4c678fbec605" : "",
      },
    }), {
      status: 200,
      headers: { "content-type": "application/json" },
    });
  };

  const { assertExpectedDaemonIdentity } = loadDaemonHelper();
  const result = await assertExpectedDaemonIdentity({ label: "identity-test" });

  assert.equal(result.raw_auth_health_used, true);
  assert.deepEqual(authorizations, ["Bearer browser-secret", "Bearer raw-daemon-token"]);
  assert.equal(result.actual.compatibilityToken, "artifact-4c678fbec605");
});

test("assertExpectedDaemonIdentity fails before product proof on mismatch", async () => {
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_VERSION = "0.62.26";
  process.env.CTX_AUTOMATION_EXPECT_DAEMON_BUILD_ID = "4c678fbec605";

  global.browser = {
    execute: async () => ({
      info: {
        kind: "ssh",
        base_url: "http://127.0.0.1:51575",
        browser_query_secret: "browser-secret",
      },
    }),
  };
  global.fetch = async () => new Response(JSON.stringify({
    daemon_version: "0.62.22",
    compatibility: {
      desktop_build_id: "older-build",
      protocol_compatibility_token: "artifact-older-build",
    },
  }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });

  const { assertExpectedDaemonIdentity } = loadDaemonHelper();
  await assert.rejects(
    () => assertExpectedDaemonIdentity({ label: "identity-test" }),
    /daemon identity mismatch before product proof/,
  );
});
