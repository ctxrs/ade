const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { chromium } = require("playwright");

const { waitForTauri } = require("./helpers/tauri.cjs");
const { daemonJson } = require("./helpers/daemon.cjs");
const {
  mkTempDir,
  initGitRepo,
  scenarioEnabled,
  assertConnectedLocalAndListening,
  runProviderFirstTurnApiSmoke,
  runProviderFileEditApiSmoke,
} = require("./helpers/workspace_wizard_flow.cjs");
const {
  getProviderStatus,
  installProviderAndWait,
  verifyProviderForWorkspace,
  resolveWorkspaceProviderModelId,
  selectSubscriptionSource,
} = require("./helpers/provider_runtime.cjs");
const {
  createProviderOAuthHarness,
  startCodexDesktopRelay,
  createTotpCode,
} = require("./helpers/provider_oauth_flow.cjs");
const { resolveBoolishFlag } = require("../../../../scripts/lib/boolish.cjs");

const DEFAULT_CASE_TIMEOUT_MS = 20 * 60_000;
const DEFAULT_LOGIN_TIMEOUT_MS = 15 * 60_000;
const DEFAULT_MODEL_POPULATION_TIMEOUT_MS = 20_000;

const parsePositiveInt = (raw, fallback) => {
  const parsed = Number.parseInt(String(raw || ""), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback;
  return parsed;
};

const waitMs = (ms) => new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(ms) || 0)));

const normalizeText = (value) => String(value || "").trim();
const escapeRegExp = (value) => String(value || "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const DEFAULT_AUTH_WINDOW = { width: 1440, height: 1080 };

const AUTH_INPUT_SELECTOR_GROUPS = {
  email: [
    "input[type='email']",
    "input[autocomplete='email']",
    "input[name*='email' i]",
    "input[name='identifier']",
    "input[name='username']",
  ],
  password: [
    "input[type='password']",
    "input[autocomplete='current-password']",
    "input[autocomplete='password']",
  ],
  otp: [
    "input[autocomplete='one-time-code']",
    "input[inputmode='numeric']",
    "input[name*='otp' i]",
    "input[name*='code' i]",
  ],
};

const parseBool = (value, fallback = false) => {
  const normalized = normalizeText(value).toLowerCase();
  if (!normalized) return fallback;
  if (["1", "true", "yes", "on"].includes(normalized)) return true;
  if (["0", "false", "no", "off"].includes(normalized)) return false;
  return fallback;
};

const sanitizeUrlForLog = (href) => {
  const raw = normalizeText(href);
  if (!raw) return "";
  try {
    const parsed = new URL(raw);
    return `${parsed.protocol}//${parsed.host}${parsed.pathname}`;
  } catch {
    return raw;
  }
};

const stateMentions = (state, pattern) => pattern.test(
  [
    normalizeText(state?.url),
    normalizeText(state?.title),
    normalizeText(state?.bodyText),
    ...(Array.isArray(state?.buttons) ? state.buttons : []),
    ...(Array.isArray(state?.inputs) ? state.inputs.map((entry) => `${entry.label} ${entry.name} ${entry.type} ${entry.autocomplete}`) : []),
  ].join("\n"),
);

const callbackReached = (currentUrl, expectedCallbackUrl) => {
  const current = normalizeText(currentUrl);
  const expected = normalizeText(expectedCallbackUrl);
  if (!current || !expected) return false;
  try {
    const currentParsed = new URL(current);
    const expectedParsed = new URL(expected);
    return currentParsed.origin === expectedParsed.origin && currentParsed.pathname === expectedParsed.pathname;
  } catch {
    return current.startsWith(expected);
  }
};

const readVisibleDomState = async (page) => await page.evaluate((textLimit) => {
  const normalize = (value) => String(value ?? "").replace(/\s+/g, " ").trim().toLowerCase();
  const isVisible = (element) => {
    if (!(element instanceof HTMLElement)) return false;
    const style = window.getComputedStyle(element);
    if (!style || style.display === "none" || style.visibility === "hidden") return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  const labelTextFor = (element) => {
    const values = [];
    values.push(element.getAttribute("aria-label") || "");
    values.push(element.getAttribute("placeholder") || "");
    const labelledBy = element.getAttribute("aria-labelledby");
    if (labelledBy) {
      for (const id of labelledBy.split(/\s+/g)) {
        const ref = document.getElementById(id);
        if (ref) values.push(ref.textContent || "");
      }
    }
    if ("labels" in element) {
      for (const label of Array.from(element.labels || [])) {
        values.push(label.textContent || "");
      }
    }
    const parentLabel = element.closest("label");
    if (parentLabel) values.push(parentLabel.textContent || "");
    return normalize(values.join(" "));
  };
  const inputs = Array.from(document.querySelectorAll("input, textarea"))
    .filter((element) => isVisible(element))
    .map((element) => ({
      tag: element.tagName.toLowerCase(),
      type: normalize(element.getAttribute("type") || ""),
      name: normalize(element.getAttribute("name") || ""),
      autocomplete: normalize(element.getAttribute("autocomplete") || ""),
      inputmode: normalize(element.getAttribute("inputmode") || ""),
      label: labelTextFor(element),
      maxLength: Number(element.maxLength || 0),
      readOnly: Boolean(element.readOnly),
      disabled: Boolean(element.disabled),
    }));
  const buttons = Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], a"))
    .filter((element) => isVisible(element))
    .map((element) => normalize(element.textContent || element.getAttribute("value") || element.getAttribute("aria-label")))
    .filter(Boolean);
  return {
    url: String(window.location.href || ""),
    title: normalize(document.title || ""),
    bodyText: normalize(document.body?.innerText || "").slice(0, Math.max(0, Number(textLimit) || 0)),
    inputs,
    buttons,
  };
}, 5000);

const readVisibleDomStateStable = async (page) => {
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      return await readVisibleDomState(page);
    } catch (error) {
      const message = String(error || "");
      if (
        !/Execution context was destroyed|Cannot find context with specified id|Target page, context or browser has been closed/i.test(message)
        || attempt === 4
      ) {
        throw error;
      }
      await page.waitForLoadState("domcontentloaded", { timeout: 5000 }).catch(() => {});
      await waitMs(250);
    }
  }
  throw new Error("failed to read auth page state after navigation retries");
};

