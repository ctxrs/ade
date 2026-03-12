const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const HELPER_PATH = require.resolve("./specs/helpers/workspace_wizard_flow.cjs");
const TAURI_HELPER_PATH = require.resolve("./specs/helpers/tauri.cjs");
const DAEMON_HELPER_PATH = require.resolve("./specs/helpers/daemon.cjs");

const loadHelper = (daemonJson, safeDaemonJson = daemonJson) => {
  delete require.cache[HELPER_PATH];
  delete require.cache[TAURI_HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];

  require.cache[TAURI_HELPER_PATH] = {
    id: TAURI_HELPER_PATH,
    filename: TAURI_HELPER_PATH,
    loaded: true,
    exports: {
      waitForTauri: async () => {},
      selectorForTestId: (value) => `[data-testid="${value}"]`,
      waitForTestId: async () => {},
      clickTestId: async () => {},
      setInputTestId: async () => {},
      getConnectionInfo: () => ({ port: 0 }),
    },
  };
  require.cache[DAEMON_HELPER_PATH] = {
    id: DAEMON_HELPER_PATH,
    filename: DAEMON_HELPER_PATH,
    loaded: true,
    exports: {
      daemonJson,
      daemonJsonOnce: daemonJson,
      safeDaemonJson,
    },
  };

  return require(HELPER_PATH);
};

test.afterEach(() => {
  delete require.cache[HELPER_PATH];
  delete require.cache[TAURI_HELPER_PATH];
  delete require.cache[DAEMON_HELPER_PATH];
});

test("resolveSessionWorktreeRoot reads the session snapshot worktree root", async () => {
  const calls = [];
  const { resolveSessionWorktreeRoot } = loadHelper(async (method, requestPath) => {
    calls.push({ method, requestPath });
    if (method === "GET" && requestPath === "/api/sessions/session-1/snapshot?limit=1") {
      return {
        status: 200,
        payload: {
          head: {
            session: {
              worktree_id: "wt-123",
            },
          },
          summary: {},
        },
      };
    }
    if (method === "GET" && requestPath === "/api/worktrees/wt-123") {
      return {
        status: 200,
        payload: {
          root_path: "/tmp/ctx-managed-worktree",
        },
      };
    }
    throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
  });

  const rootPath = await resolveSessionWorktreeRoot("session-1");

  assert.equal(rootPath, "/tmp/ctx-managed-worktree");
  assert.deepEqual(calls, [
    {
      method: "GET",
      requestPath: "/api/sessions/session-1/snapshot?limit=1",
    },
    {
      method: "GET",
      requestPath: "/api/worktrees/wt-123",
    },
  ]);
});

test("runProviderFileEditApiSmoke waits for files in the session managed worktree", async () => {
  const sourceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-file-edit-source-"));
  const managedRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-file-edit-managed-"));
  fs.writeFileSync(path.join(managedRoot, "hello.md"), "hi", "utf8");

  const { runProviderFileEditApiSmoke } = loadHelper(async (method, requestPath, body) => {
    if (method === "POST" && requestPath === "/api/workspaces/ws-1/tasks") {
      return {
        status: 200,
        payload: {
          id: "task-1",
        },
      };
    }
    if (method === "POST" && requestPath === "/api/tasks/task-1/sessions") {
      assert.equal(body.execution_environment, "host");
      return {
        status: 200,
        payload: {
          id: "session-1",
        },
      };
    }
    if (method === "POST" && requestPath === "/api/sessions/session-1/messages") {
      return {
        status: 200,
        payload: {
          ok: true,
        },
      };
    }
    if (method === "GET" && requestPath === "/api/sessions/session-1/history?limit=200") {
      return {
        status: 200,
        payload: {
          messages: [
            {
              role: "assistant",
              content: "hi",
            },
          ],
          turns: [
            {
              status: "completed",
            },
          ],
        },
      };
    }
    if (method === "GET" && requestPath === "/api/sessions/session-1/snapshot?limit=1") {
      return {
        status: 200,
        payload: {
          head: {
            session: {
              worktree_id: "wt-456",
            },
          },
          summary: {},
        },
      };
    }
    if (method === "GET" && requestPath === "/api/worktrees/wt-456") {
      return {
        status: 200,
        payload: {
          root_path: managedRoot,
        },
      };
    }
    if (requestPath.includes("/terminals")) {
      throw new Error(`unexpected terminal cwd lookup during file edit smoke: ${method} ${requestPath}`);
    }
    throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
  });

  const result = await runProviderFileEditApiSmoke(
    "ws-1",
    sourceRoot,
    {
      providerId: "kimi",
      modelId: "default",
      executionEnvironment: "host",
      relativeFilePath: "hello.md",
      fileContents: "hi",
      exactFileContents: true,
      expectedAssistantMessage: "hi",
      exactAssistantMessage: true,
    },
    10,
  );

  assert.equal(result.sessionId, "session-1");
  assert.equal(result.filePath, path.join(managedRoot, "hello.md"));
  assert.equal(result.fileContents, "hi");
  assert.equal(fs.existsSync(path.join(sourceRoot, "hello.md")), false);
});
