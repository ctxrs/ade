const shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate = ({
  daemonLocation,
  executionEnvironment,
  platform,
}) => (
  daemonLocation === "local"
  && executionEnvironment === "sandbox"
  && platform === "linux"
);

const normalizeText = (value) => String(value == null ? "" : value).trim();

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const createProviderAuthImportWorkspaceAndLaunchExecution = async ({
  dest,
  name,
  daemonLocation,
  executionEnvironment,
  timeoutMs = 15 * 60_000,
  platform = process.platform,
  onLaunchStart = null,
  initGitRepo,
  daemonJson,
  ensureLocalLinuxSandboxReady,
  getWorkspaceTerminalCwd,
}) => {
  if (shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate({
    daemonLocation,
    executionEnvironment,
    platform,
  })) {
    await ensureLocalLinuxSandboxReady();
  }

  initGitRepo(dest, name);
  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: dest,
    name,
  });
  if (create.status !== 200) {
    throw new Error(`workspace create failed (${create.status}): ${JSON.stringify(create.payload || null)}`);
  }
  const workspaceId = normalizeText(create.payload?.id);
  if (!workspaceId) {
    throw new Error(`workspace create response missing id: ${JSON.stringify(create.payload || null)}`);
  }

  if (daemonLocation === "remote") {
    throw new Error(`remote daemon location is not supported by this spec: ${daemonLocation}`);
  }
  if (executionEnvironment !== "host" && executionEnvironment !== "sandbox") {
    throw new Error(`unsupported execution environment for this spec: ${executionEnvironment}`);
  }

  const environment = executionEnvironment;
  const networkMode = executionEnvironment === "sandbox" ? "llm_only" : "all";

  const setExec = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment,
    network_mode: networkMode,
  });
  if (setExec.status !== 200) {
    throw new Error(`execution config update failed (${setExec.status}): ${JSON.stringify(setExec.payload || null)}`);
  }

  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    workspace_id: workspaceId,
  });
  if (launch.status !== 200) {
    throw new Error(`execution launch start failed (${launch.status}): ${JSON.stringify(launch.payload || null)}`);
  }
  const jobId = normalizeText(launch.payload?.job_id);
  if (!jobId) {
    throw new Error(`execution launch response missing job_id: ${JSON.stringify(launch.payload || null)}`);
  }
  if (typeof onLaunchStart === "function") {
    await onLaunchStart({
      workspaceId,
      environment,
      networkMode,
      launchJobId: jobId,
      launchStart: launch.payload || null,
    });
  }

  const startedAt = Date.now();
  let lastPayload = null;
  while (Date.now() - startedAt < timeoutMs) {
    const status = await daemonJson("GET", `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`);
    if (status.status === 200) {
      lastPayload = status.payload || null;
      const state = normalizeText(status.payload?.state).toLowerCase();
      if (state === "ready") {
        let executionRoot = "";
        try {
          executionRoot = normalizeText(await getWorkspaceTerminalCwd(workspaceId));
        } catch (error) {
          let workspacePayload = null;
          try {
            const workspaceResp = await daemonJson("GET", `/api/workspaces/${workspaceId}`);
            workspacePayload = workspaceResp?.payload || null;
          } catch {
            workspacePayload = null;
          }
          const hostWorkspaceRootExists = fs.existsSync(dest);
          const hostWorkspaceRoot = dest;
          throw new Error(
            `${String(error)}; host_workspace_root=${JSON.stringify(hostWorkspaceRoot)}; host_workspace_root_exists=${hostWorkspaceRootExists}; workspace_payload=${JSON.stringify(workspacePayload)}`,
          );
        }
        if (!executionRoot) {
          throw new Error(`workspace execution root missing for ${workspaceId}`);
        }
        return {
          workspaceId,
          environment,
          networkMode,
          executionRoot,
          launchJobId: jobId,
          launchStatus: status.payload || null,
        };
      }
      if (state === "error") {
        throw new Error(`execution launch failed: ${JSON.stringify(status.payload || null)}`);
      }
    }
    await waitMs(1000);
  }

  throw new Error(
    `execution launch timed out for workspace=${workspaceId} job=${jobId} last=${JSON.stringify(lastPayload)}`,
  );
};

module.exports = {
  createProviderAuthImportWorkspaceAndLaunchExecution,
  shouldEnsureLocalLinuxSandboxBeforeWorkspaceCreate,
};
const fs = require("node:fs");
