const DEFAULT_HTTP_URL = "http://127.0.0.1:6160";
const DEFAULT_WS_PATH = "/ws";
const DEFAULT_CONNECT_TIMEOUT_MS = 10_000;
const DEFAULT_RPC_TIMEOUT_MS = 10_000;
const DEFAULT_HTTP_TIMEOUT_MS = 15_000;
const DEFAULT_EXPECT_TIMEOUT_MS = 5_000;
const DEFAULT_POLL_INTERVAL_MS = 100;

const RPC_METHODS = {
  locatorClick: "automation.locator.click",
  locatorType: "automation.locator.type",
  locatorText: "automation.locator.text",
  locatorVisible: "automation.locator.visible",
  keyboardPress: "automation.keyboard.press",
  sessionSelect: "ctx.sessions.select",
};

function normalizeHttpUrl(value) {
  if (value === undefined || value === null || value === "") {
    return new URL(DEFAULT_HTTP_URL);
  }
  const trimmed = String(value).trim();
  if (!trimmed) {
    throw new Error("httpUrl must not be empty");
  }
  if (/^[a-z]+:\/\//i.test(trimmed)) {
    return new URL(trimmed);
  }
  return new URL(`http://${trimmed}`);
}

function normalizeWsUrl(value, httpUrl) {
  if (value !== undefined && value !== null && value !== "") {
    const trimmed = String(value).trim();
    if (!trimmed) {
      throw new Error("wsUrl must not be empty");
    }
    if (/^[a-z]+:\/\//i.test(trimmed)) {
      return new URL(trimmed);
    }
    return new URL(`ws://${trimmed}`);
  }
  const wsUrl = new URL(DEFAULT_WS_PATH, httpUrl);
  wsUrl.protocol = httpUrl.protocol === "https:" ? "wss:" : "ws:";
  return wsUrl;
}

function sleep(ms) {
  if (ms <= 0) {
    return Promise.resolve();
  }
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function normalizeTimeout(value, fallback) {
  if (value === undefined || value === null) {
    return fallback;
  }
  const number = Number(value);
  if (!Number.isFinite(number) || number < 0) {
    return fallback;
  }
  return Math.trunc(number);
}

function formatSelectorText(selector) {
  if (typeof selector === "string") {
    return selector;
  }
  if (!selector || typeof selector !== "object") {
    return String(selector);
  }
  const { kind, value, name } = selector;
  if (kind === "id") {
    return `#${value}`;
  }
  if (kind === "role") {
    if (name) {
      return `role=${value} name=${name}`;
    }
    return `role=${value}`;
  }
  if (kind === "text") {
    return `text=${value}`;
  }
  return `${kind}=${value}`;
}

function parseSelector(selector) {
  if (typeof selector === "string") {
    if (selector.startsWith("#") && selector.length > 1) {
      return { kind: "id", value: selector.slice(1) };
    }
    throw new Error(
      `Unsupported selector "${selector}". Use "#id" or page.getByRole()/getByText().`,
    );
  }
  if (!selector || typeof selector !== "object") {
    throw new Error("locator selector must be a string or selector object");
  }
  const { kind, value, name } = selector;
  if (typeof kind !== "string" || !kind) {
    throw new Error("selector.kind must be a string");
  }
  if (typeof value !== "string" || !value) {
    throw new Error("selector.value must be a string");
  }
  const normalized = { kind, value };
  if (name !== undefined && name !== null) {
    if (typeof name !== "string" || !name) {
      throw new Error("selector.name must be a string");
    }
    normalized.name = name;
  }
  return normalized;
}


async function callJson(baseUrl, path, options = {}) {
  const { method = "POST", body, timeoutMs = DEFAULT_HTTP_TIMEOUT_MS } = options;
  const url = new URL(path, baseUrl);
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
  const headers = { accept: "application/json" };
  if (body !== undefined) {
    headers["content-type"] = "application/json";
  }

  try {
    const response = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
    });
    const text = await response.text();
    let payload;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch (err) {
        throw new Error(`invalid JSON response: ${err.message}`);
      }
    }

    if (!response.ok) {
      throw new Error(`HTTP ${response.status} ${response.statusText}`);
    }
    if (!payload || payload.ok !== true) {
      throw new Error(payload?.error || "unknown error");
    }
    return payload.result;
  } catch (err) {
    throw new Error(`Request ${method} ${url} failed: ${err.message}`);
  } finally {
    clearTimeout(timeoutId);
  }
}

