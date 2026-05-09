const { navigateToTauriUrl } = require("./helpers/tauri.cjs");

const tauriInvoke = async (command, args) => {
  let result;
  try {
    result = await browser.executeAsync(({ cmd, payload }, done) => {
      const tauriInvoke = window.__TAURI__?.core?.invoke;
      const internalsInvoke = window.__TAURI_INTERNALS__?.invoke;
      const invoke = internalsInvoke || tauriInvoke;
      if (!invoke) {
        done({ error: "Tauri invoke API not available" });
        return;
      }
      Promise.resolve()
        .then(() => invoke(cmd, payload))
        .then((value) => done({ value }))
        .catch((err) => done({ error: String(err) }));
    }, { cmd: command, payload: args });
  } catch (err) {
    return { error: String(err) };
  }

  if (result && typeof result === "object" && Object.prototype.hasOwnProperty.call(result, "error")) {
    return { error: String(result.error || "") };
  }
  return { value: result ? result.value : undefined };
};

describe("updater mismatch actions require explicit confirmation", () => {
  it("rejects local daemon restart when confirm=false", async () => {
    await navigateToTauriUrl("tauri://localhost/workspaces");

    const resp = await tauriInvoke("desktop_restart_local_daemon", { req: { confirm: false } });
    const message = String(resp.error || "").toLowerCase();
    if (!message.includes("confirm required")) {
      throw new Error(`expected confirm required error, got: ${JSON.stringify(resp)}`);
    }
  });

  it("rejects remote daemon update when confirm=false", async () => {
    await navigateToTauriUrl("tauri://localhost/workspaces");

    const resp = await tauriInvoke("desktop_update_remote_daemon", { req: { confirm: false } });
    const message = String(resp.error || "").toLowerCase();
    if (!message.includes("confirm required")) {
      throw new Error(`expected confirm required error, got: ${JSON.stringify(resp)}`);
    }
  });
});
