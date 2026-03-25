const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { safeDaemonJson } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  runWizardScenario,
  getWorkspace,
  assertNoDaemonOverlayFor,
  collectWorkspaceRouteDiagnostics,
} = require("./helpers/workspace_wizard_flow.cjs");
const { ensureCodexOpenRouterWorkspaceReady } = require("./helpers/provider_runtime.cjs");
const { runDeterministicFirstTurnOutcome } = require("./helpers/first_turn_contract.cjs");
const {
  createRemoteContractRecorder,
  parseBoolean,
  resolveRemoteFixtureEnv,
} = require("../helpers/remote_fixture_contract.cjs");

const reportPath = String(
  process.env.CTX_REMOTE_CONTAINER_CONTRACT_REPORT || path.join("/tmp", "ctx-remote-container-contract.json"),
).trim();
const REQUIRE_FIRST_TURN_SUCCESS = parseBoolean(process.env.CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS || "0");
const fixture = resolveRemoteFixtureEnv({ lane: "sandbox" });
const scenarioFilter = new Set(
  String(process.env.CTX_AUTOMATION_SCENARIOS || "")
    .split(",")
    .map((entry) => entry.trim().toLowerCase())
    .filter(Boolean),
);

const scenarioEnabled = (name, tags = []) => {
  if (scenarioFilter.size === 0) return true;
  return [name, ...tags].some((entry) => scenarioFilter.has(String(entry).trim().toLowerCase()));
};

let contractRecorder = null;

const sshBaseArgs = ({ useKey = true } = {}) => {
  const args = [
    "-o",
    "StrictHostKeyChecking=no",
    "-o",
    "UserKnownHostsFile=/dev/null",
    "-o",
    "ConnectTimeout=10",
  ];
  if (fixture.sshConfigPath) {
    args.unshift(fixture.sshConfigPath);
    args.unshift("-F");
  }
  if (fixture.sshPort > 0) {
    args.push("-p", String(fixture.sshPort));
  }
  if (useKey && fixture.sshKeyPath) {
    args.unshift("IdentitiesOnly=yes");
    args.unshift("-o");
    args.unshift(fixture.sshKeyPath);
    args.unshift("-i");
  }
  return args;
};

const remoteSsh = (command, { auth = "key", label = "ssh" } = {}) => {
  const usePassword = auth === "password";
  const args = sshBaseArgs({ useKey: !usePassword });
  let binary = "ssh";
  let finalArgs;
  let env = process.env;

  if (usePassword) {
    const password = String(fixture.passwordActual || fixture.password || "").trim();
    if (!password) {
      throw new Error("password auth requested but the remote sandbox fixture password is not set");
    }
    binary = "sshpass";
    finalArgs = [
      "-e",
      "ssh",
      ...args,
      "-o",
      "BatchMode=no",
      "-o",
      "PreferredAuthentications=password,keyboard-interactive",
      "-o",
      "NumberOfPasswordPrompts=1",
      fixture.target,
      command,
    ];
    env = { ...process.env, SSHPASS: password };
  } else {
    finalArgs = [...args, "-o", "BatchMode=yes", fixture.target, command];
  }

  const result = spawnSync(binary, finalArgs, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
    env,
  });
  contractRecorder?.recordSshTranscript(label, {
    auth,
    binary,
    args: finalArgs,
    command,
    target: fixture.target,
    exit_code: result.status ?? null,
    signal: result.signal ?? null,
    ok: result.status === 0,
    stdout: String(result.stdout || "").trim(),
    stderr: String(result.stderr || "").trim(),
  });
  if (result.status !== 0) {
    const stderr = String(result.stderr || "").trim();
    const stdout = String(result.stdout || "").trim();
    const detail = [
      `command failed: ${binary} ${finalArgs.join(" ")}`,
      stderr ? `stderr: ${stderr}` : null,
      stdout ? `stdout: ${stdout}` : null,
    ]
      .filter(Boolean)
      .join("\n");
    throw new Error(detail);
  }
  return String(result.stdout || "").trim();
};

