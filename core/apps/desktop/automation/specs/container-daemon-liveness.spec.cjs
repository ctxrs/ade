const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson, sampleDaemonHealth } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  runWizardScenario,
  getWorkspaceHarnessContainer,
  assertLocalWorkspaceConfig,
  assertConnectedLocalAndListening,
  assertNoDaemonOverlayFor,
  collectWorkspaceRouteDiagnostics,
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
const CASE_TIMEOUT_MS = Number.parseInt(
  String(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "900000"),
  10,
) || 900000;

const readLaunchPanelDiagnostics = async () => {
  return await browser.execute(() => {
    const panel = document.querySelector('[data-testid="wizard-launch-log-panel"]');
    const copyButton = document.querySelector('[data-testid="wizard-launch-copy"]');
    const lines = Array.from(document.querySelectorAll(".wizard-launch-log-line")).map((row) => {
      const ts = String(row.querySelector(".wizard-launch-log-ts")?.textContent || "").trim();
      const phase = String(row.querySelector(".wizard-launch-log-phase")?.textContent || "").trim();
      const level = String(row.querySelector(".wizard-launch-log-level")?.textContent || "").trim();
      const message = String(row.querySelector(".wizard-launch-log-msg")?.textContent || "").trim();
      return { ts, phase, level, message };
    });
    return {
      hasPanel: Boolean(panel),
      copyLabel: copyButton ? String(copyButton.textContent || "").trim() : "",
      lines,
    };
  });
};

const runLocalContainerCreate = async ({ container, workspaceName, destPath, network = "full" }) => {
  try {
    return await runWizardScenario({
      location: "local",
      container,
      network,
      networkAllowlist: network === "allowlist" ? "github.com\nregistry.npmjs.org" : "",
      harnessDownloads: "skip",
      source: { kind: "new", destPath, workspaceName },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });
  } catch (error) {
    const launch = await readLaunchPanelDiagnostics();
    const routeDiag = await collectWorkspaceRouteDiagnostics();
    const hasDetailedError = launch.lines.some((line) => line.level.toLowerCase() === "error" && line.message.length > 8);
    if (!hasDetailedError) {
      throw new Error(
        `container launch failed without detailed launch diagnostics; error=${String(error)} launch=${JSON.stringify(launch)} route=${JSON.stringify(routeDiag)}`,
      );
    }
    throw new Error(
      `container launch failed; error=${String(error)} launch=${JSON.stringify(launch)} route=${JSON.stringify(routeDiag)}`,
    );
  }
};

describe("container daemon liveness", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-container-daemon-liveness-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?containerDaemonLiveness=${Date.now()}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("local host-mounted container create keeps daemon healthy", async function () {
    if (!scenarioEnabled("local-new-host-mounted", ["local", "container", "host-mounted"])) this.skip();

    const dest = path.join(localBase, "host-mounted");
    const workspaceId = await runLocalContainerCreate({
      container: "host-mounted",
      workspaceName: "container-daemon-host",
      destPath: dest,
      network: "allowlist",
    });

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "container_host_mounted",
      networkMode: "allowlist",
      allowlist: ["github.com", "registry.npmjs.org"],
    });
    const container = await getWorkspaceHarnessContainer(workspaceId);
    if (!container || !container.running || container.mount_mode !== "host_mounted") {
      throw new Error(`expected running host-mounted harness container, got ${JSON.stringify(container)}`);
    }
    await assertNoDaemonOverlayFor(20_000);
    const health = await sampleDaemonHealth({ durationMs: 20_000, intervalMs: 2_000 });
    if (!health.ok) {
      throw new Error(`daemon health degraded after host-mounted create: ${JSON.stringify(health.failures)}`);
    }
  }).timeout(CASE_TIMEOUT_MS);

  it("local disk-isolated container create keeps daemon healthy", async function () {
    if (!scenarioEnabled("local-new-disk-isolated", ["local", "container", "disk-isolated"])) this.skip();

    const dest = path.join(localBase, "disk-isolated");
    const workspaceId = await runLocalContainerCreate({
      container: "disk-isolated",
      workspaceName: "container-daemon-disk",
      destPath: dest,
      network: "full",
    });

    await assertConnectedLocalAndListening();
    await assertLocalWorkspaceConfig(workspaceId, {
      environment: "container_disk_isolated",
      networkMode: "all",
    });
    const container = await getWorkspaceHarnessContainer(workspaceId);
    if (!container || !container.running || container.mount_mode !== "disk_isolated") {
      throw new Error(`expected running disk-isolated harness container, got ${JSON.stringify(container)}`);
    }

    const tasksResp = await daemonJson("GET", `/api/workspaces/${workspaceId}/tasks`);
    if (tasksResp.status !== 200) {
      throw new Error(`failed to read workspace tasks after create (${tasksResp.status})`);
    }

    await assertNoDaemonOverlayFor(30_000);
    const health = await sampleDaemonHealth({ durationMs: 30_000, intervalMs: 2_000 });
    if (!health.ok) {
      throw new Error(`daemon health degraded after disk-isolated create: ${JSON.stringify(health.failures)}`);
    }
  }).timeout(CASE_TIMEOUT_MS);
});
