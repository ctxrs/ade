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
    if (method === "GET" && requestPath === "/api/sessions/session-1/diff") {
      return {
        status: 200,
        payload: {
          diff: [
            "diff --git a/hello.md b/hello.md",
            "new file mode 100644",
            "index 0000000..32f95c0",
            "--- /dev/null",
            "+++ b/hello.md",
            "@@ -0,0 +1 @@",
            "+hi",
            "\\ No newline at end of file",
          ].join("\n"),
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
  assert.equal(result.filePath, "hello.md");
  assert.equal(result.fileContents, "hi");
  assert.equal(fs.existsSync(path.join(sourceRoot, "hello.md")), false);
});

test("runProviderFileEditApiSmoke waits for turn completion before accepting assistant output", async () => {
  const sourceRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-file-edit-source-"));
  let historyCalls = 0;
  let diffCalls = 0;

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
      assert.equal(body.execution_environment, "sandbox");
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
      historyCalls += 1;
      if (historyCalls === 1) {
        return {
          status: 200,
          payload: {
            messages: [
              {
                role: "assistant",
                content:
                  "It seems there is an issue with creating the file in the current working directory as requested. Let me try another method to write the file correctly in the workspace root.",
              },
            ],
            turns: [
              {
                status: "running",
                turn_id: "turn-1",
              },
            ],
          },
        };
      }

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
              turn_id: "turn-1",
            },
          ],
        },
      };
    }
    if (method === "GET" && requestPath === "/api/sessions/session-1/diff") {
      diffCalls += 1;
      return {
        status: 200,
        payload: {
          diff: diffCalls === 1
            ? ""
            : [
              "diff --git a/hello.md b/hello.md",
              "new file mode 100644",
              "index 0000000..32f95c0",
              "--- /dev/null",
              "+++ b/hello.md",
              "@@ -0,0 +1 @@",
              "+hi",
              "\\ No newline at end of file",
            ].join("\n"),
        },
      };
    }
    throw new Error(`unexpected daemonJson call: ${method} ${requestPath}`);
  });

  const result = await runProviderFileEditApiSmoke(
    "ws-1",
    sourceRoot,
    {
      providerId: "codex",
      modelId: "default",
      executionEnvironment: "sandbox",
      relativeFilePath: "hello.md",
      fileContents: "hi",
      exactFileContents: true,
      expectedAssistantMessage: "hi",
      exactAssistantMessage: true,
    },
    2_000,
  );

  assert.equal(historyCalls, 2);
  assert.equal(diffCalls, 2);
  assert.equal(result.assistantMessage, "hi");
  assert.equal(result.filePath, "hello.md");
  assert.equal(result.fileContents, "hi");
});