const collectFailureArtifacts = async (stage) => {
  if (!contractRecorder) return;

  contractRecorder.recordArtifact(`${stage}_workspace_route`, await collectWorkspaceRouteDiagnostics());
  contractRecorder.recordArtifact(`${stage}_daemon_health`, await safeDaemonJson("GET", "/api/health"));
  contractRecorder.recordArtifact(`${stage}_daemon_diagnostics`, await safeDaemonJson("GET", "/api/diagnostics"));

  if (!fixture.target || !fixture.dataDir) return;

  const remoteLogFile = `${fixture.dataDir.replace(/\/+$/, "")}/logs/daemon.log`;
  const tailCmd = `if [ -f ${JSON.stringify(remoteLogFile)} ]; then tail -n 200 ${JSON.stringify(remoteLogFile)}; else echo "__CTX_MISSING__ ${remoteLogFile}"; fi`;
  try {
    const tail = remoteSsh(`sh -lc ${JSON.stringify(tailCmd)}`, {
      auth: fixture.authMode === "password" ? "password" : "key",
      label: `${stage}-remote-daemon-log-tail`,
    });
    contractRecorder.recordArtifact(`${stage}_remote_daemon_log_tail`, {
      log_file: remoteLogFile,
      tail,
    });
  } catch (error) {
    contractRecorder.recordArtifact(`${stage}_remote_daemon_log_tail`, {
      log_file: remoteLogFile,
      error: String(error),
    });
  }
};