const tryClickLocator = async (locator) => {
  try {
    await locator.click({ timeout: 3000 });
    return true;
  } catch {
    return false;
  }
};

const clickVisibleTextAction = async (page, texts) => {
  for (const text of texts) {
    const pattern = new RegExp(escapeRegExp(text), "i");
    for (const locator of [
      page.getByRole("button", { name: pattern }),
      page.getByRole("link", { name: pattern }),
      page.getByLabel(pattern),
      page.getByText(pattern, { exact: false }),
    ]) {
      const count = await locator.count().catch(() => 0);
      for (let index = 0; index < count; index += 1) {
        const candidate = locator.nth(index);
        if (!(await candidate.isVisible().catch(() => false))) continue;
        if (await tryClickLocator(candidate)) return true;
      }
    }
  }
  return false;
};

const fillFirstMatchingField = async (page, selectors, value) => {
  const normalizedValue = String(value || "");
  for (const selector of selectors) {
    const locator = page.locator(`${selector}:visible`);
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      if (!(await candidate.isVisible().catch(() => false))) continue;
      if (!(await candidate.isEditable().catch(() => false))) continue;
      try {
        await candidate.click({ timeout: 3000 });
        const existingValue = await candidate.inputValue().catch(() => "");
        if (existingValue && existingValue !== normalizedValue) {
          await candidate.press(process.platform === "darwin" ? "Meta+A" : "Control+A").catch(() => {});
          await candidate.press("Backspace").catch(() => {});
        }
        if (typeof candidate.pressSequentially === "function") {
          await candidate.pressSequentially(normalizedValue, { delay: 35 }).catch(() => {});
        } else {
          await candidate.type(normalizedValue, { delay: 35 }).catch(() => {});
        }
        let actual = await candidate.inputValue().catch(() => "");
        if (actual === normalizedValue) return true;
        await candidate.fill("").catch(() => {});
        await candidate.fill(normalizedValue).catch(() => {});
        actual = await candidate.inputValue().catch(() => "");
        if (actual === normalizedValue) return true;
      } catch {
        continue;
      }
    }
  }
  return false;
};