function openWebSocket(ws, timeoutMs) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) {
        return;
      }
      settled = true;
      try {
        ws.close();
      } catch (_) {
        // ignore
      }
      reject(new Error("WebSocket connect timed out"));
    }, timeoutMs);

    ws.addEventListener("open", () => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      resolve();
    });

    ws.addEventListener("error", (event) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      const message = event?.message ? String(event.message) : "WebSocket error";
      reject(new Error(message));
    });
  });
}

function closeWebSocket(ws, timeoutMs) {
  return new Promise((resolve) => {
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) {
        return;
      }
      settled = true;
      resolve();
    }, timeoutMs);

    ws.addEventListener(
      "close",
      () => {
        if (settled) {
          return;
        }
        settled = true;
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );

    try {
      ws.close();
    } catch (_) {
      clearTimeout(timer);
      resolve();
    }
  });
}

class JsonRpcClient {
  constructor(ws, options = {}) {
    this.ws = ws;
    this.pending = new Map();
    this.nextId = 1;
    this.defaultTimeoutMs = normalizeTimeout(
      options.timeoutMs,
      DEFAULT_RPC_TIMEOUT_MS,
    );
    this.closed = false;

    ws.addEventListener("message", (event) => {
      this.handleMessage(event);
    });
    ws.addEventListener("close", () => {
      this.handleClose();
    });
  }