describe("remote sandbox contract (env-gated desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const localBase = mkTempDir(`ctx-remote-container-contract-${runId}-`);
  const remoteBase = `/tmp/ctx-remote-contract-${runId}`;
  const remoteDataDir = fixture.dataDir || `${remoteBase}/daemon`;

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }

    if (!fixture.ready || !fixture.target) return;

    try {
      remoteSsh(`set -euo pipefail; rm -rf ${JSON.stringify(remoteBase)}`, {
        auth: fixture.authMode === "password" ? "password" : "key",
        label: "suite-cleanup",
      });
    } catch {
      // ignore cleanup failures
    }
  });

  it("creates remote sandbox workspace when remote env is present", async function () {
    this.timeout(12 * 60_000);

    contractRecorder = createRemoteContractRecorder({
      outputPath: reportPath,
      suite: "remote sandbox contract",
      lane: "sandbox",
      fixture,
      secretValues: [fixture.password, fixture.passwordActual, process.env.OPENROUTER_API_KEY],
    });

    let finalized = false;
    const finalizeReport = (payload) => {
      if (finalized) return null;
      finalized = true;
      return contractRecorder.finalize(payload);
    };

    let workspaceId = "";
    let rootPath = "";
    let provider = null;
    let firstTurn = { attempted: false };
    let finalResult = "passed";
    let finalReason = "remote sandbox contract validated";
    let finalError = "";

    contractRecorder.recordArtifact("fixture_preflight", {
      report_path: reportPath,
      require_first_turn_success: REQUIRE_FIRST_TURN_SUCCESS,
      scenario_filter: Array.from(scenarioFilter.values()),
      fixture,
    });

    if (!scenarioEnabled("remote-new-sandbox", ["remote", "sandbox"])) {
      const detail = "scenario filter excluded remote-new-sandbox";
      contractRecorder.recordAssertion("scenario_filter", "skip", detail);
      finalizeReport({ result: "skipped", reason: detail });
      this.skip();
    }

    if (!fixture.ready) {
      const detail = fixture.preflightMessage;
      if (fixture.strictRequired) {
        contractRecorder.recordAssertion("fixture_preflight", "fail", detail);
        finalizeReport({ result: "failed", reason: detail, error: detail });
        throw new Error(detail);
      }
      contractRecorder.recordAssertion("fixture_preflight", "skip", detail);
      finalizeReport({ result: "skipped", reason: detail });
      this.skip();
    }

    try {
      contractRecorder.recordAssertion("fixture_preflight", "pass", "resolved remote sandbox fixture contract");

      await browser.url(`tauri://localhost/workspace-setup?remoteContainerContract=${Date.now()}`);
      await waitForTauri();

      const sandboxCliProbe = remoteSsh(
        "if { [ -n \"${CTX_HARNESS_SANDBOX_CLI_PATH:-}\" ] && [ -x \"${CTX_HARNESS_SANDBOX_CLI_PATH}\" ]; } || command -v nerdctl >/dev/null 2>&1; then echo yes; else echo no; fi",
        {
          auth: fixture.authMode === "password" ? "password" : "key",
          label: "sandbox-cli-probe",
        },
      ).trim();
      contractRecorder.recordArtifact("sandbox_cli_probe", { result: sandboxCliProbe });
      if (sandboxCliProbe !== "yes") {
        const detail = "remote sandbox substrate unavailable";
        contractRecorder.recordAssertion("sandbox_cli_probe", "skip", detail);
        finalizeReport({ result: "skipped", reason: detail });
        this.skip();
      }
      contractRecorder.recordAssertion("sandbox_cli_probe", "pass", "remote sandbox substrate available");

      remoteSsh(
        `set -euo pipefail; rm -rf ${JSON.stringify(remoteBase)}; mkdir -p ${JSON.stringify(remoteBase)}`,
        {
          auth: fixture.authMode === "password" ? "password" : "key",
          label: "remote-base-prepare",
        },
      );

      const remoteDest = `${remoteBase}/new-sandbox`;
      workspaceId = await runWizardScenario({
        location: "remote",
        remoteHost: fixture.wizardHostInput,
        remotePort: fixture.port,
        remoteDataDir,
        container: "sandbox",
        network: "full",
        harnessDownloads: "skip",
        source: { kind: "new", destPath: remoteDest, workspaceName: "remote-contract" },
        setupHook: "",
        mergeQueue: { kind: "skip" },
      });

      const workspace = await getWorkspace(workspaceId);
      rootPath = String(workspace.root_path || "");
      contractRecorder.recordArtifact("workspace", workspace);
      if (!rootPath.startsWith(remoteBase)) {
        throw new Error(`expected remote root under ${remoteBase}, got ${rootPath}`);
      }

      await assertNoDaemonOverlayFor(20_000);
      contractRecorder.recordAssertion("workspace_launch", "pass", "remote workspace launched without daemon overlay");

      try {
        provider = await ensureCodexOpenRouterWorkspaceReady(workspaceId, {
          installTarget: "container",
          endpointName: `remote-container-openrouter-${runId}`,
        });
        contractRecorder.recordArtifact("provider_verify_payload", provider.verifyPayload || null);
        firstTurn = await runDeterministicFirstTurnOutcome(
          workspaceId,
          {
            providerId: provider.providerId,
            modelId: provider.modelId,
            prompt: "hello",
            timeoutMs: 180000,
          },
        );
        firstTurn = { attempted: true, ...firstTurn, provider };
        if (REQUIRE_FIRST_TURN_SUCCESS && firstTurn.status !== "success") {
          const failure = new Error(`expected first turn success, got ${JSON.stringify(firstTurn)}`);
          failure.firstTurnOutcome = firstTurn;
          throw failure;
        }
      } catch (error) {
        firstTurn = error?.firstTurnOutcome || {
          attempted: true,
          status: "failed",
          error: String(error),
        };
        if (REQUIRE_FIRST_TURN_SUCCESS) {
          throw error;
        }
      }

      contractRecorder.recordArtifact("first_turn", firstTurn);
      contractRecorder.recordAssertion(
        "first_turn",
        firstTurn.status === "success" ? "pass" : "warn",
        firstTurn.status === "success" ? "remote sandbox first turn succeeded" : JSON.stringify(firstTurn),
      );
    } catch (error) {
      finalResult = "failed";
      finalReason = "remote sandbox contract failed";
      finalError = String(error);
      contractRecorder.recordAssertion("contract", "fail", finalError);
      await collectFailureArtifacts("failure");
      throw error;
    } finally {
      finalizeReport({
        result: finalResult,
        reason: finalReason,
        error: finalError,
        extras: {
          workspace_id: workspaceId || null,
          root_path: rootPath || null,
          provider_config: provider,
          first_turn: firstTurn,
        },
      });
    }
  });
});