const firstVisibleEditableLocator = async (page, selectors) => {
  for (const selector of selectors) {
    const locator = page.locator(`${selector}:visible`);
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      if (!(await candidate.isVisible().catch(() => false))) continue;
      if (!(await candidate.isEditable().catch(() => false))) continue;
      return candidate;
    }
  }
  return null;
};

const readFirstVisibleFieldValue = async (page, selectors) => {
  for (const selector of selectors) {
    const locator = page.locator(`${selector}:visible`);
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      if (!(await candidate.isVisible().catch(() => false))) continue;
      return await candidate.inputValue().catch(() => "");
    }
  }
  return "";
};

const submitVisibleAuthStep = async (page, labels) => {
  if (await clickVisibleTextAction(page, labels)) return true;
  await page.keyboard.press("Enter").catch(() => {});
  return true;
};

const launchExternalCodexBrowserContext = async () => {
  const userDataDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-codex-oauth-browser-"));
  const headless = parseBool(process.env.CTX_AUTOMATION_CODEX_OAUTH_BROWSER_HEADLESS, false);
  const channel = normalizeText(process.env.CTX_AUTOMATION_CODEX_OAUTH_BROWSER_CHANNEL)
    || (process.platform === "darwin" ? "chrome" : "");
  const context = await chromium.launchPersistentContext(userDataDir, {
    channel: channel || undefined,
    headless,
    viewport: DEFAULT_AUTH_WINDOW,
    screen: DEFAULT_AUTH_WINDOW,
    args: [
      "--disable-blink-features=AutomationControlled",
      `--window-size=${DEFAULT_AUTH_WINDOW.width},${DEFAULT_AUTH_WINDOW.height}`,
    ],
    ignoreDefaultArgs: ["--enable-automation"],
  });
  return {
    context,
    dispose: async () => {
      await context.close().catch(() => {});
      fs.rmSync(userDataDir, { recursive: true, force: true });
    },
  };
};

const captureAuthFailureEvidence = async (page, reason) => {
  const stamp = Date.now();
  const screenshotPath = path.join("/tmp", `ctx-codex-oauth-browser-failure-${stamp}.png`);
  const htmlPath = path.join("/tmp", `ctx-codex-oauth-browser-failure-${stamp}.html`);
  try {
    await page.screenshot({ path: screenshotPath, fullPage: true });
  } catch {
    // ignore screenshot failures
  }
  try {
    fs.writeFileSync(htmlPath, await page.content(), "utf8");
  } catch {
    // ignore HTML capture failures
  }
  return {
    reason,
    screenshotPath,
    htmlPath,
  };
};

