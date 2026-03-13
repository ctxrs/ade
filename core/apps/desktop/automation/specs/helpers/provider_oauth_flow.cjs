const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const { daemonJson } = require("./daemon.cjs");

const DEFAULT_POLL_MS = 750;
const DEFAULT_TIMEOUT_MS = 5 * 60_000;
const DEFAULT_URL_FALLBACK_GRACE_MS = 1200;
const OPENAI_AUTH_TEXT_SNIPPET_LIMIT = 4000;
const TOTP_PERIOD_SECONDS = 30;
const TOTP_DIGITS = 6;
const BASE32_ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

const PROVIDER_OAUTH_DESCRIPTORS = Object.freeze({
  codex: {
    providerId: "codex",
    startPath: "/api/providers/codex/accounts/login/start",
    statusPath: (loginId) => `/api/providers/codex/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/codex/accounts",
    loginIdField: "account_id",
  },
  cursor: {
    providerId: "cursor",
    startPath: "/api/providers/cursor/accounts/login/start",
    statusPath: (loginId) => `/api/providers/cursor/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/cursor/accounts",
    loginIdField: "login_id",
  },
  "claude-crp": {
    providerId: "claude-crp",
    startPath: "/api/providers/claude-crp/accounts/login/start",
    statusPath: (loginId) => `/api/providers/claude-crp/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/claude-crp/accounts",
    loginIdField: "login_id",
  },
  gemini: {
    providerId: "gemini",
    startPath: "/api/providers/gemini/accounts/login/start",
    statusPath: (loginId) => `/api/providers/gemini/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/gemini/accounts",
    loginIdField: "login_id",
  },
  qwen: {
    providerId: "qwen",
    startPath: "/api/providers/qwen/accounts/login/start",
    statusPath: (loginId) => `/api/providers/qwen/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/qwen/accounts",
    loginIdField: "login_id",
  },
  amp: {
    providerId: "amp",
    startPath: "/api/providers/amp/accounts/login/start",
    statusPath: (loginId) => `/api/providers/amp/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/amp/accounts",
    loginIdField: "login_id",
  },
  mistral: {
    providerId: "mistral",
    startPath: "/api/providers/mistral/accounts/login/start",
    statusPath: (loginId) => `/api/providers/mistral/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/mistral/accounts",
    loginIdField: "login_id",
  },
});

const PROBE_GLOBAL_KEY = "__ctxProviderOauthOpenExternalProbe";

const nowIso = () => new Date().toISOString();

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const asRecord = (value) => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value;
};

const asArray = (value) => (Array.isArray(value) ? value : []);

const readString = (value) => (typeof value === "string" ? value.trim() : "");

const normalizeStatus = (value) => readString(value).toLowerCase();

const isTerminalStatus = (value) => {
  const normalized = normalizeStatus(value);
  return normalized === "success" || normalized === "failed" || normalized === "timeout";
};

const sanitizeAuthUrl = (raw) => {
  const text = readString(raw);
  if (!text) return null;

  try {
    const parsed = new URL(text);
    return {
      scheme: parsed.protocol.replace(/:$/, ""),
      host: parsed.host,
      path: parsed.pathname || "/",
      redacted: `${parsed.protocol}//${parsed.host}${parsed.pathname || "/"}`,
    };
  } catch {
    const noFragment = text.split("#", 1)[0];
    const noQuery = noFragment.split("?", 1)[0].trim();
    return noQuery
      ? {
          scheme: "",
          host: "",
          path: noQuery,
          redacted: noQuery,
        }
      : null;
  }
};

const normalizeBase32Secret = (raw) =>
  readString(raw)
    .toUpperCase()
    .replace(/[\s=-]+/g, "");

const decodeBase32Secret = (raw) => {
  const normalized = normalizeBase32Secret(raw);
  if (!normalized) {
    throw new Error("TOTP secret is required");
  }

  let bits = 0;
  let value = 0;
  const bytes = [];
  for (const ch of normalized) {
    const index = BASE32_ALPHABET.indexOf(ch);
    if (index < 0) {
      throw new Error(`invalid base32 character '${ch}' in TOTP secret`);
    }
    value = (value << 5) | index;
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      bytes.push((value >>> bits) & 0xff);
    }
  }
  return Buffer.from(bytes);
};

const createTotpCode = (secret, { timestampMs = Date.now(), periodSeconds = TOTP_PERIOD_SECONDS, digits = TOTP_DIGITS } = {}) => {
  const key = decodeBase32Secret(secret);
  const counter = Math.floor(timestampMs / 1000 / periodSeconds);
  const counterBuffer = Buffer.alloc(8);
  counterBuffer.writeUInt32BE(Math.floor(counter / 0x100000000), 0);
  counterBuffer.writeUInt32BE(counter >>> 0, 4);
  const digest = crypto.createHmac("sha1", key).update(counterBuffer).digest();
  const offset = digest[digest.length - 1] & 0x0f;
  const binary = (
    ((digest[offset] & 0x7f) << 24)
    | ((digest[offset + 1] & 0xff) << 16)
    | ((digest[offset + 2] & 0xff) << 8)
    | (digest[offset + 3] & 0xff)
  );
  return String(binary % (10 ** digits)).padStart(digits, "0");
};

const hasBrowserUrl = () => Boolean(global.browser && typeof global.browser.url === "function");
const hasBrowserKeys = () => Boolean(global.browser && typeof global.browser.keys === "function");
const hasBrowserQuery = () => Boolean(global.browser && typeof global.browser.$$ === "function");
const hasBrowserExecute = () => Boolean(global.browser && typeof global.browser.execute === "function");
let browserQuerySupported = true;

const normalizeUiText = (value) => String(value || "").replace(/\s+/g, " ").trim().toLowerCase();

const AUTH_INPUT_SELECTOR_GROUPS = Object.freeze({
  email: [
    "input[type='email']",
    "input[name='email']",
    "input[autocomplete='email']",
    "input[name*='email' i]",
    "input[aria-label*='email' i]",
    "input[placeholder*='email' i]",
  ],
  password: [
    "input[type='password']",
    "input[autocomplete='current-password']",
    "input[autocomplete='password']",
    "input[name*='password' i]",
    "input[aria-label*='password' i]",
  ],
  otp: [
    "input[autocomplete='one-time-code']",
    "input[inputmode='numeric']",
    "input[name*='otp' i]",
    "input[name*='code' i]",
    "input[aria-label*='authenticator' i]",
    "input[aria-label*='verification' i]",
    "input[aria-label*='code' i]",
  ],
});

const AUTH_BUTTON_SELECTORS = Object.freeze([
  "button[type='submit']",
  "button",
  "input[type='submit']",
  "[role='button']",
  "a",
]);

