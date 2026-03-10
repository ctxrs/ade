const test = require("node:test");
const assert = require("node:assert/strict");

const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadDaemonHelper = () => {
  delete require.cache[DAEMON_HELPER_PATH];
  return require(DAEMON_HELPER_PATH);
};

test.afterEach(() => {
  delete global.browser;
  delete global.fetch;
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
          token: "token-one",
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
      authorization: "Bearer token-one",
    },
    {
      url: "http://127.0.0.1:4399/api/providers",
      method: "GET",
      authorization: "Bearer token-one",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection"]);
});

test("daemonJson refreshes the desktop connection after an auth failure", async () => {
  const executeCalls = [];
  const fetchCalls = [];
  let connectionIndex = 0;
  const connections = [
    {
      kind: "local",
      base_url: "http://127.0.0.1:4400",
      token: "token-old",
    },
    {
      kind: "local",
      base_url: "http://127.0.0.1:4400",
      token: "token-new",
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
      authorization: "Bearer token-old",
    },
    {
      url: "http://127.0.0.1:4400/api/workspaces",
      authorization: "Bearer token-new",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection", "desktop_get_connection"]);
});

test("daemonJsonOnce performs a single transport attempt while still refreshing auth once", async () => {
  const executeCalls = [];
  const fetchCalls = [];
  let connectionIndex = 0;
  const connections = [
    {
      kind: "local",
      base_url: "http://127.0.0.1:4401",
      token: "token-old",
    },
    {
      kind: "local",
      base_url: "http://127.0.0.1:4401",
      token: "token-new",
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
      authorization: "Bearer token-old",
    },
    {
      url: "http://127.0.0.1:4401/api/health",
      authorization: "Bearer token-new",
    },
  ]);
  assert.deepEqual(executeCalls, ["desktop_get_connection", "desktop_get_connection"]);
});
