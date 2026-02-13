const waitForTauri = async () => {
  await browser.waitUntil(
    async () => {
      const hasTauri = await browser.execute(() => Boolean(window.__TAURI__));
      return Boolean(hasTauri);
    },
    { timeout: 30000, timeoutMsg: "Tauri bridge not available in time." },
  );
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
  selectorForTestId,
  waitForTestId,
  clickTestId,
  setInputTestId,
  getConnectionInfo,
};