const OPENAI_EMAIL_CHALLENGE_PATTERNS = Object.freeze([
  /check your inbox/i,
  /check your email/i,
  /enter (?:the )?code we sent/i,
  /we sent (?:a )?code to/i,
  /use the code from your email/i,
  /email verification code/i,
]);

const OPENAI_AUTHENTICATOR_PATTERNS = Object.freeze([
  /authenticator/i,
  /authentication app/i,
  /auth app/i,
  /two-factor/i,
  /\b2fa\b/i,
]);

const OPENAI_GENERIC_CODE_CHALLENGE_PATTERNS = Object.freeze([
  /verification code/i,
  /enter (?:the )?code/i,
  /\b6-digit code\b/i,
  /confirm it'?s you/i,
]);

const elementKey = (element) =>
  readString(element?.elementId || element?.ELEMENT || element?.["element-6066-11e4-a52e-4f735466cecf"]);

const collectVisibleBrowserElements = async (selectors) => {
  if (!browserQuerySupported || !hasBrowserQuery()) return [];

  const visible = [];
  const seen = new Set();
  const selectorList = Array.isArray(selectors) ? selectors : [selectors];
  for (const selector of selectorList) {
    let candidates = [];
    try {
      candidates = await browser.$$(selector);
    } catch {
      browserQuerySupported = false;
      return [];
    }
    for (const candidate of candidates) {
      try {
        if (typeof candidate?.isDisplayed === "function" && !(await candidate.isDisplayed())) {
          continue;
        }
        const key = elementKey(candidate);
        if (key && seen.has(key)) continue;
        if (key) seen.add(key);
        visible.push(candidate);
      } catch {
        // Ignore stale or unsupported element handles.
      }
    }
    if (visible.length > 0) {
      return visible;
    }
  }
  return visible;
};

const readBrowserElementText = async (element) => {
  const values = [];
  try {
    if (typeof element?.getText === "function") {
      values.push(await element.getText());
    }
  } catch {
    // ignore
  }
  for (const attribute of ["value", "aria-label", "title", "name"]) {
    try {
      if (typeof element?.getAttribute === "function") {
        values.push(await element.getAttribute(attribute));
      }
    } catch {
      // ignore
    }
  }
  return normalizeUiText(values.filter(Boolean).join(" "));
};

const focusBrowserElement = async (element) => {
  if (!element) return;
  try {
    if (typeof element.scrollIntoView === "function") {
      await element.scrollIntoView();
    }
  } catch {
    // ignore
  }
  if (typeof element.click === "function") {
    await element.click();
  }
};

const setBrowserElementValue = async (element, value) => {
  if (!element) return false;
  await focusBrowserElement(element);
  if (typeof element.clearValue === "function") {
    try {
      await element.clearValue();
    } catch {
      // Some password/code fields do not support clearValue consistently.
    }
  }
  if (typeof element.setValue === "function") {
    await element.setValue(String(value || ""));
    return true;
  }
  if (hasBrowserKeys()) {
    await browser.keys(String(value || ""));
    return true;
  }
  return false;
};

const navigateBrowserToUrl = async (href) => {
  const target = readString(href);
  if (!target) {
    throw new Error("auth URL is required to navigate browser");
  }
  if (hasBrowserUrl()) {
    await browser.url(target);
    return;
  }
  if (!hasBrowserExecute()) {
    throw new Error("browser.url or browser.execute is required to navigate auth URL");
  }
  await browser.execute((authUrl) => {
    window.location.assign(authUrl);
  }, target);
};

