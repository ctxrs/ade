const fs = require("node:fs");
const path = require("node:path");

const { resolveBoolishFlag } = require("../../../../scripts/lib/boolish.cjs");
const { navigateToTauriUrl } = require("./helpers/tauri.cjs");
const {
  tauriInvoke,
  normalizePhase,
  isRestartReady,
  readCheck,
  readState,
  readAttempt,
  applyUpdate,
  summarize,
  waitForState,
} = require("./helpers/updater_native_proof.cjs");

const reportPath = String(
  process.env.CTX_UPDATER_NATIVE_SMOKE_REPORT
  || process.env.CTX_UPDATER_PROOF_REPORT_PATH
  || path.join("/tmp", "ctx-updater-native-state-report.json"),
).trim();
const specChannel = String(process.env.CTX_UPDATER_E2E_CHANNEL || process.env.CTX_UPDATER_PROOF_CHANNEL || "").trim() || undefined;
const expectAutoReady = resolveBoolishFlag(
  process.env.CTX_UPDATER_PROOF_EXPECT_AUTO_READY,
  false,
  "CTX_UPDATER_PROOF_EXPECT_AUTO_READY",
);
const expectUpdateAvailable = resolveBoolishFlag(
  process.env.CTX_UPDATER_PROOF_EXPECT_UPDATE_AVAILABLE || process.env.CTX_UPDATER_E2E_ASSERT_UPDATE,
  false,
  "CTX_UPDATER_PROOF_EXPECT_UPDATE_AVAILABLE",
);
const expectUpToDate = resolveBoolishFlag(
  process.env.CTX_UPDATER_PROOF_EXPECT_UP_TO_DATE,
  false,
  "CTX_UPDATER_PROOF_EXPECT_UP_TO_DATE",
);
const applyRequested = resolveBoolishFlag(
  process.env.CTX_UPDATER_PROOF_APPLY_UPDATE,
  false,
  "CTX_UPDATER_PROOF_APPLY_UPDATE",
);
const restartRequested = resolveBoolishFlag(
  process.env.CTX_UPDATER_E2E_ASSERT_RESTART,
  false,
  "CTX_UPDATER_E2E_ASSERT_RESTART",
);

const expectedPhases = new Set([
  "idle",
  "checking",
  "staging",
  "staged_ready",
  "restart_required",
]);

const writeReport = (payload) => {
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
};

const assertConfiguredState = (state) => {
  if (typeof state?.configured !== "boolean") {
    throw new Error(`native updater state missing configured flag: ${JSON.stringify(state)}`);
  }
  if (!state.configured) {
    throw new Error(
      `native updater not configured: ${state.message || "missing embedded updater public key"}`,
    );
  }
};

const assertExpectedPhase = (state) => {
  const phase = normalizePhase(state?.phase);
  if (!phase) return;
  if (expectedPhases.has(phase)) return;
  if (state?.available || state?.restart_required) return;
  throw new Error(`unexpected updater phase: ${phase}`);
};

const finalizePayload = async ({ check, state, attempt, apply, autoState }) => ({
  check,
  state,
  attempt,
  apply: apply || null,
  auto_state: autoState || null,
  summary: {
    ...summarize(check, state, attempt),
    route: await browser.getUrl(),
    report_path: reportPath,
  },
});

describe("desktop updater native state smoke", () => {
  it("proves native updater state and manual-check/apply behavior deterministically", async () => {
    await navigateToTauriUrl("tauri://localhost/workspaces");

    let autoState = null;
    let state = await readState(specChannel);
    assertConfiguredState(state);
    assertExpectedPhase(state);
    let check = await readCheck(specChannel);
    let attempt = await readAttempt();
    const initialPayload = await finalizePayload({
      check,
      state,
      attempt,
      apply: null,
      autoState,
    });

    if (expectAutoReady) {
      autoState = await waitForState(specChannel, (candidate) => isRestartReady(candidate), {
        label: "automatic updater readiness",
      });
      state = autoState;
    }

    check = await readCheck(specChannel);
    state = await readState(specChannel);
    assertConfiguredState(state);
    assertExpectedPhase(state);

    if (expectUpdateAvailable && !isRestartReady(state) && !check.available) {
      state = await waitForState(specChannel, (candidate) => isRestartReady(candidate), {
        label: "manual update availability",
      });
      check = await readCheck(specChannel);
    }

    if (expectUpToDate) {
      const phase = normalizePhase(state?.phase);
      if (check.available || state.available || state.restart_required || phase === "staging" || phase === "staged_ready") {
        throw new Error(`expected updater to be up to date, got ${JSON.stringify({ check, state })}`);
      }
    }

    let apply = null;
    if (applyRequested) {
      if (!isRestartReady(state)) {
        state = await waitForState(specChannel, (candidate) => isRestartReady(candidate), {
          label: "apply-eligible updater state",
        });
      }
      apply = await applyUpdate(specChannel);
      if (!apply.applied && !apply.needs_restart && !apply.up_to_date) {
        throw new Error(`unexpected apply response: ${JSON.stringify(apply)}`);
      }
      state = await readState(specChannel);
      check = await readCheck(specChannel);
    }

    attempt = await readAttempt();
    if (attempt && attempt.stage_count !== undefined && attempt.result && !attempt.target_version) {
      throw new Error(`last attempt invalid: ${JSON.stringify(attempt)}`);
    }

    const payload = await finalizePayload({
      check,
      state,
      attempt,
      apply,
      autoState,
    });
    payload.initial = {
      check: initialPayload.check,
      state: initialPayload.state,
      attempt: initialPayload.attempt,
      summary: initialPayload.summary,
    };
    writeReport(payload);

    if (expectUpdateAvailable && !payload.summary.available && !payload.summary.restart_required) {
      throw new Error(`expected update availability, got ${JSON.stringify(payload.summary)}`);
    }

    if (restartRequested && apply?.needs_restart) {
      const restartResp = await tauriInvoke("desktop_restart_app", {});
      if (restartResp.error) {
        throw new Error(`desktop_restart_app failed: ${restartResp.error}`);
      }
    }
  });
});