const driveCodexOpenAiLoginInExternalBrowser = async ({
  page,
  authUrl,
  expectedCallbackUrl,
  email,
  password,
  totpSecret,
  timeoutMs,
  pollMs,
}) => {
  const normalizedEmail = normalizeText(email);
  const normalizedPassword = String(password || "");
  const normalizedTotpSecret = normalizeText(totpSecret);
  if (!normalizedEmail) throw new Error("CTX_E2E_CODEX_OAUTH_EMAIL is required");
  if (!normalizedPassword) throw new Error("CTX_E2E_CODEX_OAUTH_PASSWORD is required");
  if (!normalizedTotpSecret) throw new Error("CTX_E2E_CODEX_OAUTH_TOTP_SECRET is required");

  await page.goto(authUrl, { waitUntil: "domcontentloaded", timeout: 120_000 });
  const startedAt = Date.now();
  let lastState = null;
  let usedTotp = false;
  let clickedLogin = false;
  let selectedKnownAccount = false;
  let lastSubmittedStep = "";
  let lastSubmittedAtMs = 0;

  const submittedRecently = (step) =>
    lastSubmittedStep === step && Date.now() - lastSubmittedAtMs < Math.max(2000, pollMs * 3);

  const markSubmitted = (step) => {
    lastSubmittedStep = step;
    lastSubmittedAtMs = Date.now();
  };

  while (Date.now() - startedAt <= timeoutMs) {
    const state = await readVisibleDomStateStable(page);
    lastState = state;
    if (callbackReached(state.url, expectedCallbackUrl) || stateMentions(state, /completing sign-in|login callback received/i)) {
      return {
        status: "callback_reached",
        finalUrl: sanitizeUrlForLog(state.url),
        usedTotp,
      };
    }

    const passwordField = await firstVisibleEditableLocator(page, AUTH_INPUT_SELECTOR_GROUPS.password);
    const emailField = await firstVisibleEditableLocator(page, AUTH_INPUT_SELECTOR_GROUPS.email);
    const otpField = await firstVisibleEditableLocator(page, AUTH_INPUT_SELECTOR_GROUPS.otp);
    const hasPasswordInput = Boolean(passwordField);
    const hasEmailInput = Boolean(emailField);
    const hasOtpInput = Boolean(otpField);

    if (stateMentions(state, /check your inbox|sent to your email|email verification code/i)) {
      throw new Error(`OpenAI presented an email verification challenge: ${JSON.stringify({ url: sanitizeUrlForLog(state.url), title: state.title })}`);
    }
    if (stateMentions(state, /captcha|verify you are human|unusual activity|suspicious/i)) {
      throw new Error(`OpenAI presented an interactive challenge: ${JSON.stringify({ url: sanitizeUrlForLog(state.url), title: state.title })}`);
    }
    if (stateMentions(state, /incorrect|invalid password|wrong password|try again later|too many requests/i)) {
      throw new Error(`OpenAI auth page reported an error: ${JSON.stringify({ url: sanitizeUrlForLog(state.url), title: state.title, bodyText: state.bodyText.slice(0, 300) })}`);
    }

    if (
      /\/sign-in-with-chatgpt\/codex\/consent\b/i.test(state.url)
      || stateMentions(state, /sign in to codex with chatgpt|codex with chatgpt/i)
    ) {
      if (submittedRecently("consent")) {
        await waitMs(pollMs);
        continue;
      }
      await submitVisibleAuthStep(page, ["continue", "allow", "authorize", "approve", "confirm", "sign in"]);
      markSubmitted("consent");
      await waitMs(pollMs);
      continue;
    }

    if (!hasEmailInput && !hasPasswordInput && !hasOtpInput && !clickedLogin && stateMentions(state, /log in|login/i)) {
      const clicked = await clickVisibleTextAction(page, ["log in", "login"]);
      if (clicked) {
        clickedLogin = true;
        markSubmitted("login");
        await waitMs(pollMs);
        continue;
      }
    }

    if (!hasEmailInput && !hasPasswordInput && !hasOtpInput && !selectedKnownAccount && stateMentions(state, new RegExp(escapeRegExp(normalizedEmail), "i"))) {
      const clicked = await clickVisibleTextAction(page, [normalizedEmail]);
      if (clicked) {
        selectedKnownAccount = true;
        await waitMs(pollMs);
        continue;
      }
    }

    if (hasPasswordInput) {
      if (submittedRecently("password")) {
        await waitMs(pollMs);
        continue;
      }
      const filled = await fillFirstMatchingField(page, AUTH_INPUT_SELECTOR_GROUPS.password, normalizedPassword);
      if (!filled) {
        const evidence = await captureAuthFailureEvidence(page, "failed_to_fill_password");
        throw new Error(`failed to fill OpenAI password field: ${JSON.stringify({
          evidence,
          state: {
            url: sanitizeUrlForLog(state.url),
            title: state.title,
            bodyText: state.bodyText.slice(0, 300),
            buttons: state.buttons.slice(0, 12),
            inputs: state.inputs,
          },
        })}`);
      }
      await submitVisibleAuthStep(page, ["continue", "log in", "login", "next"]);
      markSubmitted("password");
      await waitMs(pollMs);
      continue;
    }

    if (hasEmailInput) {
      const visibleEmailValue = normalizeText(await readFirstVisibleFieldValue(page, AUTH_INPUT_SELECTOR_GROUPS.email)).toLowerCase();
      if (visibleEmailValue && visibleEmailValue === normalizedEmail.toLowerCase()) {
        if (submittedRecently("email")) {
          await waitMs(pollMs);
          continue;
        }
        await submitVisibleAuthStep(page, ["continue with email", "continue", "next", "log in", "login"]);
        markSubmitted("email");
        await waitMs(pollMs);
        continue;
      }
      if (submittedRecently("email")) {
        await waitMs(pollMs);
        continue;
      }
      const filled = await fillFirstMatchingField(page, AUTH_INPUT_SELECTOR_GROUPS.email, normalizedEmail);
      if (!filled) {
        const evidence = await captureAuthFailureEvidence(page, "failed_to_fill_email");
        throw new Error(`failed to fill OpenAI email field: ${JSON.stringify({
          evidence,
          state: {
            url: sanitizeUrlForLog(state.url),
            title: state.title,
            bodyText: state.bodyText.slice(0, 300),
            buttons: state.buttons.slice(0, 12),
            inputs: state.inputs,
          },
        })}`);
      }
      await submitVisibleAuthStep(page, ["continue with email", "continue", "next", "log in", "login"]);
      markSubmitted("email");
      await waitMs(pollMs);
      continue;
    }

    if (hasOtpInput) {
      if (submittedRecently("otp")) {
        await waitMs(pollMs);
        continue;
      }
      const code = createTotpCode(normalizedTotpSecret);
      const filled = await fillFirstMatchingField(page, AUTH_INPUT_SELECTOR_GROUPS.otp, code);
      if (!filled) {
        const evidence = await captureAuthFailureEvidence(page, "failed_to_fill_totp");
        throw new Error(`failed to fill OpenAI TOTP field: ${JSON.stringify({
          evidence,
          state: {
            url: sanitizeUrlForLog(state.url),
            title: state.title,
            bodyText: state.bodyText.slice(0, 300),
            buttons: state.buttons.slice(0, 12),
            inputs: state.inputs,
          },
        })}`);
      }
      usedTotp = true;
      await submitVisibleAuthStep(page, ["continue", "verify", "submit", "log in", "login"]);
      markSubmitted("otp");
      await waitMs(pollMs);
      continue;
    }

    await waitMs(pollMs);
  }

  throw new Error(`timed out driving external Codex OAuth browser flow: ${JSON.stringify({
    url: sanitizeUrlForLog(lastState?.url),
    title: normalizeText(lastState?.title),
    bodyText: normalizeText(lastState?.bodyText).slice(0, 300),
    buttons: Array.isArray(lastState?.buttons) ? lastState.buttons.slice(0, 10) : [],
    inputs: Array.isArray(lastState?.inputs) ? lastState.inputs : [],
  })}`);
};