const readVisibleDomState = async () => {
  if (!hasBrowserExecute()) {
    throw new Error("browser.execute is required to inspect auth page state");
  }
  return await browser.execute((textLimit) => {
    const normalize = (value) => String(value || "").replace(/\s+/g, " ").trim();
    const isVisible = (element) => {
      if (!(element instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(element);
      if (!style || style.visibility === "hidden" || style.display === "none") return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const labelTextFor = (element) => {
      const texts = [];
      if (element instanceof HTMLElement) {
        if (typeof element.getAttribute === "function") {
          texts.push(element.getAttribute("aria-label"));
          const labelledBy = element.getAttribute("aria-labelledby");
          if (labelledBy) {
            for (const id of labelledBy.split(/\s+/g)) {
              const labelEl = document.getElementById(id);
              if (labelEl) texts.push(labelEl.textContent);
            }
          }
          const placeholder = element.getAttribute("placeholder");
          if (placeholder) texts.push(placeholder);
        }
        if ("labels" in element && Array.isArray(Array.from(element.labels || []))) {
          for (const label of Array.from(element.labels || [])) {
            texts.push(label.textContent);
          }
        }
        const parentLabel = element.closest("label");
        if (parentLabel) texts.push(parentLabel.textContent);
      }
      return normalize(texts.filter(Boolean).join(" "));
    };
    const inputMetadata = Array.from(document.querySelectorAll("input, textarea"))
      .filter((element) => isVisible(element))
      .map((element) => ({
        tag: element.tagName.toLowerCase(),
        type: normalize(element.getAttribute("type") || "").toLowerCase(),
        name: normalize(element.getAttribute("name") || "").toLowerCase(),
        autocomplete: normalize(element.getAttribute("autocomplete") || "").toLowerCase(),
        inputmode: normalize(element.getAttribute("inputmode") || "").toLowerCase(),
        label: labelTextFor(element),
        maxLength: Number(element.maxLength || 0),
      }));
    const buttonTexts = Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], a"))
      .filter((element) => isVisible(element))
      .map((element) => normalize(element.textContent || element.getAttribute("value") || element.getAttribute("aria-label")))
      .filter(Boolean);
    const bodyText = normalize(document.body?.innerText || "").slice(0, Math.max(0, Number(textLimit) || 0));
    return {
      url: String(window.location.href || ""),
      title: normalize(document.title || ""),
      bodyText,
      inputs: inputMetadata,
      buttons: buttonTexts,
    };
  }, OPENAI_AUTH_TEXT_SNIPPET_LIMIT);
};

const stateMentions = (state, pattern) => {
  const regex = pattern instanceof RegExp ? pattern : new RegExp(String(pattern || ""), "i");
  return regex.test(readString(state?.title))
    || regex.test(readString(state?.bodyText))
    || (Array.isArray(state?.buttons) && state.buttons.some((value) => regex.test(readString(value))));
};

const callbackReached = (state, expectedCallbackUrl) => {
  const current = sanitizeAuthUrl(state?.url);
  const expected = sanitizeAuthUrl(expectedCallbackUrl);
  if (!current || !expected) return false;
  return current.scheme === expected.scheme && current.host === expected.host && current.path === expected.path;
};

const summarizeAuthState = (state) => ({
  url: sanitizeAuthUrl(state?.url),
  title: readString(state?.title),
  inputs: asArray(state?.inputs).map((entry) => ({
    tag: readString(entry?.tag),
    type: readString(entry?.type),
    name: readString(entry?.name),
    autocomplete: readString(entry?.autocomplete),
    inputmode: readString(entry?.inputmode),
    label: readString(entry?.label),
    maxLength: Number(entry?.maxLength || 0),
  })),
  buttons: asArray(state?.buttons).map((entry) => readString(entry)).filter(Boolean).slice(0, 12),
});

const collectAuthStateText = (state) => normalizeUiText([
  readString(state?.title),
  readString(state?.bodyText),
  ...asArray(state?.buttons).map((entry) => readString(entry)),
  ...asArray(state?.inputs).map((entry) => [
    readString(entry?.type),
    readString(entry?.name),
    readString(entry?.autocomplete),
    readString(entry?.inputmode),
    readString(entry?.label),
  ].join(" ")),
].join(" "));

const matchStatePatterns = (text, patterns) => patterns.filter((pattern) => pattern.test(text));

const createProviderOAuthBlockedError = ({ stage, blockedReason, message, challenge = null }) => {
  const error = createProviderOAuthError(stage, message, challenge ? { challenge } : {});
  error.name = "ProviderOAuthBlockedError";
  error.blocked = true;
  error.blockedReason = readString(blockedReason) || "blocked";
  return error;
};

const isProviderOAuthBlockedError = (error) => Boolean(error?.blocked === true || error?.name === "ProviderOAuthBlockedError");

const codexExternalBrowserAutomationUnsupported = () => createProviderOAuthBlockedError({
  stage: "drive_codex_openai_login",
  blockedReason: "external_browser_automation_unsupported",
  message: "Codex OAuth currently opens in the system browser, but this WDIO harness can only drive the ctx Tauri webview. Use manual validation for the real localhost callback path.",
});

const classifyCodexOpenAiChallenge = ({ state, hasOtpInput = false } = {}) => {
  const text = collectAuthStateText(state);
  if (!text) return null;

  if (/captcha|verify you are human|just a moment/i.test(text)) {
    return {
      kind: "interactive_challenge_required",
      message: "OpenAI auth page requires an interactive anti-bot or captcha challenge",
      state: summarizeAuthState(state),
      matched_signals: ["captcha_or_human_verification"],
    };
  }

  const emailMatches = matchStatePatterns(text, OPENAI_EMAIL_CHALLENGE_PATTERNS);
  if (emailMatches.length > 0) {
    return {
      kind: "email_challenge_required",
      message: "OpenAI auth requires an email verification code challenge that automation cannot satisfy deterministically",
      state: summarizeAuthState(state),
      matched_signals: emailMatches.map((pattern) => String(pattern)),
    };
  }

  if (!hasOtpInput) {
    return null;
  }

  const authenticatorMatches = matchStatePatterns(text, OPENAI_AUTHENTICATOR_PATTERNS);
  if (authenticatorMatches.length > 0) {
    return null;
  }

  const genericMatches = matchStatePatterns(text, OPENAI_GENERIC_CODE_CHALLENGE_PATTERNS);
  if (genericMatches.length === 0) {
    return null;
  }

  return {
    kind: "verification_code_challenge_required",
    message: "OpenAI auth requires an out-of-band verification code challenge that automation cannot satisfy deterministically",
    state: summarizeAuthState(state),
    matched_signals: genericMatches.map((pattern) => String(pattern)),
  };
};

const fillBrowserAuthField = async (fieldKind, value) => {
  const selectors = AUTH_INPUT_SELECTOR_GROUPS[fieldKind] || [];
  const rawValue = String(value || "");
  if (selectors.length > 0) {
    const elements = await collectVisibleBrowserElements(selectors);
    if (elements.length > 0) {
      if (fieldKind === "otp") {
        const digitInputs = [];
        for (const element of elements) {
          try {
            const maxLength = Number.parseInt(String(await element.getAttribute("maxlength") || ""), 10);
            if (maxLength === 1) {
              digitInputs.push(element);
            }
          } catch {
            // ignore attribute read failures
          }
        }
        if (digitInputs.length >= rawValue.length) {
          for (let index = 0; index < rawValue.length; index += 1) {
            const ok = await setBrowserElementValue(digitInputs[index], rawValue.charAt(index));
            if (!ok) {
              return { ok: false, reason: "webdriver split otp entry unsupported" };
            }
          }
          return { ok: true, filled: rawValue.length, mode: "webdriver-split" };
        }
      }

      const ok = await setBrowserElementValue(elements[0], rawValue);
      if (ok) {
        return { ok: true, filled: 1, mode: "webdriver" };
      }
    }
  }

  if (!hasBrowserExecute()) {
    throw new Error("browser element queries or browser.execute are required to fill auth fields");
  }
  return await browser.execute((kind, rawValue) => {
    const normalize = (entry) => String(entry || "").replace(/\s+/g, " ").trim().toLowerCase();
    const isVisible = (element) => {
      if (!(element instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(element);
      if (!style || style.visibility === "hidden" || style.display === "none") return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const typeText = (element, nextValue) => {
      const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      const setter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
      element.focus();
      if (typeof element.select === "function") {
        try {
          element.select();
        } catch {
          // ignore
        }
      }
      if (setter) {
        setter.call(element, "");
      } else {
        element.value = "";
      }
      try {
        element.dispatchEvent(new InputEvent("input", {
          bubbles: true,
          inputType: "deleteContentBackward",
          data: null,
        }));
      } catch {
        element.dispatchEvent(new Event("input", { bubbles: true }));
      }
      for (const ch of String(nextValue || "")) {
        element.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: ch }));
        element.dispatchEvent(new KeyboardEvent("keypress", { bubbles: true, key: ch }));
        const currentValue = `${element.value || ""}${ch}`;
        if (setter) {
          setter.call(element, currentValue);
        } else {
          element.value = currentValue;
        }
        try {
          element.dispatchEvent(new InputEvent("beforeinput", {
            bubbles: true,
            inputType: "insertText",
            data: ch,
          }));
          element.dispatchEvent(new InputEvent("input", {
            bubbles: true,
            inputType: "insertText",
            data: ch,
          }));
        } catch {
          element.dispatchEvent(new Event("input", { bubbles: true }));
        }
        element.dispatchEvent(new KeyboardEvent("keyup", { bubbles: true, key: ch }));
      }
      element.dispatchEvent(new Event("change", { bubbles: true }));
      if (typeof element.blur === "function") {
        element.blur();
      }
    };
    const getLabel = (element) => {
      const chunks = [];
      if (typeof element.getAttribute === "function") {
        chunks.push(element.getAttribute("aria-label"));
        chunks.push(element.getAttribute("placeholder"));
      }
      if ("labels" in element) {
        for (const label of Array.from(element.labels || [])) {
          chunks.push(label.textContent);
        }
      }
      const parentLabel = element.closest("label");
      if (parentLabel) chunks.push(parentLabel.textContent);
      return normalize(chunks.filter(Boolean).join(" "));
    };
    const candidates = Array.from(document.querySelectorAll("input, textarea")).filter((element) => isVisible(element));
    const matches = candidates.filter((element) => {
      const type = normalize(element.getAttribute("type"));
      const name = normalize(element.getAttribute("name"));
      const autocomplete = normalize(element.getAttribute("autocomplete"));
      const inputmode = normalize(element.getAttribute("inputmode"));
      const label = getLabel(element);
      if (kind === "email") {
        return type === "email" || autocomplete === "email" || name.includes("email") || label.includes("email");
      }
      if (kind === "password") {
        return type === "password" || autocomplete.includes("password") || label.includes("password");
      }
      if (kind === "otp") {
        return autocomplete === "one-time-code"
          || inputmode === "numeric"
          || name.includes("otp")
          || name.includes("code")
          || label.includes("code")
          || label.includes("authenticator")
          || label.includes("verification");
      }
      return false;
    });

    if (kind === "otp") {
      const digitInputs = matches.filter((element) => Number(element.maxLength || 0) === 1);
      if (digitInputs.length >= String(rawValue).length) {
        for (let index = 0; index < String(rawValue).length; index += 1) {
          const element = digitInputs[index];
          const value = String(rawValue).charAt(index);
          typeText(element, value);
        }
        return { ok: true, filled: digitInputs.length, mode: "split" };
      }
    }

    const target = matches[0] || null;
    if (!target) return { ok: false, reason: `no visible ${kind} input` };
    typeText(target, rawValue);
    return { ok: true, filled: 1, mode: "single" };
  }, fieldKind, String(value || ""));
};

const submitVisibleAuthStep = async (preferredButtonTexts = []) => {
  const wants = Array.isArray(preferredButtonTexts)
    ? preferredButtonTexts.map((entry) => normalizeUiText(entry)).filter(Boolean)
    : [];
  const buttons = await collectVisibleBrowserElements(AUTH_BUTTON_SELECTORS);
  if (buttons.length > 0) {
    const entries = [];
    for (const button of buttons) {
      entries.push({
        button,
        text: await readBrowserElementText(button),
      });
    }
    const matched = entries.find((entry) => wants.some((want) => entry.text.includes(want)))
      || entries.find((entry) => entry.text.includes("continue"))
      || entries.find((entry) => entry.text.includes("next"))
      || entries.find((entry) => entry.text.includes("verify"))
      || entries.find((entry) => entry.text.includes("log in"))
      || entries.find((entry) => entry.text.includes("login"))
      || null;
    if (matched) {
      await focusBrowserElement(matched.button);
      return { ok: true, strategy: "webdriver-button", text: matched.text };
    }
  }

  if (!hasBrowserExecute()) {
    if (hasBrowserKeys()) {
      await browser.keys("Enter");
      return { ok: true, strategy: "webdriver-keys" };
    }
    throw new Error("browser element queries, browser.keys, or browser.execute are required to submit auth steps");
  }
  const result = await browser.execute((preferredTexts) => {
    const normalize = (value) => String(value || "").replace(/\s+/g, " ").trim().toLowerCase();
    const wants = Array.isArray(preferredTexts) ? preferredTexts.map((entry) => normalize(entry)).filter(Boolean) : [];
    const isVisible = (element) => {
      if (!(element instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(element);
      if (!style || style.visibility === "hidden" || style.display === "none") return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };

    const candidates = Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], a"))
      .filter((element) => isVisible(element))
      .map((element) => ({
        element,
        text: normalize(element.textContent || element.getAttribute("value") || element.getAttribute("aria-label")),
      }));
    const matched = candidates.find((entry) => wants.some((want) => entry.text.includes(want)))
      || candidates.find((entry) => entry.text.includes("continue"))
      || candidates.find((entry) => entry.text.includes("next"))
      || candidates.find((entry) => entry.text.includes("verify"))
      || candidates.find((entry) => entry.text.includes("log in"))
      || candidates.find((entry) => entry.text.includes("login"))
      || null;
    if (!matched) {
      const focused = document.activeElement;
      if (focused && focused instanceof HTMLElement) {
        const form = focused.closest("form");
        if (form && typeof form.requestSubmit === "function") {
          form.requestSubmit();
          return { ok: true, strategy: "form" };
        }
      }
      return { ok: false, strategy: "none" };
    }
    if (matched.element instanceof HTMLElement) {
      matched.element.focus();
      for (const type of ["mousedown", "mouseup", "click"]) {
        matched.element.dispatchEvent(new MouseEvent(type, {
          bubbles: true,
          cancelable: true,
          view: window,
        }));
      }
    } else {
      matched.element.click();
    }
    return { ok: true, strategy: "button", text: matched.text };
  }, preferredButtonTexts);

  if (!result?.ok && hasBrowserKeys()) {
    await browser.keys("Enter");
    return { ok: true, strategy: "keys" };
  }
  return result;
};

const startCodexDesktopRelay = async ({ loginId, callbackUrl, completionToken }) => {
  if (!hasBrowserExecute()) {
    throw new Error("browser.execute is required to start the codex desktop relay");
  }
  const result = await browser.execute(async (req) => {
    const invoke = window.__TAURI__?.core?.invoke;
    if (!invoke) return { ok: false, error: "Tauri invoke not available" };
    try {
      const accepted = await invoke("desktop_start_codex_login_relay", { req });
      return { ok: Boolean(accepted) };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  }, {
    login_id: loginId,
    callback_url: callbackUrl,
    completion_token: completionToken,
  });
  if (!result || result.ok !== true) {
    throw new Error(readString(result?.error) || "failed to start codex desktop relay");
  }
};

const driveCodexOpenAiLoginWithCredentials = async ({
  authUrl,
} = {}) => {
  void authUrl;
  throw codexExternalBrowserAutomationUnsupported();
};

const secretKeyPattern = /(completion_token|callback_code|api_?key|secret|password|token|oauth_creds_json|google_accounts_json|credentials_json)/i;

const redactPayload = (value, key = "") => {
  if (value === null || typeof value === "undefined") return value;

  const keyName = readString(key);
  if (typeof value === "string") {
    if (/(auth_url|callback_url|expected_callback_url)/i.test(keyName)) {
      return sanitizeAuthUrl(value);
    }
    if (secretKeyPattern.test(keyName)) {
      return value.trim() ? "[redacted]" : "";
    }
    return value;
  }

  if (typeof value === "number" || typeof value === "boolean") return value;

  if (Array.isArray(value)) {
    return value.map((entry) => redactPayload(entry, keyName));
  }

  if (typeof value === "object") {
    const output = {};
    for (const [entryKey, entryValue] of Object.entries(asRecord(value))) {
      output[entryKey] = redactPayload(entryValue, entryKey);
    }
    return output;
  }

  return String(value);
};

const getProviderDescriptor = (providerId) => {
  const descriptor = PROVIDER_OAUTH_DESCRIPTORS[providerId];
  if (descriptor) return descriptor;
  throw new Error(
    `unsupported provider oauth flow: ${providerId}; expected one of ${Object.keys(PROVIDER_OAUTH_DESCRIPTORS).join(", ")}`,
  );
};

const normalizeLoginStartPayload = (descriptor, payload) => {
  const row = asRecord(payload);
  const loginId = readString(
    row[descriptor.loginIdField],
  ) || readString(row.login_id) || readString(row.account_id);

  return {
    providerId: descriptor.providerId,
    loginId,
    authUrl: readString(row.auth_url),
    expectedCallbackUrl: readString(row.expected_callback_url),
    completionToken: readString(row.completion_token),
    payload: row,
    redactedPayload: redactPayload(row),
  };
};

const normalizeLoginStatusPayload = (descriptor, payload) => {
  const row = asRecord(payload);
  const status = normalizeStatus(row.status);
  return {
    providerId: descriptor.providerId,
    loginId: readString(row.login_id) || readString(row.account_id),
    accountId: readString(row.account_id),
    status,
    authUrl: readString(row.auth_url),
    expectedCallbackUrl: readString(row.expected_callback_url),
    completionTokenPresent: readString(row.completion_token).length > 0,
    error: readString(row.error),
    payload: row,
    redactedPayload: redactPayload(row),
  };
};

const normalizeAccountsPayload = (descriptor, payload) => {
  const row = asRecord(payload);
  const accounts = asArray(row.accounts).map((entry) => asRecord(entry));
  const activeAccountId = readString(row.active_account_id);
  return {
    providerId: descriptor.providerId,
    activeAccountId,
    accounts,
    accountIds: accounts.map((entry) => readString(entry.id)).filter(Boolean),
    redactedPayload: redactPayload(row),
  };
};

const createProviderOAuthError = (stage, message, extras = {}) => {
  const error = new Error(message);
  error.name = "ProviderOAuthFlowError";
  error.stage = stage;
  Object.assign(error, extras);
  return error;
};

const chooseObservedAuthUrl = ({ probeEvents, fallbackAuthUrl }) => {
  const sanitizedFallback = sanitizeAuthUrl(fallbackAuthUrl);
  if (!probeEvents.length) {
    return sanitizedFallback
      ? {
          source: "login_status",
          authUrl: fallbackAuthUrl,
          sanitizedAuthUrl: sanitizedFallback,
        }
      : null;
  }

  if (sanitizedFallback) {
    const matched = [...probeEvents].reverse().find((event) => {
      const candidate = sanitizeAuthUrl(event.href);
      return candidate
        && candidate.host === sanitizedFallback.host
        && candidate.path === sanitizedFallback.path;
    });
    if (matched) {
      return {
        source: "desktop_open_external",
        authUrl: matched.href,
        sanitizedAuthUrl: sanitizeAuthUrl(matched.href),
      };
    }
  }

  if (probeEvents.length === 1) {
    return {
      source: "desktop_open_external",
      authUrl: probeEvents[0].href,
      sanitizedAuthUrl: sanitizeAuthUrl(probeEvents[0].href),
    };
  }

  return sanitizedFallback
    ? {
        source: "login_status",
        authUrl: fallbackAuthUrl,
        sanitizedAuthUrl: sanitizedFallback,
      }
    : null;
};

const installDesktopOpenExternalProbe = async () => {
  if (!hasBrowserExecute()) return { installed: false, reason: "browser.execute unavailable" };

  return await browser.execute((probeKey) => {
    const root = window;
    const state = root[probeKey] || {
      events: [],
      installed_at_ms: Date.now(),
      wrapped_paths: [],
    };
    root[probeKey] = state;

    const wrapInvoke = (holder, pathLabel) => {
      if (!holder || typeof holder.invoke !== "function") return false;
      if (holder.invoke.__ctxOauthProbeWrapped) return true;
      const originalInvoke = holder.invoke;
      const wrappedInvoke = async function wrappedInvoke(command, ...rest) {
        try {
          if (typeof command === "string" && command.includes("plugin:shell|open")) {
            const args = rest[0];
            const href =
              typeof args?.path === "string" ? args.path
                : typeof args?.url === "string" ? args.url
                  : typeof args?.href === "string" ? args.href
                    : "";
            state.events.push({
              at_ms: Date.now(),
              command,
              source: pathLabel,
              href,
            });
          }
        } catch {
          // ignore probe failures
        }
        return await originalInvoke.apply(this, [command, ...rest]);
      };
      wrappedInvoke.__ctxOauthProbeWrapped = true;
      holder.invoke = wrappedInvoke;
      state.wrapped_paths.push(pathLabel);
      return true;
    };

    const installedCore = wrapInvoke(root.__TAURI__?.core, "__TAURI__.core");
    const installedInternals = wrapInvoke(root.__TAURI_INTERNALS__, "__TAURI_INTERNALS__");
    const installed = installedCore || installedInternals;

    return {
      installed,
      wrapped_paths: Array.from(new Set(state.wrapped_paths)),
      event_count: Array.isArray(state.events) ? state.events.length : 0,
    };
  }, PROBE_GLOBAL_KEY);
};

const resetDesktopOpenExternalProbe = async () => {
  if (!hasBrowserExecute()) return [];
  return await browser.execute((probeKey) => {
    const root = window;
    const state = root[probeKey];
    if (!state || !Array.isArray(state.events)) return [];
    const previous = state.events.slice();
    state.events = [];
    return previous;
  }, PROBE_GLOBAL_KEY);
};

const readDesktopOpenExternalProbe = async () => {
  if (!hasBrowserExecute()) return [];
  return await browser.execute((probeKey) => {
    const root = window;
    const state = root[probeKey];
    if (!state || !Array.isArray(state.events)) return [];
    return state.events.slice();
  }, PROBE_GLOBAL_KEY);
};

const openExternalUrlViaDesktop = async (href) => {
  const target = readString(href);
  if (!target) {
    throw new Error("auth URL is required to open an external browser");
  }
  if (!hasBrowserExecute()) {
    throw new Error("browser.execute is required to open an external browser");
  }
  const result = await browser.execute(async (authUrl, probeKey) => {
    try {
      const root = window;
      const state = root[probeKey] || {
        events: [],
        installed_at_ms: Date.now(),
        wrapped_paths: [],
      };
      root[probeKey] = state;

      const wrapInvoke = (holder, pathLabel) => {
        if (!holder || typeof holder.invoke !== "function") return false;
        if (holder.invoke.__ctxOauthProbeWrapped) return true;
        const originalInvoke = holder.invoke;
        const wrappedInvoke = async function wrappedInvoke(command, ...rest) {
          try {
            if (typeof command === "string" && command.includes("plugin:shell|open")) {
              const args = rest[0];
              const href =
                typeof args?.path === "string" ? args.path
                  : typeof args?.url === "string" ? args.url
                    : typeof args?.href === "string" ? args.href
                      : "";
              state.events.push({
                at_ms: Date.now(),
                command,
                source: pathLabel,
                href,
              });
            }
          } catch {
            // ignore probe failures
          }
          return await originalInvoke.apply(this, [command, ...rest]);
        };
        wrappedInvoke.__ctxOauthProbeWrapped = true;
        holder.invoke = wrappedInvoke;
        state.wrapped_paths.push(pathLabel);
        return true;
      };

      const installedCore = wrapInvoke(root.__TAURI__?.core, "__TAURI__.core");
      const installedInternals = wrapInvoke(root.__TAURI_INTERNALS__, "__TAURI_INTERNALS__");
      const invokeHolder = typeof root.__TAURI_INTERNALS__?.invoke === "function"
        ? root.__TAURI_INTERNALS__
        : root.__TAURI__?.core;
      if (typeof invokeHolder?.invoke !== "function") {
        return {
          ok: false,
          error: "Tauri invoke is unavailable",
          probe: {
            installed: installedCore || installedInternals,
            wrapped_paths: Array.from(new Set(state.wrapped_paths)),
            event_count: Array.isArray(state.events) ? state.events.length : 0,
          },
        };
      }
      await invokeHolder.invoke("plugin:shell|open", { path: authUrl });
      return {
        ok: true,
        probe: {
          installed: installedCore || installedInternals,
          wrapped_paths: Array.from(new Set(state.wrapped_paths)),
          event_count: Array.isArray(state.events) ? state.events.length : 0,
        },
      };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  }, target, PROBE_GLOBAL_KEY);
  if (!result || result.ok !== true) {
    throw new Error(readString(result?.error) || "failed to open external auth URL");
  }
  return result.probe || null;
};

const createProviderOAuthHarness = ({
  outputPath = "",
  pollMs = DEFAULT_POLL_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  urlFallbackGraceMs = DEFAULT_URL_FALLBACK_GRACE_MS,
} = {}) => {
  const startedAt = nowIso();
  const sessions = new Map();
  const stageLog = [];
  const desktopOpenEvents = [];
  const seenDesktopOpenEventKeys = new Set();

  const writeArtifact = () => {
    if (!outputPath) return;
    const payload = {
      schema_version: 1,
      started_at: startedAt,
      updated_at: nowIso(),
      sessions: Array.from(sessions.values()).map((session) => ({
        provider_id: session.providerId,
        login_id: session.loginId,
        auth_url_source: session.authUrlSource || null,
        auth_url: sanitizeAuthUrl(session.authUrl),
        expected_callback_url: sanitizeAuthUrl(session.expectedCallbackUrl),
        completion_token_present: Boolean(session.completionToken),
        terminal_status: session.terminalStatus || null,
        account_id: session.accountId || null,
        error: session.error || null,
        status_history: session.statusHistory.slice(),
      })),
      desktop_open_external_events: desktopOpenEvents.map((event) => ({
        at: event.at,
        source: readString(event.source),
        auth_url: sanitizeAuthUrl(event.href),
      })),
      stages: stageLog.slice(),
    };
    fs.mkdirSync(path.dirname(outputPath), { recursive: true });
    fs.writeFileSync(outputPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  };

  const recordStage = (stage, detail, payload = {}, extras = {}) => {
    stageLog.push({
      at: nowIso(),
      stage,
      detail: readString(detail),
      payload: redactPayload(payload),
      ...redactPayload(extras),
    });
    writeArtifact();
  };

  const appendStatusHistory = (session, statusPayload) => {
    if (!statusPayload.status) return false;
    const previous = session.statusHistory[session.statusHistory.length - 1];
    if (
      previous
      && previous.status === statusPayload.status
      && previous.account_id === (statusPayload.accountId || null)
      && previous.error === (statusPayload.error || null)
    ) {
      return false;
    }
    session.statusHistory.push({
      at: nowIso(),
      status: statusPayload.status,
      account_id: statusPayload.accountId || null,
      error: statusPayload.error || null,
    });
    return true;
  };

  const getSession = (loginId) => {
    const normalized = readString(loginId);
    if (!normalized) {
      throw createProviderOAuthError("lookup_session", "loginId is required");
    }
    const session = sessions.get(normalized);
    if (session) return session;
    throw createProviderOAuthError("lookup_session", `unknown loginId: ${normalized}`, { loginId: normalized });
  };

  const fetchStatus = async (session) => {
    const response = await daemonJson("GET", session.descriptor.statusPath(session.loginId));
    if (response.status !== 200) {
      throw createProviderOAuthError(
        "fetch_login_status",
        `provider login status failed (${response.status})`,
        {
          providerId: session.providerId,
          loginId: session.loginId,
          response: redactPayload(response.payload || null),
        },
      );
    }
    const normalized = normalizeLoginStatusPayload(session.descriptor, response.payload);
    session.accountId = normalized.accountId || session.accountId;
    session.error = normalized.error || session.error;
    if (appendStatusHistory(session, normalized)) {
      recordStage(
        "login_status_transition",
        `${session.providerId} login transitioned to ${normalized.status || "unknown"}`,
        normalized.redactedPayload,
        { provider_id: session.providerId, login_id: session.loginId },
      );
    }
    if (normalized.authUrl) {
      session.authUrl = normalized.authUrl;
      session.authUrlSource = session.authUrlSource || "login_status";
    }
    if (normalized.expectedCallbackUrl) {
      session.expectedCallbackUrl = normalized.expectedCallbackUrl;
    }
    return normalized;
  };

  const startProviderLogin = async (providerId, label = "") => {
    const descriptor = getProviderDescriptor(providerId);
    const probeInfo = await installDesktopOpenExternalProbe();
    await resetDesktopOpenExternalProbe();
    const response = await daemonJson("POST", descriptor.startPath, label ? { label: readString(label) } : {});
    if (response.status !== 200) {
      throw createProviderOAuthError(
        "start_provider_login",
        `provider login start failed (${response.status})`,
        {
          providerId,
          response: redactPayload(response.payload || null),
        },
      );
    }
    const normalized = normalizeLoginStartPayload(descriptor, response.payload);
    if (!normalized.loginId) {
      throw createProviderOAuthError(
        "start_provider_login",
        `provider login start returned no login id for ${providerId}`,
        {
          providerId,
          response: normalized.redactedPayload,
        },
      );
    }

    const session = {
      descriptor,
      providerId,
      loginId: normalized.loginId,
      startedAtMs: Date.now(),
      authUrl: normalized.authUrl,
      authUrlSource: normalized.authUrl ? "start_response" : "",
      expectedCallbackUrl: normalized.expectedCallbackUrl,
      completionToken: normalized.completionToken,
      accountId: providerId === "codex" ? normalized.loginId : "",
      error: "",
      terminalStatus: "",
      statusHistory: [],
    };
    sessions.set(session.loginId, session);
    recordStage(
      "start_provider_login",
      `started ${providerId} login`,
      normalized.redactedPayload,
      {
        provider_id: providerId,
        login_id: session.loginId,
        desktop_open_probe: probeInfo,
      },
    );
    return {
      providerId,
      loginId: session.loginId,
      authUrl: normalized.authUrl,
      expectedCallbackUrl: normalized.expectedCallbackUrl || null,
      completionToken: normalized.completionToken || null,
    };
  };

  const awaitLoginUrl = async (loginId, timeout = timeoutMs) => {
    const session = getSession(loginId);
    const startedAuthUrl = sanitizeAuthUrl(session.authUrl);
    if (startedAuthUrl) {
      recordStage(
        "await_login_url",
        `${session.providerId} auth URL observed via ${session.authUrlSource || "start_response"}`,
        {
          auth_url: session.authUrl,
          latest_status: session.statusHistory[session.statusHistory.length - 1]?.status || null,
        },
        { provider_id: session.providerId, login_id: session.loginId },
      );
      return {
        providerId: session.providerId,
        loginId: session.loginId,
        source: session.authUrlSource || "start_response",
        authUrl: session.authUrl,
        sanitizedAuthUrl: startedAuthUrl,
      };
    }
    const deadline = Date.now() + Math.max(0, Number(timeout) || 0);
    let fallbackAuthUrl = session.authUrl;
    let fallbackObservedAtMs = fallbackAuthUrl ? Date.now() : 0;

    while (Date.now() <= deadline) {
      const probeEvents = await readDesktopOpenExternalProbe();
      const unseenProbeEvents = probeEvents.filter((entry) => {
        const key = JSON.stringify([
          Number(entry?.at_ms) || 0,
          readString(entry?.source),
          readString(entry?.href),
        ]);
        if (seenDesktopOpenEventKeys.has(key)) return false;
        seenDesktopOpenEventKeys.add(key);
        return true;
      });
      if (unseenProbeEvents.length) {
        for (const entry of unseenProbeEvents) {
          desktopOpenEvents.push({
            at: new Date(Number(entry?.at_ms) || Date.now()).toISOString(),
            source: readString(entry?.source),
            href: readString(entry?.href),
          });
        }
        writeArtifact();
      }

      const status = await fetchStatus(session);
      if (status.authUrl) {
        fallbackAuthUrl = status.authUrl;
        fallbackObservedAtMs = Date.now();
      }

      const observed = chooseObservedAuthUrl({
        probeEvents,
        fallbackAuthUrl,
      });
      if (observed) {
        const shouldUseFallback =
          observed.source !== "desktop_open_external"
          && Date.now() - fallbackObservedAtMs < urlFallbackGraceMs
          && !isTerminalStatus(status.status);
        if (!shouldUseFallback) {
          session.authUrl = observed.authUrl;
          session.authUrlSource = observed.source;
          recordStage(
            "await_login_url",
            `${session.providerId} auth URL observed via ${observed.source}`,
            {
              auth_url: observed.authUrl,
              latest_status: status.status,
            },
            { provider_id: session.providerId, login_id: session.loginId },
          );
          return {
            providerId: session.providerId,
            loginId: session.loginId,
            source: observed.source,
            authUrl: observed.authUrl,
            sanitizedAuthUrl: observed.sanitizedAuthUrl,
          };
        }
      }

      if (isTerminalStatus(status.status) && !fallbackAuthUrl) {
        throw createProviderOAuthError(
          "await_login_url",
          `${session.providerId} login reached terminal state before emitting auth URL (${status.status})`,
          {
            providerId: session.providerId,
            loginId: session.loginId,
            status: status.redactedPayload,
          },
        );
      }

      await waitMs(pollMs);
    }

    if (fallbackAuthUrl) {
      const sanitizedAuthUrl = sanitizeAuthUrl(fallbackAuthUrl);
      session.authUrl = fallbackAuthUrl;
      session.authUrlSource = "login_status";
      recordStage(
        "await_login_url",
        `${session.providerId} auth URL fallback from login status`,
        { auth_url: fallbackAuthUrl },
        { provider_id: session.providerId, login_id: session.loginId },
      );
      return {
        providerId: session.providerId,
        loginId: session.loginId,
        source: "login_status",
        authUrl: fallbackAuthUrl,
        sanitizedAuthUrl,
      };
    }

    throw createProviderOAuthError(
      "await_login_url",
      `timed out waiting for ${session.providerId} auth URL`,
      {
        providerId: session.providerId,
        loginId: session.loginId,
      },
    );
  };

  const awaitLoginTerminal = async (loginId, timeout = timeoutMs) => {
    const session = getSession(loginId);
    const deadline = Date.now() + Math.max(0, Number(timeout) || 0);

    while (Date.now() <= deadline) {
      const status = await fetchStatus(session);
      if (isTerminalStatus(status.status)) {
        session.terminalStatus = status.status;
        session.accountId = status.accountId || session.accountId;
        session.error = status.error || session.error;
        recordStage(
          "await_login_terminal",
          `${session.providerId} login reached ${status.status}`,
          status.redactedPayload,
          { provider_id: session.providerId, login_id: session.loginId },
        );
        return status;
      }
      await waitMs(pollMs);
    }

    throw createProviderOAuthError(
      "await_login_terminal",
      `timed out waiting for ${session.providerId} login terminal status`,
      {
        providerId: session.providerId,
        loginId: session.loginId,
      },
    );
  };

  const assertAccountActivated = async (providerId) => {
    const descriptor = getProviderDescriptor(providerId);
    const response = await daemonJson("GET", descriptor.accountsPath);
    if (response.status !== 200) {
      throw createProviderOAuthError(
        "assert_account_activated",
        `accounts fetch failed for ${providerId} (${response.status})`,
        {
          providerId,
          response: redactPayload(response.payload || null),
        },
      );
    }

    const normalized = normalizeAccountsPayload(descriptor, response.payload);
    if (!normalized.activeAccountId) {
      throw createProviderOAuthError(
        "assert_account_activated",
        `provider ${providerId} has no active account after login`,
        {
          providerId,
          response: normalized.redactedPayload,
        },
      );
    }
    const activeAccount = normalized.accounts.find((entry) => readString(entry.id) === normalized.activeAccountId);
    if (!activeAccount) {
      throw createProviderOAuthError(
        "assert_account_activated",
        `active account ${normalized.activeAccountId} not found in ${providerId} account list`,
        {
          providerId,
          response: normalized.redactedPayload,
        },
      );
    }

    const successfulSessions = Array.from(sessions.values()).filter(
      (session) => session.providerId === providerId && session.terminalStatus === "success",
    );
    if (successfulSessions.length > 0) {
      const expectedIds = new Set(successfulSessions.map((session) => session.accountId).filter(Boolean));
      if (expectedIds.size > 0 && !expectedIds.has(normalized.activeAccountId)) {
        throw createProviderOAuthError(
          "assert_account_activated",
          `active account ${normalized.activeAccountId} does not match successful ${providerId} login`,
          {
            providerId,
            expectedAccountIds: Array.from(expectedIds),
            response: normalized.redactedPayload,
          },
        );
      }
    }

    recordStage(
      "assert_account_activated",
      `${providerId} active account verified`,
      normalized.redactedPayload,
      { provider_id: providerId, active_account_id: normalized.activeAccountId },
    );
    return {
      providerId,
      activeAccountId: normalized.activeAccountId,
      accountCount: normalized.accounts.length,
      account: redactPayload(activeAccount),
    };
  };

  const openAuthUrl = async (href) => {
    const probeInfo = await installDesktopOpenExternalProbe();
    const runtimeProbe = await openExternalUrlViaDesktop(href);
    recordStage(
      "open_auth_url",
      "opened auth URL through desktop shell path",
      { auth_url: href },
      { desktop_open_probe: probeInfo, desktop_open_runtime_probe: runtimeProbe },
    );
  };

  return {
    startProviderLogin,
    awaitLoginUrl,
    awaitLoginTerminal,
    readLoginStatus: async (loginId) => {
      const session = getSession(loginId);
      return await fetchStatus(session);
    },
    assertAccountActivated,
    openAuthUrl,
    installDesktopOpenExternalProbe,
    readDesktopOpenExternalProbe,
    resetDesktopOpenExternalProbe,
    recordStage,
  };
};

const completeCodexOauthWithBrowserCredentials = async ({
  label = "",
  email,
  password,
  totpSecret,
  outputPath = "",
  pollMs = DEFAULT_POLL_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  urlFallbackGraceMs = DEFAULT_URL_FALLBACK_GRACE_MS,
} = {}) => {
  const harness = createProviderOAuthHarness({
    outputPath,
    pollMs,
    timeoutMs,
    urlFallbackGraceMs,
  });
  const login = await harness.startProviderLogin("codex", label);
  if (login.expectedCallbackUrl && login.completionToken) {
    await startCodexDesktopRelay({
      loginId: login.loginId,
      callbackUrl: login.expectedCallbackUrl,
      completionToken: login.completionToken,
    });
  }

  const authUrl = await harness.awaitLoginUrl(login.loginId, timeoutMs);
  let browserFlow;
  try {
    browserFlow = await driveCodexOpenAiLoginWithCredentials({
      authUrl: authUrl.authUrl,
      expectedCallbackUrl: login.expectedCallbackUrl,
      email,
      password,
      totpSecret,
      timeoutMs,
      pollMs,
    });
  } catch (error) {
    if (!isProviderOAuthBlockedError(error)) {
      throw error;
    }

    let loginStatus = null;
    try {
      const status = await harness.readLoginStatus(login.loginId);
      loginStatus = status.redactedPayload || redactPayload(status);
    } catch (statusError) {
      loginStatus = {
        status: "unavailable",
        error: String(statusError),
      };
    }

    harness.recordStage(
      "codex_browser_flow_blocked",
      error.message,
      {
        blocked_reason: error.blockedReason,
        challenge: error.challenge || null,
      },
      { provider_id: "codex", login_id: login.loginId },
    );
    return {
      status: "blocked",
      providerId: "codex",
      loginId: login.loginId,
      authUrl: authUrl.sanitizedAuthUrl,
      browserFlow: {
        status: "blocked",
        blocked_reason: error.blockedReason,
        message: error.message,
        challenge: error.challenge || null,
      },
      loginStatus,
    };
  }

  harness.recordStage(
    "codex_browser_flow_callback_reached",
    "codex browser flow reached callback",
    browserFlow,
    { provider_id: "codex", login_id: login.loginId },
  );
  const terminal = await harness.awaitLoginTerminal(login.loginId, timeoutMs);
  if (terminal.status !== "success") {
    throw new Error(`codex oauth login did not succeed: ${JSON.stringify(terminal.redactedPayload || terminal)}`);
  }
  const activeAccount = await harness.assertAccountActivated("codex");
  return {
    status: "success",
    providerId: "codex",
    loginId: login.loginId,
    authUrl: authUrl.sanitizedAuthUrl,
    browserFlow,
    terminal: terminal.redactedPayload || redactPayload(terminal),
    activeAccount,
  };
};

module.exports = {
  PROVIDER_OAUTH_DESCRIPTORS,
  sanitizeAuthUrl,
  normalizeBase32Secret,
  decodeBase32Secret,
  createTotpCode,
  redactPayload,
  fillBrowserAuthField,
  submitVisibleAuthStep,
  startCodexDesktopRelay,
  driveCodexOpenAiLoginWithCredentials,
  normalizeLoginStartPayload,
  normalizeLoginStatusPayload,
  normalizeAccountsPayload,
  isTerminalStatus,
  chooseObservedAuthUrl,
  createProviderOAuthHarness,
  completeCodexOauthWithBrowserCredentials,
  isProviderOAuthBlockedError,
};
