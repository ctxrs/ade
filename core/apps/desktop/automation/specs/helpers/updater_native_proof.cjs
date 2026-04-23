const normalizePhase = (value) => String(value || "").trim().toLowerCase();

const normalizeOptionalString = (value) => {
  const text = String(value || "").trim();
  return text || null;
};

const tauriInvoke = async (command, payload) => {
  let result;
  try {
    result = await browser.executeAsync(({ cmd, args }, done) => {
      const tauriInvoke = window.__TAURI__?.core?.invoke;
      const internalsInvoke = window.__TAURI_INTERNALS__?.invoke;
      const invoke = internalsInvoke || tauriInvoke;
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

const isRestartReady = (state) => {
  const phase = normalizePhase(state?.phase);
  return Boolean(state?.restart_required) || (Boolean(state?.available) && phase === "staged_ready");
};

const readCheck = async (channel) => {
  const resp = await tauriInvoke("desktop_check_app_update", {
    req: channel ? { channel } : {},
  });
  if (resp.error) {
    throw new Error(`desktop_check_app_update failed: ${resp.error}`);
  }
  return resp.value || {};
};

const readState = async (channel) => {
  const resp = await tauriInvoke("desktop_get_app_update_state", {
    req: channel ? { channel } : {},
  });
  if (resp.error) {
    throw new Error(`desktop_get_app_update_state failed: ${resp.error}`);
  }
  return resp.value || {};
};

const readAttempt = async () => {
  const resp = await tauriInvoke("desktop_get_last_app_update_attempt", undefined);
  if (resp.error) return null;
  return resp.value || null;
};

const applyUpdate = async (channel) => {
  const resp = await tauriInvoke("desktop_apply_app_update", {
    req: {
      confirm: true,
      ...(channel ? { channel } : {}),
    },
  });
  if (resp.error) {
    throw new Error(`desktop_apply_app_update failed: ${resp.error}`);
  }
  return resp.value || {};
};

const summarize = (check, state, attempt) => {
  const latestVersion = normalizeOptionalString(check?.latest_version ?? state?.latest_version);
  const currentVersion = normalizeOptionalString(check?.current_version ?? state?.current_version);
  return {
    current_version: currentVersion,
    latest_version: latestVersion,
    available: Boolean(check?.available ?? state?.available),
    restart_required: Boolean(check?.restart_required ?? state?.restart_required),
    configured: Boolean(check?.configured ?? state?.configured),
    phase: normalizeOptionalString(state?.phase ?? check?.phase),
    endpoint: normalizeOptionalString(check?.endpoint ?? state?.endpoint),
    target: normalizeOptionalString(check?.target ?? state?.target),
    message: normalizeOptionalString(check?.message ?? state?.message),
    last_attempt_id: normalizeOptionalString(check?.last_attempt_id ?? state?.last_attempt_id),
    last_error: normalizeOptionalString(check?.last_error ?? state?.last_error),
    attempt_result: normalizeOptionalString(attempt?.result),
    attempt_target_version: normalizeOptionalString(attempt?.target_version),
  };
};

const waitForState = async (
  channel,
  predicate,
  {
    timeoutMs = Number.parseInt(String(process.env.CTX_UPDATER_E2E_WAIT_TIMEOUT_MS || "120000"), 10) || 120000,
    intervalMs = Number.parseInt(String(process.env.CTX_UPDATER_E2E_WAIT_INTERVAL_MS || "1000"), 10) || 1000,
    label = "updater state predicate",
  } = {},
) => {
  const startedAt = Date.now();
  let state = await readState(channel);
  while (Date.now() - startedAt < timeoutMs) {
    if (predicate(state)) {
      return state;
    }
    if (normalizePhase(state?.phase) === "failed") {
      throw new Error(`${label} failed: updater entered failed phase (${JSON.stringify(state)})`);
    }
    await browser.pause(intervalMs);
    state = await readState(channel);
  }
  throw new Error(`${label} timed out after ${timeoutMs}ms (${JSON.stringify(state)})`);
};

module.exports = {
  tauriInvoke,
  normalizePhase,
  normalizeOptionalString,
  isRestartReady,
  readCheck,
  readState,
  readAttempt,
  applyUpdate,
  summarize,
  waitForState,
};
