const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");
const {
  mkTempDir,
  runWizardScenario,
  getWorkspace,
  assertNoDaemonOverlayFor,
  runCodexFirstTurnApiSmoke,
  ensureRemoteTarget,
  ssh,
  REMOTE_HOST,
  REMOTE_WIZARD_HOST_INPUT,
  REMOTE_PORT,
  REMOTE_DATA_DIR_RAW,
} = require("./helpers/workspace_wizard_flow.cjs");
const { ensureCodexOpenRouterWorkspaceReady } = require("./helpers/provider_runtime.cjs");

const reportPath = process.env.CTX_REMOTE_CONTAINER_CONTRACT_REPORT || path.join("/tmp", "ctx-remote-container-contract.json");
const REQUIRE_FIRST_TURN_SUCCESS = ["1", "true", "yes"].includes(
  String(process.env.CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS || "0").trim().toLowerCase(),
);
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

const writeReport = (payload) => {
  try {
    fs.mkdirSync(path.dirname(reportPath), { recursive: true });
    fs.writeFileSync(reportPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  } catch {
    // best effort only
  }
};

describe("remote container contract (env-gated desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-remote-container-contract-${runId}-`);
  let remoteTarget = "";
  let remoteBase = "";
  let remoteDataDir = "";

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?remoteContainerContract=${Date.now()}`);
    await waitForTauri();
    remoteTarget = ensureRemoteTarget() || "";
    remoteBase = `/tmp/ctx-remote-contract-${runId}`;
    remoteDataDir = String(REMOTE_DATA_DIR_RAW || "").trim() || `${remoteBase}/daemon`;
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
    if (remoteTarget) {
      try {
        ssh(remoteTarget, `set -euo pipefail; rm -rf ${JSON.stringify(remoteBase)}`);
      } catch {
        // ignore cleanup failures
      }
    }
  });

  it("creates remote disk-isolated container workspace when remote env is present", async function () {
    this.timeout(12 * 60_000);
    if (!scenarioEnabled("remote-new-disk-isolated", ["remote", "remote-container", "disk-isolated"])) this.skip();
    const report = {
      reportPath,
      skipped: false,
      skip_reason: null,
      remote_host: REMOTE_HOST || null,
      remote_target: remoteTarget || null,
    };

    if (!REMOTE_HOST || !remoteTarget) {
      report.skipped = true;
      report.skip_reason = "missing remote env (CTX_AUTOMATION_REMOTE_HOST)";
      writeReport(report);
      this.skip();
    }

    let podmanPresent = false;
    try {
      const podmanProbe = ssh(
        remoteTarget,
        "if command -v podman >/dev/null 2>&1; then echo yes; else echo no; fi",
      ).trim();
      podmanPresent = podmanProbe === "yes";
    } catch (error) {
      report.skipped = true;
      report.skip_reason = `remote podman probe failed: ${String(error)}`;
      writeReport(report);
      this.skip();
    }

    if (!podmanPresent) {
      report.skipped = true;
      report.skip_reason = "remote podman unavailable";
      writeReport(report);
      this.skip();
    }

    ssh(remoteTarget, `set -euo pipefail; rm -rf ${JSON.stringify(remoteBase)}; mkdir -p ${JSON.stringify(remoteBase)}`);
    const remoteDest = `${remoteBase}/new-disk-isolated`;

    const workspaceId = await runWizardScenario({
      location: "remote",
      remoteHost: REMOTE_WIZARD_HOST_INPUT || REMOTE_HOST,
      remotePort: REMOTE_PORT,
      remoteDataDir,
      container: "disk-isolated",
      network: "full",
      harnessDownloads: "skip",
      source: { kind: "new", destPath: remoteDest, workspaceName: "remote-contract" },
      setupHook: "",
      mergeQueue: { kind: "skip" },
    });

    const ws = await getWorkspace(workspaceId);
    const rootPath = String(ws.root_path || "");
    if (!rootPath.startsWith(remoteBase)) {
      throw new Error(`expected remote root under ${remoteBase}, got ${rootPath}`);
    }

    await assertNoDaemonOverlayFor(20_000);

    let firstTurn = { attempted: false };
    try {
      const provider = await ensureCodexOpenRouterWorkspaceReady(workspaceId, {
        installTarget: "container",
        endpointName: `remote-container-openrouter-${runId}`,
      });
      const turn = await runCodexFirstTurnApiSmoke(workspaceId, {
        providerId: provider.providerId,
        modelId: provider.modelId,
      }, 180000);
      firstTurn = {
        attempted: true,
        status: "success",
        session_id: turn?.sessionId || null,
        assistant_preview: String(turn?.assistantMessage || "").slice(0, 200),
        provider,
      };
    } catch (error) {
      firstTurn = {
        attempted: true,
        status: "failed",
        error: String(error),
      };
      if (REQUIRE_FIRST_TURN_SUCCESS) {
        report.first_turn = firstTurn;
        writeReport(report);
        throw error;
      }
    }

    report.workspace_id = workspaceId;
    report.root_path = rootPath;
    report.first_turn = firstTurn;
    writeReport(report);
  });
});
