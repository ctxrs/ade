const test = require("node:test");
const assert = require("node:assert/strict");

const {
  createProviderAuthImportWorkspaceAndLaunchExecution,
  shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate,
} = require("./provider_auth_import_workspace_launch.cjs");

test("shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate only gates local linux sandbox cells", () => {
  assert.equal(
    shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate({
      daemonLocation: "local",
      executionEnvironment: "sandbox",
      platform: "linux",
    }),
    true,
  );
  assert.equal(
    shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate({
      daemonLocation: "local",
      executionEnvironment: "host",
      platform: "linux",
    }),
    false,
  );
  assert.equal(
    shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate({
      daemonLocation: "remote",
      executionEnvironment: "sandbox",
      platform: "linux",
    }),
    false,
  );
  assert.equal(
    shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate({
      daemonLocation: "local",
      executionEnvironment: "sandbox",
      platform: "darwin",
    }),
    false,
  );
});

test("createProviderAuthImportWorkspaceAndLaunchExecution prepares local linux sandbox before workspace create", async () => {
  const calls = [];
  const workspace = await createProviderAuthImportWorkspaceAndLaunchExecution({
    dest: "/tmp/ws",
    name: "ws",
    daemonLocation: "local",
    executionEnvironment: "sandbox",
    platform: "linux",
    timeoutMs: 50,
    initGitRepo: (dest, name) => {
      calls.push(["initGitRepo", dest, name]);
    },
    ensureLocalLinuxSandboxReady: async () => {
      calls.push(["ensureLocalLinuxSandboxReady"]);
    },
    daemonJson: async (method, apiPath, body) => {
      calls.push(["daemonJson", method, apiPath, body || null]);
      if (method === "POST" && apiPath === "/api/workspaces") {
        return { status: 200, payload: { id: "workspace-1" } };
      }
      if (method === "POST" && apiPath === "/api/workspaces/workspace-1/execution_config") {
        return { status: 200, payload: {} };
      }
      if (method === "POST" && apiPath === "/api/execution/launch/start") {
        return { status: 200, payload: { job_id: "job-1" } };
      }
      if (method === "GET" && apiPath === "/api/execution/launch/status?job_id=job-1") {
        return { status: 200, payload: { state: "ready" } };
      }
      throw new Error(`unexpected daemonJson call: ${method} ${apiPath}`);
    },
    getWorkspaceTerminalCwd: async (workspaceId) => {
      calls.push(["getWorkspaceTerminalCwd", workspaceId]);
      return "/tmp/ws";
    },
  });

  assert.equal(workspace.workspaceId, "workspace-1");
  assert.deepEqual(
    calls.map(([name]) => name),
    [
      "ensureLocalLinuxSandboxReady",
      "initGitRepo",
      "daemonJson",
      "daemonJson",
      "daemonJson",
      "daemonJson",
      "getWorkspaceTerminalCwd",
    ],
  );
});

test("createProviderAuthImportWorkspaceAndLaunchExecution reports host workspace root existence when terminal creation fails", async () => {
  await assert.rejects(
    createProviderAuthImportWorkspaceAndLaunchExecution({
      dest: "/tmp/ws-missing",
      name: "ws",
      daemonLocation: "local",
      executionEnvironment: "sandbox",
      platform: "linux",
      timeoutMs: 50,
      initGitRepo: () => {},
      ensureLocalLinuxSandboxReady: async () => {},
      daemonJson: async (method, apiPath) => {
        if (method === "POST" && apiPath === "/api/workspaces") {
          return { status: 200, payload: { id: "workspace-1" } };
        }
        if (method === "POST" && apiPath === "/api/workspaces/workspace-1/execution_config") {
          return { status: 200, payload: {} };
        }
        if (method === "POST" && apiPath === "/api/execution/launch/start") {
          return { status: 200, payload: { job_id: "job-1" } };
        }
        if (method === "GET" && apiPath === "/api/execution/launch/status?job_id=job-1") {
          return { status: 200, payload: { state: "ready" } };
        }
        if (method === "GET" && apiPath === "/api/workspaces/workspace-1") {
          return { status: 200, payload: { id: "workspace-1", root_path: "/tmp/ws-missing" } };
        }
        throw new Error(`unexpected daemonJson call: ${method} ${apiPath}`);
      },
      getWorkspaceTerminalCwd: async () => {
        throw new Error("terminal creation failed");
      },
    }),
    /host_workspace_root="\/tmp\/ws-missing"; host_workspace_root_exists=false; workspace_payload=\{"id":"workspace-1","root_path":"\/tmp\/ws-missing"\}/,
  );
});