const completeCodexOauthWithExternalBrowserCredentials = async ({
  label = "",
  email,
  password,
  totpSecret,
  outputPath = "",
  pollMs = 1000,
  timeoutMs = DEFAULT_LOGIN_TIMEOUT_MS,
  urlFallbackGraceMs = 2000,
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
  const browserContext = await launchExternalCodexBrowserContext();
  try {
    const page = await browserContext.context.newPage();
    const browserFlow = await driveCodexOpenAiLoginInExternalBrowser({
      page,
      authUrl: authUrl.authUrl,
      expectedCallbackUrl: login.expectedCallbackUrl,
      email,
      password,
      totpSecret,
      timeoutMs,
      pollMs,
    });
    harness.recordStage(
      "codex_external_browser_callback_reached",
      "codex external browser flow reached callback",
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
      terminal: terminal.redactedPayload || terminal,
      activeAccount,
    };
  } finally {
    await browserContext.dispose();
  }
};

const readWorkspaceProviderOptions = async (workspaceId, providerId) => {
  const response = await daemonJson(
    "GET",
    `/api/workspaces/${workspaceId}/providers/${encodeURIComponent(providerId)}/options`,
  );
  if (response.status !== 200) {
    throw new Error(
      `provider options read failed (${response.status}): ${JSON.stringify(response.payload || null)}`,
    );
  }
  return response.payload || {};
};

const clearCodexActiveAccount = async () => {
  const response = await daemonJson("PUT", "/api/providers/codex/active-account", {
    account_id: null,
  });
  if (response.status !== 200) {
    throw new Error(
      `clear codex active account failed (${response.status}): ${JSON.stringify(response.payload || null)}`,
    );
  }
};

const seedUnauthenticatedCodexOptionsCache = async (workspaceId) => {
  await selectSubscriptionSource("codex");
  await clearCodexActiveAccount();
  const payload = await readWorkspaceProviderOptions(workspaceId, "codex");
  const models = payload?.models && typeof payload.models === "object" ? payload.models : {};
  const currentModelId = normalizeText(models.current_model_id || models.currentModelId);
  const modelCount = Array.isArray(models.models) ? models.models.length : 0;
  if (currentModelId || modelCount > 0) {
    throw new Error(
      `expected empty Codex model options before OAuth login, got ${JSON.stringify(payload)}`,
    );
  }
  return payload;
};

const ensureCodexProviderInstalled = async () => {
  const status = await getProviderStatus("codex", "host");
  if (status.installed) return status;
  await installProviderAndWait("codex", "host");
  return await getProviderStatus("codex", "host");
};

const writeSkipReport = (reportPath, reason) => {
  if (!reportPath) return;
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, `${JSON.stringify({
    schema_version: 1,
    result: "skipped",
    reason: normalizeText(reason),
    completed_at: new Date().toISOString(),
  }, null, 2)}\n`, "utf8");
};