  async call(method, params, options = {}) {
    if (this.closed) {
      throw new Error("WebSocket is closed");
    }
    const id = this.nextId;
    this.nextId += 1;
    const timeoutMs = normalizeTimeout(
      options.timeoutMs,
      this.defaultTimeoutMs,
    );
    const payload = { jsonrpc: "2.0", id, method, params };
    return new Promise((resolve, reject) => {
      const timeoutId = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`RPC ${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pending.set(id, { resolve, reject, timeoutId });
      try {
        this.ws.send(JSON.stringify(payload));
      } catch (err) {
        clearTimeout(timeoutId);
        this.pending.delete(id);
        reject(err);
      }
    });
  }

  async close() {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.handleClose();
    await closeWebSocket(this.ws, 1000);
  }

  handleMessage(event) {
    const raw =
      typeof event.data === "string"
        ? event.data
        : event.data?.toString?.() ?? "";
    if (!raw) {
      return;
    }
    let message;
    try {
      message = JSON.parse(raw);
    } catch (_) {
      return;
    }
    if (!message || message.id === undefined || message.id === null) {
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    clearTimeout(pending.timeoutId);
    if (message.error) {
      const errorMessage =
        typeof message.error === "string"
          ? message.error
          : message.error.message || JSON.stringify(message.error);
      pending.reject(new Error(errorMessage));
      return;
    }
    pending.resolve(message.result);
  }

  handleClose() {
    if (this.pending.size === 0) {
      return;
    }
    const error = new Error("WebSocket closed");
    for (const [id, pending] of this.pending.entries()) {
      clearTimeout(pending.timeoutId);
      pending.reject(error);
      this.pending.delete(id);
    }
  }
}

class Locator {
  constructor(rpc, selectorText) {
    this.rpc = rpc;
    this.selectorText = formatSelectorText(selectorText);
    this.selector = parseSelector(selectorText);
  }

  toString() {
    return `locator(${this.selectorText})`;
  }

  async click() {
    return this.rpc.call(RPC_METHODS.locatorClick, {
      selector: this.selector,
    });
  }

  async type(text) {
    if (text === undefined || text === null) {
      throw new Error("locator.type requires text");
    }
    return this.rpc.call(RPC_METHODS.locatorType, {
      selector: this.selector,
      text: String(text),
    });
  }

  async text() {
    const result = await this.rpc.call(RPC_METHODS.locatorText, {
      selector: this.selector,
    });
    if (typeof result === "string") {
      return result;
    }
    if (result && typeof result === "object") {
      if (typeof result.text === "string") {
        return result.text;
      }
      if (typeof result.value === "string") {
        return result.value;
      }
    }
    return String(result ?? "");
  }

  async isVisible() {
    const result = await this.rpc.call(RPC_METHODS.locatorVisible, {
      selector: this.selector,
    });
    if (typeof result === "boolean") {
      return result;
    }
    if (result && typeof result === "object") {
      if (typeof result.visible === "boolean") {
        return result.visible;
      }
      if (typeof result.value === "boolean") {
        return result.value;
      }
    }
    return Boolean(result);
  }
}

function createPage(rpc, httpUrl) {
  return {
    locator(selector) {
      return new Locator(rpc, selector);
    },
    getByRole(role, options = {}) {
      if (role === undefined || role === null || role === "") {
        throw new Error("getByRole requires a role");
      }
      const selector = { kind: "role", value: String(role) };
      if (options.name !== undefined && options.name !== null) {
        selector.name = String(options.name);
      }
      return new Locator(rpc, selector);
    },
    getByText(text) {
      if (text === undefined || text === null || text === "") {
        throw new Error("getByText requires text");
      }
      return new Locator(rpc, { kind: "text", value: String(text) });
    },
    async screenshot({ path, name } = {}) {
      return callJson(httpUrl, "/screenshot", {
        method: "POST",
        body: { path, name },
      });
    },
    keyboard: {
      async press(key) {
        if (!key) {
          throw new Error("keyboard.press requires a key");
        }
        return rpc.call(RPC_METHODS.keyboardPress, { key: String(key) });
      },
    },
    async clickSession(index) {
      if (!Number.isInteger(index) || index < 0) {
        throw new Error("clickSession requires a non-negative integer index");
      }
      return rpc.call(RPC_METHODS.sessionSelect, { index });
    },
    rpc(method, params, options) {
      return rpc.call(method, params, options);
    },
  };
}

function formatExpected(expected) {
  if (expected instanceof RegExp) {
    return expected.toString();
  }
  return JSON.stringify(expected);
}

function matchExpected(actual, expected) {
  if (expected instanceof RegExp) {
    return expected.test(actual);
  }
  return actual === String(expected);
}

async function pollUntil(check, options) {
  const {
    timeoutMs,
    intervalMs,
    onTimeout,
    onErrorMessage = "Expectation failed",
  } = options;
  const start = Date.now();
  let last;
  while (Date.now() - start < timeoutMs) {
    try {
      const result = await check();
      last = result;
      if (result && result.ok) {
        return result.value;
      }
    } catch (err) {
      last = { error: err };
    }
    await sleep(intervalMs);
  }
  if (typeof onTimeout === "function") {
    throw new Error(onTimeout(last));
  }
  if (last?.error) {
    throw new Error(`${onErrorMessage}: ${last.error.message}`);
  }
  throw new Error(onErrorMessage);
}

export function expect(locator, options = {}) {
  const defaultTimeoutMs = normalizeTimeout(
    options.timeoutMs ?? options.timeout,
    DEFAULT_EXPECT_TIMEOUT_MS,
  );
  const defaultIntervalMs = normalizeTimeout(
    options.intervalMs ?? options.pollIntervalMs,
    DEFAULT_POLL_INTERVAL_MS,
  );

  return {
    async toHaveText(expected, assertOptions = {}) {
      const timeoutMs = normalizeTimeout(
        assertOptions.timeoutMs ?? assertOptions.timeout,
        defaultTimeoutMs,
      );
      const intervalMs = normalizeTimeout(
        assertOptions.intervalMs ?? assertOptions.pollIntervalMs,
        defaultIntervalMs,
      );
      return pollUntil(
        async () => {
          const actual = await locator.text();
          const ok = matchExpected(actual, expected);
          return { ok, value: actual, actual };
        },
        {
          timeoutMs,
          intervalMs,
          onTimeout: (last) => {
            const actual = last?.actual ?? "<no value>";
            return `Expected ${locator} to have text ${formatExpected(
              expected,
            )} but got ${JSON.stringify(actual)}`;
          },
        },
      );
    },
    async toBeVisible(assertOptions = {}) {
      const timeoutMs = normalizeTimeout(
        assertOptions.timeoutMs ?? assertOptions.timeout,
        defaultTimeoutMs,
      );
      const intervalMs = normalizeTimeout(
        assertOptions.intervalMs ?? assertOptions.pollIntervalMs,
        defaultIntervalMs,
      );
      return pollUntil(
        async () => {
          const actual = await locator.isVisible();
          return { ok: Boolean(actual), value: actual, actual };
        },
        {
          timeoutMs,
          intervalMs,
          onTimeout: () =>
            `Expected ${locator} to be visible but it stayed hidden`,
        },
      );
    },
  };
}

export async function connect(options = {}) {
  const httpUrl = normalizeHttpUrl(options.httpUrl);
  const wsUrl = normalizeWsUrl(options.wsUrl, httpUrl);
  const ws = new WebSocket(wsUrl.href);
  await openWebSocket(
    ws,
    normalizeTimeout(options.connectTimeoutMs, DEFAULT_CONNECT_TIMEOUT_MS),
  );
  const rpc = new JsonRpcClient(ws, {
    timeoutMs: options.rpcTimeoutMs,
  });
  const page = createPage(rpc, httpUrl);
  return {
    page,
    rpc: rpc.call.bind(rpc),
    close: () => rpc.close(),
    httpUrl: httpUrl.href,
    wsUrl: wsUrl.href,
  };
}
