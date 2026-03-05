const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  runWizardScenario,
  getWorkspaceHarnessContainer,
  assertConnectedLocalAndListening,
  assertLocalWorkspaceConfig,
} = require("./helpers/workspace_wizard_flow.cjs");

const scenarioFilter = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean),
);

const scenarioEnabled = (name, tags = []) => {
  if (scenarioFilter.size === 0) return true;
  return [name, ...tags].some((t) => scenarioFilter.has(String(t).trim().toLowerCase()));
};

const normalizeErr = (value) => String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
const readAllowlist = (containerStatus) => (
  Array.isArray(containerStatus?.allowlist) ? containerStatus.allowlist : []
);

const ensureWorkspaceContainer = async (workspaceId) => {
  const ensure = await daemonJson("POST", `/api/workspaces/${workspaceId}/harness_container/ensure`, {});
  if (ensure.status !== 204) {
    throw new Error(
      `failed to ensure harness container (${ensure.status}): ${JSON.stringify(ensure.payload || null)}`,
    );
  }
};

const waitForContainerPolicy = async ({
  workspaceId,
  expectedNetworkMode,
  expectedAllowlist,
  forbiddenAllowlist = [],
  timeoutMs = 60_000,
}) => {
  const deadline = Date.now() + timeoutMs;
  let lastContainer = null;
  while (Date.now() < deadline) {
    const container = await getWorkspaceHarnessContainer(workspaceId);
    lastContainer = container;
    const allowlist = readAllowlist(container);
    const hasExpected = expectedAllowlist.every((entry) => allowlist.includes(entry));
    const hasForbidden = forbiddenAllowlist.some((entry) => allowlist.includes(entry));
    if (
      container
      && container.running
      && container.network_mode === expectedNetworkMode
      && hasExpected
      && !hasForbidden
    ) {
      return container;
    }
    await browser.pause(2000);
  }
  throw new Error(
    `container policy did not converge to ${expectedNetworkMode} ${JSON.stringify(expectedAllowlist)}; last=${JSON.stringify(lastContainer)}`,
  );
};

const egressProxyConfigPath = (workspaceId) => {
  const daemonDataDir = String(process.env.CTX_DESKTOP_DAEMON_DATA_DIR || "").trim();
  if (!daemonDataDir) {
    throw new Error("CTX_DESKTOP_DAEMON_DATA_DIR is required for container-network-policy spec");
  }
  return path.join(
    daemonDataDir,
    "containers",
    "workspaces",
    workspaceId,
    "data",
    "egress-proxy.json",
  );
};

const waitForEgressProxyPolicy = async ({
  workspaceId,
  expectedMode,
  expectedAllowlist,
  forbiddenAllowlist = [],
  timeoutMs = 60_000,
  pollMs = 3000,
}) => {
  const deadline = Date.now() + timeoutMs;
  const configPath = egressProxyConfigPath(workspaceId);
  let lastRaw = "";
  while (Date.now() < deadline) {
    if (fs.existsSync(configPath)) {
      try {
        const raw = fs.readFileSync(configPath, "utf8");
        lastRaw = raw;
        const parsed = JSON.parse(raw);
        const allowlist = Array.isArray(parsed.allowlist) ? parsed.allowlist : [];
        const hasExpected = expectedAllowlist.every((entry) => allowlist.includes(entry));
        const hasForbidden = forbiddenAllowlist.some((entry) => allowlist.includes(entry));
        if (String(parsed.mode || "") === expectedMode && hasExpected && !hasForbidden) {
          return parsed;
        }
      } catch (error) {
        lastRaw = `parse_error:${String(error)}`;
      }
    }
    await browser.pause(pollMs);
  }
  throw new Error(
    `egress proxy config did not converge for workspace ${workspaceId}; path=${configPath} last=${normalizeErr(lastRaw)}`,
  );
};

describe("container network policy (desktop e2e)", function () {
  this.timeout(12 * 60_000);
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-container-network-policy-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?containerNetworkPolicy=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("enforces allowlist contract for workspace egress policy", async function () {
    this.timeout(12 * 60_000);
    if (!scenarioEnabled("local-new-disk-isolated", ["local", "container", "disk-isolated"])) this.skip();

    const dest = path.join(localBase, "network-policy");
    const workspaceId = await runWizardScenario({
      location: "local",
      container: "disk-isolated",
      network: "allowlist",
      networkAllowlist: "github.com\nopenrouter.ai",
      harnessDownloads: "skip",
      source: { kind: "new", destPath: dest, workspaceName: "network-policy" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "container_disk_isolated",
      networkMode: "allowlist",
      allowlist: ["github.com", "openrouter.ai"],
    });
    const container = await getWorkspaceHarnessContainer(workspaceId);
    if (!container || !container.running || container.mount_mode !== "disk_isolated") {
      throw new Error(`expected running disk-isolated harness container, got ${JSON.stringify(container)}`);
    }
    await ensureWorkspaceContainer(workspaceId);
    await waitForContainerPolicy({
      workspaceId,
      expectedNetworkMode: "allowlist",
      expectedAllowlist: ["github.com", "openrouter.ai"],
    });
    await waitForEgressProxyPolicy({
      workspaceId,
      expectedMode: "allowlist",
      expectedAllowlist: ["github.com", "openrouter.ai"],
    });

    const update = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
      environment: "container_disk_isolated",
      network_mode: "allowlist",
      allowlist: ["github.com"],
    });
    if (update.status !== 200) {
      throw new Error(`failed to update workspace allowlist (${update.status}): ${JSON.stringify(update.payload)}`);
    }

    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "container_disk_isolated",
      networkMode: "allowlist",
      allowlist: ["github.com"],
    });

    await ensureWorkspaceContainer(workspaceId);
    await waitForContainerPolicy({
      workspaceId,
      expectedNetworkMode: "allowlist",
      expectedAllowlist: ["github.com"],
      forbiddenAllowlist: ["openrouter.ai"],
    });
    await waitForEgressProxyPolicy({
      workspaceId,
      expectedMode: "allowlist",
      expectedAllowlist: ["github.com"],
      forbiddenAllowlist: ["openrouter.ai"],
    });
  });
});
