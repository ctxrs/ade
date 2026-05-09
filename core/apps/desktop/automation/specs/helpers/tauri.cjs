const waitForTauri = async () => {
  await browser.waitUntil(
    async () => {
      const hasTauri = await browser.execute(() => Boolean(window.__TAURI__));
      return Boolean(hasTauri);
    },
    { timeout: 30000, timeoutMsg: "Tauri bridge not available in time." },
  );
};

const normalizeTauriPathname = (pathname) => String(pathname || "").trim() || "/";

const expectedRouteForTauriUrl = (target, options = {}) => {
  const url = new URL(target);
  return {
    host: url.host,
    matchSearch: options.matchSearch !== false,
    pathname: normalizeTauriPathname(url.pathname),
    protocol: url.protocol,
    search: url.search,
  };
};

const errorMessage = (error) => String(error && error.message ? error.message : error);

const navigateToTauriUrl = async (target, options = {}) => {
  const expected = options.route || expectedRouteForTauriUrl(target, options);
  const timeoutMs = options.timeoutMs || 60000;
  const scriptTimeoutMs = options.scriptTimeoutMs || 10000;
  let lastScriptError = "";
  try {
    await browser.waitUntil(
      async () => {
        try {
          return Boolean(await browser.execute((href, route) => {
            if (!window.location) return false;
            const pathname = String(window.location.pathname || "").trim() || "/";
            if (
              window.location.protocol === route.protocol
              && window.location.host === route.host
              && pathname === route.pathname
              && (route.matchSearch === false || window.location.search === route.search)
            ) {
              return true;
            }
            window.location.assign(href);
            return true;
          }, target, expected));
        } catch (error) {
          lastScriptError = errorMessage(error);
          return false;
        }
      },
      {
        interval: 250,
        timeout: scriptTimeoutMs,
        timeoutMsg: `Tauri script navigation did not start for ${target}`,
      },
    );
  } catch (error) {
    const detail = lastScriptError || errorMessage(error);
    throw new Error(
      `Tauri script navigation failed for ${target}: ${detail}`,
    );
  }
  let lastRouteError = "";
  try {
    await browser.waitUntil(
      async () => {
        try {
          return await browser.execute((route) => {
            const pathname = String(window.location.pathname || "").trim() || "/";
            return Boolean(window.__TAURI__)
              && document.readyState !== "loading"
              && window.location.protocol === route.protocol
              && window.location.host === route.host
              && pathname === route.pathname
              && (route.matchSearch === false || window.location.search === route.search);
          }, expected);
        } catch (error) {
          lastRouteError = errorMessage(error);
          return false;
        }
      },
      {
        timeout: timeoutMs,
        timeoutMsg: `Tauri route did not reach ${target}`,
      },
    );
  } catch (error) {
    const detail = lastRouteError || errorMessage(error);
    throw new Error(`Tauri route did not reach ${target}: ${detail}`);
  }
};

const selectorForTestId = (id) => `[data-testid="${id}"]`;

const waitForTestId = async (id, timeoutMs = 30000) => {
  const sel = selectorForTestId(id);
  await browser.waitUntil(
    async () => await browser.execute((s) => Boolean(document.querySelector(s)), sel),
    { timeout: timeoutMs, timeoutMsg: `element not found: ${sel}` },
  );
};

const clickTestId = async (id) => {
  const sel = selectorForTestId(id);
  await browser.waitUntil(
    async () => await browser.execute((s) => {
      const el = document.querySelector(s);
      if (!el) return false;
      el.click();
      return true;
    }, sel),
    { timeout: 30000, timeoutMsg: `failed to click: ${sel}` },
  );
};

const setInputTestId = async (id, value) => {
  await waitForTestId(id);
  const sel = selectorForTestId(id);
  const ok = await browser.execute((s, v) => {
    const el = document.querySelector(s);
    if (!el) return false;
    const isTextArea = el instanceof HTMLTextAreaElement;
    const proto = isTextArea ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    const desc = Object.getOwnPropertyDescriptor(proto, "value");
    const setter = desc && desc.set;
    if (!setter) return false;
    setter.call(el, v);
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  }, sel, String(value));
  if (!ok) throw new Error(`failed to set value for: ${sel}`);
};

const getConnectionInfo = async () => {
  const result = await browser.execute(async () => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { error: "Tauri invoke not available" };
    try {
      const info = await invoke("desktop_get_connection");
      return { info };
    } catch (e) {
      return { error: String(e) };
    }
  });
  if (result && typeof result === "object" && result.error) {
    throw new Error(result.error);
  }
  return result.info;
};

module.exports = {
  waitForTauri,
  navigateToTauriUrl,
  selectorForTestId,
  waitForTestId,
  clickTestId,
  setInputTestId,
  getConnectionInfo,
};
