const fs = require("fs");
const path = require("path");

const { waitForTauri } = require("./helpers/tauri.cjs");

const reportPath = process.env.CTX_UPDATER_NATIVE_SMOKE_REPORT || path.join("/tmp", "ctx-updater-native-state-report.json");
const specChannel = process.env.CTX_UPDATER_E2E_CHANNEL || undefined;

const tauriInvoke = async (command, payload) => {
  let result;
  try {
    result = await browser.executeAsync(({ cmd, args }, done) => {
      const invoke = window.__TAURI__?.core?.invoke;
      if (!invoke) {
        done({ error: "Tauri invoke API not available" });
        return;
      }
      Promise.resolve()
        .then(() => invoke(cmd, args))
        .then((value) => done({ value }))
        .catch((err) => done({ error: String(err) }));
    }, { cmd: command, args: payload });
  } catch (err) {
    return { error: String(err) };
  }
  if (result && typeof result === "object" && Object.prototype.hasOwnProperty.call(result, "error")) {
    return { error: String(result.error || "") };
  }
  return { value: result ? result.value : undefined };
};

const writeReport = (payload) => {
  try {
    fs.mkdirSync(path.dirname(reportPath), { recursive: true });
    fs.writeFileSync(reportPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  } catch {
    // best effort only
  }
};

const isExpectedPhases = new Set([
  "idle",
  "staging",
  "staged_ready",
  "restart_required",
]);

const readUpdateState = async (stateReq) => {
  const stateResp = await tauriInvoke("desktop_get_app_update_state", stateReq);
  if (stateResp.error) {
    throw new Error(`desktop_get_app_update_state failed: ${stateResp.error}`);
  }
  return stateResp.value || {};
};

const readUpdateCheck = async (stateReq) => {
  const checkResp = await tauriInvoke("desktop_check_app_update", stateReq);
  if (checkResp.error) {
    throw new Error(`desktop_check_app_update failed: ${checkResp.error}`);
  }
  return checkResp.value || {};
};

const readLastAttempt = async () => {
  const attemptResp = await tauriInvoke("desktop_get_last_app_update_attempt", undefined);
  return attemptResp.value ? attemptResp.value : null;
};

const normalizePhase = (value) => String(value || "").trim().toLowerCase();

const waitForApplyEligibleState = async (stateReq, initialState) => {
  let observed = initialState || {};
  const timeoutMs = Number(process.env.CTX_UPDATER_E2E_WAIT_TIMEOUT_MS || 120000);
  const intervalMs = Number(process.env.CTX_UPDATER_E2E_WAIT_INTERVAL_MS || 1000);
  const deadline = Date.now() + timeoutMs;

  while (Date.now() < deadline) {
    const phase = normalizePhase(observed.phase);
    if (observed.restart_required || (observed.available && phase === "staged_ready")) {
      return observed;
    }
    if (phase === "failed") {
      throw new Error(`desktop updater entered failed phase: ${JSON.stringify(observed)}`);
    }
    await new Promise((resolve) => setTimeout(resolve, intervalMs));
    observed = await readUpdateState(stateReq);
  }

  throw new Error(
    `desktop updater did not become apply-eligible within ${timeoutMs}ms: ${JSON.stringify(observed)}`,
  );
};

const buildSummary = async (check, state) => {
  const phase = normalizePhase(state.phase || check.phase);
  return {
    current_version: String(check.current_version || state.current_version || "").trim(),
    latest_version: check.latest_version || state.latest_version || null,
    available: Boolean(check.available || state.available || false),
    restart_required: Boolean(check.restart_required || state.restart_required || false),
    configured: Boolean(check.configured || state.configured || false),
    phase: phase || null,
    target: String(check.target || state.target || "").trim() || null,
    endpoint: String(check.endpoint || state.endpoint || "").trim() || null,
    message: check.message || state.message || null,
    last_attempt_id: check.last_attempt_id || state.last_attempt_id || null,
    last_error: check.last_error || state.last_error || null,
    check_failed: false,
    route: await browser.getUrl(),
    reportPath,
  };
};

describe("desktop updater native state smoke", () => {
  it("query plugin state and surface native error state deterministically", async () => {
    await browser.url("tauri://localhost/workspaces");
    await waitForTauri();

    const stateReq = { req: specChannel ? { channel: specChannel } : {} };

    let check = await readUpdateCheck(stateReq);
    let state = await readUpdateState(stateReq);
    if (typeof state.configured !== "boolean") {
      throw new Error(`native state missing configured flag: ${JSON.stringify(state)}`);
    }
    if (state.configured === false) {
      throw new Error(
        `native updater not configured: ${state.message || "missing CTX_DESKTOP_UPDATER_PUBKEY"}`,
      );
    }
    const phase = normalizePhase(state.phase);
    if (phase && !isExpectedPhases.has(phase) && !state.available && !state.restart_required) {
      throw new Error(`unexpected updater phase: ${phase}`);
    }

    if (
      process.env.CTX_UPDATER_E2E_ASSERT_UPDATE === "1"
      && !state.restart_required
      && phase === "staging"
    ) {
      state = await waitForApplyEligibleState(stateReq, state);
      check = await readUpdateCheck(stateReq);
    }

    let summary = await buildSummary(check, state);
    let attempt = await readLastAttempt();
    if (attempt && attempt.stage_count !== undefined && attempt.result && !attempt.target_version) {
      throw new Error(`last attempt invalid: ${JSON.stringify(attempt)}`);
    }

    let payload = {
      check,
      state,
      attempt,
      summary,
    };
    writeReport(payload);

    if (process.env.CTX_UPDATER_E2E_ASSERT_UPDATE === "1" && summary.available && !summary.restart_required) {
      const applyResp = await tauriInvoke("desktop_apply_app_update", {
        req: {
          confirm: true,
          channel: specChannel,
        },
      });
      if (applyResp.error) {
        throw new Error(`desktop_apply_app_update failed: ${applyResp.error}`);
      }
      const apply = applyResp.value || {};
      if (!apply.applied && !apply.needs_restart && !apply.up_to_date) {
        throw new Error(`unexpected apply response: ${JSON.stringify(apply)}`);
      }
      state = await readUpdateState(stateReq);
      check = await readUpdateCheck(stateReq);
      attempt = await readLastAttempt();
      summary = await buildSummary(check, state);
      payload = {
        check,
        state,
        attempt,
        summary,
        apply,
      };
      writeReport(payload);
      if (process.env.CTX_UPDATER_E2E_ASSERT_RESTART === "1" && apply.needs_restart) {
        const restartResp = await tauriInvoke("desktop_restart_app", {});
        if (restartResp.error) {
          throw new Error(`desktop_restart_app failed: ${restartResp.error}`);
        }
      }
    }
  });
});