const createWorkspaceAndLaunchExecution = async ({ baseDir, name }) => {
  const dest = path.join(baseDir, name.replace(/[^a-zA-Z0-9._-]+/g, "-"));
  initGitRepo(dest, name);

  const create = await daemonJson("POST", "/api/workspaces", {
    root_path: dest,
    name,
  });
  if (create.status !== 200) {
    throw new Error(`workspace create failed (${create.status}): ${JSON.stringify(create.payload || null)}`);
  }
  const workspaceId = normalizeText(create.payload?.id);
  if (!workspaceId) {
    throw new Error(`workspace create response missing id: ${JSON.stringify(create.payload || null)}`);
  }

  const setExec = await daemonJson("POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment: "host",
    network_mode: "all",
  });
  if (setExec.status !== 200) {
    throw new Error(`execution config update failed (${setExec.status}): ${JSON.stringify(setExec.payload || null)}`);
  }

  const launch = await daemonJson("POST", "/api/execution/launch/start", {
    workspace_id: workspaceId,
  });
  if (launch.status !== 200) {
    throw new Error(`execution launch start failed (${launch.status}): ${JSON.stringify(launch.payload || null)}`);
  }
  const jobId = normalizeText(launch.payload?.job_id);
  if (!jobId) {
    throw new Error(`execution launch response missing job_id: ${JSON.stringify(launch.payload || null)}`);
  }

  const deadline = Date.now() + 5 * 60_000;
  let lastPayload = null;
  while (Date.now() <= deadline) {
    const status = await daemonJson("GET", `/api/execution/launch/status?job_id=${encodeURIComponent(jobId)}`);
    if (status.status === 200) {
      lastPayload = status.payload || null;
      const state = normalizeText(status.payload?.state).toLowerCase();
      if (state === "ready") {
        return { workspaceId, dest };
      }
      if (state === "error") {
        throw new Error(`execution launch failed: ${JSON.stringify(status.payload || null)}`);
      }
    }
    await waitMs(1000);
  }

  throw new Error(`execution launch timed out for workspace=${workspaceId}; last=${JSON.stringify(lastPayload)}`);
};

