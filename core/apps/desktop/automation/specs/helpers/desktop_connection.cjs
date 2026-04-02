const tauriInvoke = async (command, args) => {
  try {
    const result = await browser.executeAsync(({ cmd, payload }, done) => {
      const tauriCoreInvoke = window.__TAURI__?.core?.invoke;
      const tauriInternalsInvoke = window.__TAURI_INTERNALS__?.invoke;
      const invoke = tauriInternalsInvoke || tauriCoreInvoke;
      if (!invoke) {
        done({ error: "Tauri invoke API not available" });
        return;
      }
      Promise.resolve()
        .then(() => invoke(cmd, payload))
        .then((value) => done({ value }))
        .catch((err) => done({ error: String(err) }));
    }, { cmd: command, payload: args });
    return result && typeof result === "object" ? result : { value: result };
  } catch (err) {
    return { error: String(err) };
  }
};

const connectSshWithPolling = async (req, timeoutMs = 240000) => {
  const begin = await tauriInvoke("desktop_connect_ssh_begin", { req });
  if (begin.error) {
    const detail = String(begin.error || "").toLowerCase();
    if (detail.includes("desktop_connect_ssh_begin") || detail.includes("unknown command")) {
      return tauriInvoke("desktop_connect_ssh", { req });
    }
    return begin;
  }

  const jobId = String(begin.value || "").trim();
  if (!jobId) {
    return { error: "desktop_connect_ssh_begin returned empty job id" };
  }

  const startedAt = Date.now();
  let lastError = "";
  while (Date.now() - startedAt < timeoutMs) {
    const poll = await tauriInvoke("desktop_connect_ssh_poll", {
      req: { job_id: jobId, consume: false },
    });
    if (poll.error) {
      lastError = String(poll.error || "");
      await browser.pause(500);
      continue;
    }
    const snapshot = poll.value || {};
    const status = String(snapshot.status || "").trim().toLowerCase();
    if (status === "succeeded") {
      await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
      return { value: snapshot.info || null };
    }
    if (status === "failed") {
      await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
      return { error: String(snapshot.error || "desktop_connect_ssh failed") };
    }
    await browser.pause(500);
  }

  await tauriInvoke("desktop_connect_ssh_poll", { req: { job_id: jobId, consume: true } });
  return {
    error: `desktop_connect_ssh timed out waiting for async completion${lastError ? `: ${lastError}` : ""}`,
  };
};

module.exports = {
  tauriInvoke,
  connectSshWithPolling,
};
