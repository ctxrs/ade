const daemonJson = async (method, apiPath, body) => {
  const exec = async (m, p, b) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const headers = [["content-type", "application/json"]];
      const req = {
        method: m,
        path: p,
        // CrabNebula arg marshaller is strict with null payloads.
        body: typeof b === "undefined" ? null : JSON.stringify(b),
        headers,
      };
      const raw = await invoke("desktop_daemon_request", { req });
      const payload = JSON.parse(raw.body || "{}");
      return { status: raw.status, payload };
    } catch (e) {
      return { error: String(e) };
    }
  };

  const resp = typeof body === "undefined"
    ? await browser.execute(exec, method, apiPath)
    : await browser.execute(exec, method, apiPath, body);
  if (resp && typeof resp === "object" && resp.error) {
    throw new Error(resp.error);
  }
  return resp;
};

const safeDaemonJson = async (method, apiPath, body) => {
  try {
    return await daemonJson(method, apiPath, body);
  } catch (error) {
    return { error: String(error) };
  }
};

module.exports = {
  daemonJson,
  safeDaemonJson,
};