describe("codex oauth harness framework (desktop e2e)", () => {
  const runId = `${Date.now()}`;
  const reportPath = normalizeText(process.env.CTX_AUTOMATION_CODEX_OAUTH_REPORT)
    || path.join("/tmp", `ctx-codex-oauth-framework-${runId}.json`);
  const localBase = mkTempDir(`ctx-codex-oauth-framework-${runId}-`);

  before(async () => {
    await browser.url(`tauri://localhost/workspace-setup?codexOauthFramework=${runId}`);
    await waitForTauri();
  });

  after(async () => {
    try {
      fs.rmSync(localBase, { recursive: true, force: true });
    } catch {
      // ignore cleanup failures
    }
  });

  it("drives the Codex subscription login lifecycle through the shared oauth helper", async function () {
    this.timeout(parsePositiveInt(process.env.CTX_AUTOMATION_CASE_TIMEOUT_MS || "", DEFAULT_CASE_TIMEOUT_MS));

    if (!scenarioEnabled("local-codex-smoke", ["local", "provider", "oauth"])) this.skip();

    const enabled = resolveBoolishFlag(
      process.env.CTX_AUTOMATION_CODEX_OAUTH_ENABLE,
      false,
      "CTX_AUTOMATION_CODEX_OAUTH_ENABLE",
    );
    if (!enabled) {
      const reason =
        "CTX_AUTOMATION_CODEX_OAUTH_ENABLE=1 is required for codex-oauth-framework.spec.cjs; this flow needs real external Codex/ChatGPT browser credentials and is intentionally gated when they are absent.";
      console.error(`[skip] ${reason}`);
      writeSkipReport(reportPath, reason);
      this.skip();
    }

    const oauthTimeoutMs = parsePositiveInt(
      process.env.CTX_AUTOMATION_CODEX_OAUTH_TIMEOUT_MS || "",
      DEFAULT_LOGIN_TIMEOUT_MS,
    );
    const modelPopulationTimeoutMs = parsePositiveInt(
      process.env.CTX_AUTOMATION_CODEX_MODEL_POPULATION_TIMEOUT_MS || "",
      DEFAULT_MODEL_POPULATION_TIMEOUT_MS,
    );

    let workspaceId = "";
    try {
      await assertConnectedLocalAndListening();
      const workspace = await createWorkspaceAndLaunchExecution({
        baseDir: localBase,
        name: `codex-oauth-framework-${runId}`,
      });
      workspaceId = workspace.workspaceId;
      await seedUnauthenticatedCodexOptionsCache(workspace.workspaceId);

      const oauthLogin = await completeCodexOauthWithExternalBrowserCredentials({
        label: normalizeText(process.env.CTX_AUTOMATION_CODEX_OAUTH_LABEL) || `codex-oauth-${runId}`,
        email: normalizeText(process.env.CTX_E2E_CODEX_OAUTH_EMAIL),
        password: process.env.CTX_E2E_CODEX_OAUTH_PASSWORD || "",
        totpSecret: process.env.CTX_E2E_CODEX_OAUTH_TOTP_SECRET || "",
        outputPath: reportPath,
        pollMs: 1000,
        timeoutMs: oauthTimeoutMs,
        urlFallbackGraceMs: 2000,
      });

      await ensureCodexProviderInstalled();
      await verifyProviderForWorkspace(workspace.workspaceId, "codex");
      const modelId = await resolveWorkspaceProviderModelId(workspace.workspaceId, "codex", {
        timeoutMs: modelPopulationTimeoutMs,
        pollMs: 2000,
      });
      await runProviderFirstTurnApiSmoke(
        workspace.workspaceId,
        {
          providerId: "codex",
          modelId,
          prompt: `codex-oauth-framework-${Date.now()}: reply with exactly pong`,
        },
        240_000,
      );
      const writeToken = `CTX_CODEX_WRITE_OK_${Date.now()}`;
      await runProviderFileEditApiSmoke(
        workspace.workspaceId,
        workspace.dest,
        {
          providerId: "codex",
          modelId,
          relativeFilePath: "codex-write-proof.txt",
          fileContents: writeToken,
        },
        240_000,
      );
    } finally {
      try {
        await clearCodexActiveAccount();
      } catch {
        // ignore cleanup failures
      }
      if (workspaceId) {
        await daemonJson("DELETE", `/api/workspaces/${workspaceId}`);
      }
    }
  });
});
