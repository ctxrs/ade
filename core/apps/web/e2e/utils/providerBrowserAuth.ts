import { execFileSync, spawn, type ChildProcess } from "child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "fs";
import { createServer } from "net";
import { tmpdir } from "os";
import path from "path";
import { chromium, type APIRequestContext, type BrowserContext, type Locator, type Page } from "playwright/test";

type ClaudeLoginStartResponse = {
  login_id?: unknown;
  auth_url?: unknown;
};

type ClaudeLoginStatus = {
  status?: unknown;
  auth_url?: unknown;
  account_id?: unknown;
  error?: unknown;
};

type VisibleDomState = {
  url: string;
  title: string;
  bodyText: string;
  inputs: Array<{
    tag: string;
    type: string;
    name: string;
    autocomplete: string;
    inputmode: string;
    label: string;
    maxLength: number;
  }>;
  buttons: string[];
};

type DriveProgress = {
  clickedGoogleEntry: boolean;
  selectedAccount: boolean;
  usedEmail: boolean;
  usedPassword: boolean;
  grantedConsent: boolean;
};

type DriveStateArgs = {
  page: Page;
  state: VisibleDomState;
  progress: DriveProgress;
  hasEmailInput: boolean;
  hasPasswordInput: boolean;
  hasOtpInput: boolean;
  isGoogleHost: boolean;
};

type DriveStateResult =
  | { done: true; result: Record<string, unknown> }
  | { handled: true; page?: Page }
  | null;

type GoogleDriveOptions = {
  page: Page;
  authUrl: string;
  email: string;
  password: string;
  providerLabel: string;
  timeoutMs?: number;
  pollMs?: number;
  onState?: (args: DriveStateArgs) => Promise<DriveStateResult>;
};

export type ClaudeBrowserOauthOptions = {
  context: BrowserContext;
  request: APIRequestContext;
  email: string;
  password: string;
  label?: string;
  timeoutMs?: number;
  pollMs?: number;
};

export type BrowserAuthSuccess = {
  loginId: string;
  accountId: string;
  authUrl: string;
};

export type BrowserAuthContextHandle = {
  context: BrowserContext;
  dispose: () => Promise<void>;
};

const DEFAULT_TIMEOUT_MS = 5 * 60_000;
const DEFAULT_POLL_MS = 1_000;
const DEFAULT_CLAUDE_POST_GOOGLE_SETTLE_MS = 90_000;
const DEFAULT_AUTH_WINDOW_SIZE = { width: 1920, height: 1080 } as const;
const TEXT_LIMIT = 4_000;
const BROWSER_AUTH_DEBUG_ENABLED = process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_DEBUG === "1";

const AUTH_INPUT_SELECTOR_GROUPS = Object.freeze({
  email: [
    "input[type='email']",
    "input[name='email']",
    "input[name='identifier']",
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
});

const GOOGLE_INTERACTIVE_CHALLENGE_PATTERNS = Object.freeze([
  /captcha/i,
  /verify you are human/i,
  /not a robot/i,
]);

const GOOGLE_ADDITIONAL_VERIFICATION_PATTERNS = Object.freeze([
  /verify it'?s you/i,
  /confirm it'?s you/i,
  /check your phone/i,
  /tap yes/i,
  /use your phone/i,
  /choose how you want to sign in/i,
  /get a verification code/i,
  /enter a phone number/i,
  /2-step verification/i,
]);

const GOOGLE_ERROR_PATTERNS = Object.freeze([
  /couldn'?t sign you in/i,
  /wrong password/i,
  /incorrect password/i,
  /couldn'?t find your google account/i,
  /account disabled/i,
  /something went wrong/i,
]);

const CLAUDE_LOGIN_ERROR_PATTERNS = Object.freeze([
  /there was an error logging you in/i,
  /problem persists contact support/i,
]);

const parseBool = (value?: string): boolean =>
  ["1", "true", "yes", "on"].includes(String(value ?? "").trim().toLowerCase());

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const readString = (value: unknown): string => (typeof value === "string" ? value.trim() : "");

const escapeRegExp = (value: string): string => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const normalizeUiText = (value: unknown): string => String(value ?? "").replace(/\s+/g, " ").trim().toLowerCase();

const waitMs = async (ms: number): Promise<void> => {
  await new Promise((resolve) => {
    setTimeout(resolve, Math.max(0, ms));
  });
};

const sanitizeAuthUrl = (raw: string): string => {
  const text = readString(raw);
  if (!text) return "";
  try {
    const parsed = new URL(text);
    return `${parsed.protocol}//${parsed.host}${parsed.pathname || "/"}`;
  } catch {
    return text.split("#", 1)[0]?.split("?", 1)[0]?.trim() ?? "";
  }
};

const collectAuthStateText = (state: VisibleDomState): string =>
  [state.title, state.bodyText, ...state.buttons].map((value) => readString(value)).join(" ").toLowerCase();

const stateMentions = (state: VisibleDomState, pattern: RegExp): boolean => pattern.test(collectAuthStateText(state));

const readStateHost = (state: VisibleDomState): string => {
  try {
    return new URL(readString(state.url)).host.toLowerCase();
  } catch {
    return "";
  }
};

const readBrowserAuthTimeoutMs = (fallbackMs: number): number => {
  const raw = Number(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_TIMEOUT_MS ?? "");
  if (!Number.isFinite(raw) || raw <= 0) return fallbackMs;
  return Math.floor(raw);
};

const readBrowserAuthTypingDelayMs = (): number => {
  const raw = Number(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_KEY_DELAY_MS ?? "");
  if (!Number.isFinite(raw) || raw <= 0) return 80;
  return Math.floor(raw);
};

const readClaudePostGoogleSettleMs = (): number => {
  const raw = Number(process.env.CTX_E2E_CLAUDE_POST_GOOGLE_SETTLE_MS ?? "");
  if (!Number.isFinite(raw) || raw <= 0) return DEFAULT_CLAUDE_POST_GOOGLE_SETTLE_MS;
  return Math.floor(raw);
};

const logBrowserAuthDebug = (providerLabel: string, message: string, details?: unknown): void => {
  if (!BROWSER_AUTH_DEBUG_ENABLED) return;
  if (details === undefined) {
    console.log(`[provider-browser-auth:${providerLabel}] ${message}`);
    return;
  }
  console.log(`[provider-browser-auth:${providerLabel}] ${message}: ${JSON.stringify(details)}`);
};

const chromeUserDataRoot = (): string =>
  path.join(process.env.HOME ?? "", "Library", "Application Support", "Google", "Chrome");

const findLocalChromeProfileForEmail = (email: string): string | null => {
  const normalizedEmail = readString(email).toLowerCase();
  if (!normalizedEmail) return null;
  const baseDir = chromeUserDataRoot();
  for (const candidate of ["Profile 1", "Profile 5", "Default", "Profile 2", "Profile 3", "Profile 4", "Profile 6"]) {
    const preferencesPath = path.join(baseDir, candidate, "Preferences");
    if (!existsSync(preferencesPath)) continue;
    try {
      const contents = readFileSync(preferencesPath, "utf8").toLowerCase();
      if (contents.includes(normalizedEmail)) {
        return candidate;
      }
    } catch {
      continue;
    }
  }
  return null;
};

const seedLocalChromeProfile = (userDataDir: string, email: string): string => {
  const baseDir = chromeUserDataRoot();
  const profileDirectory = findLocalChromeProfileForEmail(email);
  if (!profileDirectory) {
    throw new Error(`no local Chrome profile matched ${email}; set up a signed-in Chrome profile for the shared Google account first`);
  }

  const localStatePath = path.join(baseDir, "Local State");
  const firstRunPath = path.join(baseDir, "First Run");
  const sourceProfileDir = path.join(baseDir, profileDirectory);
  if (!existsSync(localStatePath) || !existsSync(sourceProfileDir)) {
    throw new Error(`local Chrome profile seed is incomplete for ${profileDirectory}`);
  }

  cpSync(localStatePath, path.join(userDataDir, "Local State"));
  if (existsSync(firstRunPath)) {
    cpSync(firstRunPath, path.join(userDataDir, "First Run"));
  } else {
    writeFileSync(path.join(userDataDir, "First Run"), "");
  }
  cpSync(sourceProfileDir, path.join(userDataDir, profileDirectory), { recursive: true });
  return profileDirectory;
};

const reserveTcpPort = async (): Promise<number> =>
  await new Promise<number>((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("failed to reserve local TCP port")));
        return;
      }
      const { port } = address;
      server.close((error) => {
        if (error) {
          reject(error);
          return;
        }
        resolve(port);
      });
    });
  });

const waitForChromeDevtools = async (port: number, timeoutMs: number): Promise<void> => {
  const deadline = Date.now() + timeoutMs;
  const url = `http://127.0.0.1:${port}/json/version`;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {
      // keep polling until timeout
    }
    await waitMs(250);
  }
  throw new Error(`timed out waiting for Chrome DevTools on port ${port}`);
};

const waitForChildProcessExit = async (child: ChildProcess, timeoutMs: number): Promise<void> =>
  await new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) {
      resolve();
      return;
    }
    let settled = false;
    const finish = () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      child.off("close", finish);
      child.off("exit", finish);
      resolve();
    };
    const timer = setTimeout(finish, Math.max(0, timeoutMs));
    child.once("close", finish);
    child.once("exit", finish);
  });

const terminateChildProcess = async (child: ChildProcess, timeoutMs: number): Promise<void> => {
  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGTERM");
  }
  await waitForChildProcessExit(child, timeoutMs);
};

const removeDirWithRetries = async (targetDir: string, attempts = 5): Promise<void> => {
  let lastError: unknown = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      rmSync(targetDir, { force: true, recursive: true });
      return;
    } catch (error) {
      lastError = error;
      if (!(error instanceof Error) || !/ENOTEMPTY|EBUSY/i.test(error.message) || attempt === attempts - 1) {
        throw error;
      }
      await waitMs(250 * (attempt + 1));
    }
  }
  if (lastError) {
    throw lastError;
  }
};

const launchLocalChromeProfileContext = async (
  userDataDir: string,
  profileDirectory: string,
  useStealthishMode: boolean,
): Promise<{ context: BrowserContext; browserProcess: ChildProcess }> => {
  const chromeExecutable = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
  if (!existsSync(chromeExecutable)) {
    throw new Error(`Google Chrome executable not found at ${chromeExecutable}`);
  }
  const devtoolsPort = await reserveTcpPort();
  const args = [
    `--user-data-dir=${userDataDir}`,
    `--profile-directory=${profileDirectory}`,
    `--remote-debugging-port=${devtoolsPort}`,
    `--window-size=${DEFAULT_AUTH_WINDOW_SIZE.width},${DEFAULT_AUTH_WINDOW_SIZE.height}`,
    "--new-window",
    ...(useStealthishMode ? ["--disable-blink-features=AutomationControlled"] : []),
    "about:blank",
  ];
  const browserProcess = spawn(chromeExecutable, args, {
    stdio: "ignore",
    detached: false,
  });
  await waitForChromeDevtools(devtoolsPort, 15_000);
  const browser = await chromium.connectOverCDP(`http://127.0.0.1:${devtoolsPort}`);
  const context = browser.contexts()[0];
  if (!context) {
    await browser.close().catch(() => {});
    browserProcess.kill("SIGTERM");
    throw new Error("Chrome CDP connection did not expose a browser context");
  }
  return { context, browserProcess };
};

const applyStealthishInitScript = async (context: BrowserContext): Promise<void> => {
  await context.addInitScript(() => {
    Object.defineProperty(navigator, "webdriver", {
      configurable: true,
      get: () => undefined,
    });

    const chromeValue = (window as Window & { chrome?: object }).chrome;
    if (!chromeValue) {
      Object.defineProperty(window, "chrome", {
        configurable: true,
        value: { runtime: {} },
      });
    }

    if (navigator.permissions?.query) {
      const originalQuery = navigator.permissions.query.bind(navigator.permissions);
      navigator.permissions.query = ((parameters: PermissionDescriptor) => {
        if (parameters.name === "notifications") {
          return Promise.resolve({
            name: "notifications",
            onchange: null,
            state: Notification.permission,
            addEventListener() {},
            dispatchEvent() {
              return true;
            },
            removeEventListener() {},
          } as PermissionStatus);
        }
        return originalQuery(parameters);
      }) as typeof navigator.permissions.query;
    }
  });
};

export const createClaudeBrowserAuthContext = async (
  pageContext: BrowserContext,
): Promise<BrowserAuthContextHandle> => {
  const usePersistentContext = parseBool(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_PERSISTENT);
  const useStealthishMode = parseBool(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_STEALTH);

  if (!usePersistentContext) {
    if (useStealthishMode) {
      await applyStealthishInitScript(pageContext);
    }
    return {
      context: pageContext,
      dispose: async () => {},
    };
  }

  const userDataDir = mkdtempSync(path.join(tmpdir(), "ctx-claude-oauth-profile-"));
  const keepProfile = parseBool(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_KEEP_PROFILE);
  const preferredChannel = readString(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_CHANNEL)
    || (process.platform === "darwin" ? "chrome" : "");
  const useLocalGoogleProfile = parseBool(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_USE_LOCAL_GOOGLE_PROFILE);
  const googleEmail = readString(process.env.GOOGLE_TEST_EMAIL);
  const profileDirectory = useLocalGoogleProfile
    ? seedLocalChromeProfile(userDataDir, googleEmail)
    : "Default";
  if (useLocalGoogleProfile) {
    const { context, browserProcess } = await launchLocalChromeProfileContext(
      userDataDir,
      profileDirectory,
      useStealthishMode,
    );
    if (useStealthishMode) {
      await applyStealthishInitScript(context);
    }
    return {
      context,
      dispose: async () => {
        await context.browser()?.close().catch(() => {});
        await terminateChildProcess(browserProcess, 5_000);
        if (!keepProfile) {
          await removeDirWithRetries(userDataDir);
        }
      },
    };
  }

  const persistentContext = await chromium.launchPersistentContext(userDataDir, {
    channel: preferredChannel || undefined,
    headless: parseBool(process.env.CTX_E2E_PROVIDER_BROWSER_AUTH_HEADLESS),
    viewport: DEFAULT_AUTH_WINDOW_SIZE,
    screen: DEFAULT_AUTH_WINDOW_SIZE,
    args: [
      ...(useStealthishMode ? ["--disable-blink-features=AutomationControlled"] : []),
      `--profile-directory=${profileDirectory}`,
      `--window-size=${DEFAULT_AUTH_WINDOW_SIZE.width},${DEFAULT_AUTH_WINDOW_SIZE.height}`,
    ],
    ...(useStealthishMode ? { ignoreDefaultArgs: ["--enable-automation"] } : {}),
  });
  if (useStealthishMode) {
    await applyStealthishInitScript(persistentContext);
  }

  return {
    context: persistentContext,
    dispose: async () => {
      await persistentContext.close().catch(() => {});
      if (!keepProfile) {
        rmSync(userDataDir, { force: true, recursive: true });
      }
    },
  };
};

const summarizeState = (state: VisibleDomState) => ({
  url: sanitizeAuthUrl(state.url),
  title: state.title,
  buttons: state.buttons.slice(0, 8),
  inputs: state.inputs.map((entry) => ({
    type: entry.type,
    name: entry.name,
    autocomplete: entry.autocomplete,
    label: entry.label,
  })),
});

const readVisibleDomState = async (page: Page): Promise<VisibleDomState> =>
  page.evaluate((textLimit) => {
    const normalize = (value: unknown) => String(value ?? "").replace(/\s+/g, " ").trim();
    const isVisible = (element: Element | null): element is HTMLElement => {
      if (!(element instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(element);
      if (!style || style.display === "none" || style.visibility === "hidden") return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const labelTextFor = (element: Element): string => {
      const values: string[] = [];
      if (element instanceof HTMLElement) {
        values.push(element.getAttribute("aria-label") || "");
        const labelledBy = element.getAttribute("aria-labelledby");
        if (labelledBy) {
          for (const id of labelledBy.split(/\s+/g)) {
            const ref = document.getElementById(id);
            if (ref) values.push(ref.textContent || "");
          }
        }
        values.push(element.getAttribute("placeholder") || "");
        if ("labels" in element) {
          for (const label of Array.from((element as HTMLInputElement).labels || [])) {
            values.push(label.textContent || "");
          }
        }
        const parentLabel = element.closest("label");
        if (parentLabel) values.push(parentLabel.textContent || "");
      }
      return normalize(values.join(" "));
    };
    const inputs = Array.from(document.querySelectorAll("input, textarea"))
      .filter((element) => isVisible(element))
      .map((element) => ({
        tag: element.tagName.toLowerCase(),
        type: normalize(element.getAttribute("type") || "").toLowerCase(),
        name: normalize(element.getAttribute("name") || "").toLowerCase(),
        autocomplete: normalize(element.getAttribute("autocomplete") || "").toLowerCase(),
        inputmode: normalize(element.getAttribute("inputmode") || "").toLowerCase(),
        label: labelTextFor(element).toLowerCase(),
        maxLength: Number((element as HTMLInputElement).maxLength || 0),
      }));
    const buttons = Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], a"))
      .filter((element) => isVisible(element))
      .map((element) => normalize(element.textContent || element.getAttribute("value") || element.getAttribute("aria-label")))
      .filter(Boolean)
      .map((value) => value.toLowerCase());
    return {
      url: String(window.location.href || ""),
      title: normalize(document.title || "").toLowerCase(),
      bodyText: normalize(document.body?.innerText || "").slice(0, Math.max(0, Number(textLimit) || 0)).toLowerCase(),
      inputs,
      buttons,
    };
  }, TEXT_LIMIT);

const isTransientNavigationStateReadError = (error: unknown): boolean => {
  const message = error instanceof Error ? error.message : String(error);
  return /Execution context was destroyed/i.test(message)
    || /Cannot find context with specified id/i.test(message);
};

const isClosedPageReadError = (error: unknown): boolean => {
  const message = error instanceof Error ? error.message : String(error);
  return /Target page, context or browser has been closed/i.test(message)
    || /has been closed/i.test(message);
};

const resolveReplacementPage = (page: Page): Page | null => {
  const candidates = page.context().pages().filter((candidate) => !candidate.isClosed());
  if (candidates.length === 0) return null;
  return candidates.at(-1) ?? null;
};

const resolvePreferredAuthPage = (page: Page): Page | null => {
  const candidates = page.context().pages().filter((candidate) => !candidate.isClosed());
  const googlePage = candidates.find((candidate) => {
    try {
      return candidate !== page && candidate.url().includes("accounts.google.");
    } catch {
      return false;
    }
  });
  return googlePage ?? resolveReplacementPage(page);
};

const typeIntoBrowserAuthFieldLikeHuman = async (locator: Locator, value: string): Promise<void> => {
  const normalizedValue = readString(value);
  if (!normalizedValue) return;
  const currentValue = await locator.inputValue().catch(() => "");
  if (currentValue === normalizedValue) return;

  await locator.click({ timeout: 5_000 });
  if (currentValue) {
    await locator.press(process.platform === "darwin" ? "Meta+A" : "Control+A").catch(() => {});
    await locator.press("Backspace").catch(() => {});
  }
  await locator.pressSequentially(normalizedValue, { delay: readBrowserAuthTypingDelayMs() });
  const typedValue = await locator.inputValue().catch(() => "");
  if (typedValue === normalizedValue) {
    return;
  }

  await locator.fill("").catch(() => {});
  await locator.fill(normalizedValue);
  const filledValue = await locator.inputValue().catch(() => "");
  if (filledValue !== normalizedValue) {
    throw new Error(`browser auth field value mismatch after fill fallback: expected ${normalizedValue.length} chars, got ${filledValue.length}`);
  }
};

const tryLocatorClick = async (locator: Pick<Locator, "click">, timeout: number): Promise<boolean> => {
  try {
    await locator.click({ timeout });
    return true;
  } catch {
    return false;
  }
};

const clickVisibleTextAction = async (page: Page, candidates: string[]): Promise<boolean> => {
  for (const label of candidates) {
    const pattern = new RegExp(escapeRegExp(label), "i");
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
        if (await tryLocatorClick(candidate, 3_000)) {
          return true;
        }
      }
    }
  }

  return page.evaluate((texts) => {
    const normalize = (value: unknown) => String(value ?? "").replace(/\s+/g, " ").trim().toLowerCase();
    const wanted = texts.map((entry) => normalize(entry)).filter(Boolean);
    const isVisible = (element: Element | null): element is HTMLElement => {
      if (!(element instanceof HTMLElement)) return false;
      const style = window.getComputedStyle(element);
      if (!style || style.display === "none" || style.visibility === "hidden") return false;
      const rect = element.getBoundingClientRect();
      return rect.width > 0 && rect.height > 0;
    };
    const elements = Array.from(document.querySelectorAll("button, [role='button'], input[type='submit'], a, div, span"));
    for (const element of elements) {
      if (!isVisible(element)) continue;
      const text = normalize(element.textContent || element.getAttribute("value") || element.getAttribute("aria-label"));
      if (!text || !wanted.some((wantedText) => text.includes(wantedText))) continue;
      const target = element.closest("button, [role='button'], a, label, div, span") || element;
      if (!(target instanceof HTMLElement)) continue;
      target.click();
      return true;
    }
    return false;
  }, candidates);
};

const clickVisibleTextActionAndObserveNextPage = async (
  page: Page,
  candidates: string[],
  timeoutMs = 5_000,
): Promise<Page | null> => {
  const existingPages = new Set(page.context().pages());
  const popupPromise = page.waitForEvent("popup", { timeout: timeoutMs }).catch(() => null);
  const contextPagePromise = page.context().waitForEvent("page", { timeout: timeoutMs }).catch(() => null);
  const clicked = await clickVisibleTextAction(page, candidates);
  if (!clicked) return null;
  const [popupPage, contextPage] = await Promise.all([popupPromise, contextPagePromise]);
  const nextPage = popupPage
    || contextPage
    || page.context().pages().find((candidate) => !existingPages.has(candidate))
    || page;
  await nextPage.waitForLoadState("domcontentloaded", { timeout: timeoutMs }).catch(() => {});
  return nextPage;
};

const clickGoogleAccountChooserSelection = async (page: Page, email: string): Promise<boolean> => {
  const footerPattern = /privacy|terms|help|create account|forgot email|use another account/i;
  const normalizedEmail = normalizeUiText(email);
  const candidateLocators: Locator[] = [
    page.locator("[data-email]:visible"),
    page.locator("[data-identifier]:visible"),
    page.locator("[role='link']:visible"),
    page.locator("[role='button']:visible"),
    page.locator("li:visible"),
    page.locator("div[tabindex]:visible"),
    page.locator("div[jscontroller]:visible"),
  ];

  const clickMatchingCandidate = async (preferEmail: boolean): Promise<boolean> => {
    for (const locator of candidateLocators) {
      const count = await locator.count().catch(() => 0);
      for (let index = 0; index < count; index += 1) {
        const candidate = locator.nth(index);
        if (!(await candidate.isVisible().catch(() => false))) continue;
        const text = normalizeUiText(await candidate.textContent().catch(() => ""));
        if (!text || footerPattern.test(text)) continue;
        if (preferEmail && normalizedEmail && !text.includes(normalizedEmail)) continue;
        if (await tryLocatorClick(candidate, 3_000)) {
          return true;
        }
      }
    }
    return false;
  };

  return await clickMatchingCandidate(true) || await clickMatchingCandidate(false);
};

const fillBrowserAuthField = async (
  page: Page,
  kind: keyof typeof AUTH_INPUT_SELECTOR_GROUPS,
  value: string,
): Promise<boolean> => {
  const fillLocator = async (locator: Locator): Promise<boolean> => {
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      if (!(await candidate.isVisible().catch(() => false))) continue;
      const metadata = await candidate.evaluate((element) => ({
        ariaHidden: element.getAttribute("aria-hidden") === "true",
        tabIndex: element instanceof HTMLElement ? element.tabIndex : 0,
      })).catch(() => null);
      if (metadata?.ariaHidden || (metadata?.tabIndex ?? 0) < 0) continue;
      if (!(await candidate.isEditable().catch(() => false))) continue;
      try {
        await typeIntoBrowserAuthFieldLikeHuman(candidate, value);
        return true;
      } catch {
        continue;
      }
    }
    return false;
  };

  const preferredLocators: Locator[] = kind === "email"
    ? [
      page.getByLabel(/email or phone|email/i),
      page.getByPlaceholder(/email or phone|email/i),
      page.locator("input[name='identifier']:visible"),
    ]
    : [
      page.getByLabel(/password/i),
      page.getByPlaceholder(/password/i),
    ];

  for (const locator of preferredLocators) {
    if (await fillLocator(locator)) {
      return true;
    }
  }

  for (const selector of AUTH_INPUT_SELECTOR_GROUPS[kind]) {
    if (await fillLocator(page.locator(`${selector}:visible`))) {
      return true;
    }
  }

  return false;
};

const submitFocusedAuthFieldWithEnter = async (
  page: Page,
  kind: keyof typeof AUTH_INPUT_SELECTOR_GROUPS,
): Promise<boolean> => {
  for (const selector of AUTH_INPUT_SELECTOR_GROUPS[kind]) {
    const locator = page.locator(`${selector}:visible`);
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      if (!(await candidate.isVisible().catch(() => false))) continue;
      if (!(await candidate.isEditable().catch(() => false))) continue;
      try {
        await candidate.click({ timeout: 2_000 });
        await candidate.press("Enter", { timeout: 2_000 });
        return true;
      } catch {
        continue;
      }
    }
  }
  return false;
};

const submitVisibleAuthStep = async (page: Page, labels: string[]): Promise<boolean> => {
  for (const label of labels) {
    const pattern = new RegExp(`^\\s*${escapeRegExp(label)}\\s*$`, "i");
    for (const locator of [
      page.getByRole("button", { name: pattern }),
      page.getByRole("link", { name: pattern }),
      page.getByLabel(pattern),
    ]) {
      const count = await locator.count().catch(() => 0);
      for (let index = 0; index < count; index += 1) {
        const candidate = locator.nth(index);
        if (!(await candidate.isVisible().catch(() => false))) continue;
        if (await tryLocatorClick(candidate, 3_000)) {
          return true;
        }
      }
    }
  }
  if (await clickVisibleTextAction(page, labels)) {
    return true;
  }
  await page.keyboard.press("Enter").catch(() => {});
  return true;
};

const submitGoogleSignInStep = async (
  page: Page,
  step: "identifier" | "password",
): Promise<boolean> => {
  const submittedForm = await page.evaluate((currentStep) => {
    const selector = currentStep === "identifier" ? "input[name='identifier']" : "input[type='password']";
    const input = document.querySelector(selector);
    if (!(input instanceof HTMLElement)) return false;
    const form = input.closest("form");
    if (!(form instanceof HTMLFormElement)) return false;
    if (typeof form.requestSubmit === "function") {
      form.requestSubmit();
      return true;
    }
    form.submit();
    return true;
  }, step).catch(() => false);
  if (submittedForm) {
    return true;
  }

  const selectors = step === "identifier"
    ? ["#identifierNext button", "#identifierNext", "div#identifierNext"]
    : ["#passwordNext button", "#passwordNext", "div#passwordNext"];
  for (const selector of selectors) {
    const locator = page.locator(`${selector}:visible`);
    const count = await locator.count().catch(() => 0);
    for (let index = 0; index < count; index += 1) {
      const candidate = locator.nth(index);
      if (!(await candidate.isVisible().catch(() => false))) continue;
      if (await tryLocatorClick(candidate, 3_000)) {
        return true;
      }
      const clicked = await candidate.evaluate((element) => {
        if (!(element instanceof HTMLElement)) return false;
        element.click();
        return true;
      }).catch(() => false);
      if (clicked) {
        return true;
      }
    }
  }
  return false;
};

const classifyGoogleChallenge = ({ state, hasOtpInput }: { state: VisibleDomState; hasOtpInput: boolean }) => {
  if (!readStateHost(state).includes("accounts.google.")) {
    return "";
  }
  if (GOOGLE_INTERACTIVE_CHALLENGE_PATTERNS.some((pattern) => stateMentions(state, pattern))) {
    return "google_interactive_challenge";
  }
  if (GOOGLE_ADDITIONAL_VERIFICATION_PATTERNS.some((pattern) => stateMentions(state, pattern))) {
    return "google_additional_verification_required";
  }
  if (hasOtpInput) {
    return "google_otp_required";
  }
  return "";
};

const classifyClaudeLoginState = ({
  state,
  clickedGoogleEntry,
}: {
  state: VisibleDomState;
  clickedGoogleEntry: boolean;
}): string => {
  if (!clickedGoogleEntry && stateMentions(state, /continue with google/i)) {
    return "";
  }
  const hasError = CLAUDE_LOGIN_ERROR_PATTERNS.some((pattern) => stateMentions(state, pattern));
  if (clickedGoogleEntry && hasError && !stateMentions(state, /continue with google/i)) {
    return "claude_login_error";
  }
  return "";
};

const startClaudeLogin = async (
  request: APIRequestContext,
  label?: string,
): Promise<{ loginId: string; authUrl: string }> => {
  const response = await request.post("/api/providers/claude-crp/accounts/login/start", {
    data: label ? { label } : {},
    timeout: 60_000,
  });
  if (!response.ok()) {
    const detail = await response.text().catch(() => "");
    throw new Error(`claude login start failed (${response.status()}): ${detail}`);
  }
  const payload = asRecord(await response.json() as ClaudeLoginStartResponse);
  const loginId = readString(payload.login_id);
  const authUrl = readString(payload.auth_url);
  if (!loginId) {
    throw new Error(`claude login start response missing login_id: ${JSON.stringify(payload)}`);
  }
  return { loginId, authUrl };
};

const getClaudeLogin = async (request: APIRequestContext, loginId: string): Promise<ClaudeLoginStatus> => {
  const response = await request.get(`/api/providers/claude-crp/accounts/login/${encodeURIComponent(loginId)}`, {
    timeout: 30_000,
  });
  if (!response.ok()) {
    throw new Error(`claude login status failed (${response.status()})`);
  }
  return await response.json() as ClaudeLoginStatus;
};

const waitForClaudeLoginAuthUrl = async (
  request: APIRequestContext,
  loginId: string,
  timeoutMs: number,
  pollMs: number,
): Promise<string> => {
  const startedAt = Date.now();
  let lastDetail = "pending";
  while (Date.now() - startedAt <= timeoutMs) {
    const status = asRecord(await getClaudeLogin(request, loginId));
    const authUrl = readString(status.auth_url);
    if (authUrl) return authUrl;
    const normalized = readString(status.status).toLowerCase();
    if (normalized === "failed" || normalized === "timeout") {
      throw new Error(`claude login ${normalized}: ${readString(status.error) || JSON.stringify(status)}`);
    }
    lastDetail = readString(status.error) || normalized || "pending";
    await waitMs(pollMs);
  }
  throw new Error(`timed out waiting for claude auth url: ${lastDetail}`);
};

const waitForClaudeLoginSuccess = async (
  request: APIRequestContext,
  loginId: string,
  timeoutMs: number,
  pollMs: number,
): Promise<string> => {
  const startedAt = Date.now();
  let lastDetail = "pending";
  while (Date.now() - startedAt <= timeoutMs) {
    const status = asRecord(await getClaudeLogin(request, loginId));
    const normalized = readString(status.status).toLowerCase();
    if (normalized === "success") {
      const accountId = readString(status.account_id);
      if (!accountId) {
        throw new Error(`claude login success missing account id: ${JSON.stringify(status)}`);
      }
      return accountId;
    }
    if (normalized === "failed" || normalized === "timeout") {
      throw new Error(`claude login ${normalized}: ${readString(status.error) || JSON.stringify(status)}`);
    }
    lastDetail = readString(status.error) || normalized || "pending";
    await waitMs(pollMs);
  }
  throw new Error(`timed out waiting for claude login success: ${lastDetail}`);
};

const completeClaudeLogin = async (
  request: APIRequestContext,
  loginId: string,
  callbackCode: string,
): Promise<void> => {
  const normalizedCode = readString(callbackCode);
  if (!normalizedCode) {
    throw new Error("claude callback code is empty");
  }
  const response = await request.post(`/api/providers/claude-crp/accounts/login/${encodeURIComponent(loginId)}`, {
    data: { callback_code: normalizedCode },
    timeout: 30_000,
  });
  if (!response.ok()) {
    const detail = await response.text().catch(() => "");
    throw new Error(`claude login completion failed (${response.status()}): ${detail}`);
  }
};

const isClaudeOauthCallbackUrl = (parsedUrl: URL): boolean => {
  const host = parsedUrl.hostname.toLowerCase();
  if (host !== "platform.claude.com" && host !== "console.anthropic.com") {
    return false;
  }
  return /^\/oauth\/code\/(callback|success)\b/i.test(parsedUrl.pathname);
};

export const extractClaudeCallbackCodeFromUrl = (rawUrl: string): string => {
  try {
    const parsed = new URL(rawUrl);
    if (!isClaudeOauthCallbackUrl(parsed)) {
      return "";
    }
    const directCode = readString(parsed.searchParams.get("code"));
    if (directCode && directCode.toLowerCase() !== "true") {
      return directCode;
    }
    const hash = parsed.hash.startsWith("#") ? parsed.hash.slice(1) : parsed.hash;
    if (hash) {
      const hashParams = new URLSearchParams(hash);
      const hashCode = readString(hashParams.get("code"));
      if (hashCode) {
        return hashCode;
      }
    }
  } catch {
    // fall through to empty result
  }
  return "";
};

const clearMacClipboard = (): void => {
  if (process.platform !== "darwin") return;
  execFileSync("pbcopy", { input: "" });
};

const readMacClipboardText = (): string => {
  if (process.platform !== "darwin") return "";
  return readString(execFileSync("pbpaste", { encoding: "utf8" }));
};

const selectSubscriptionSource = async (request: APIRequestContext, providerId: string): Promise<void> => {
  const response = await request.post(`/api/providers/${encodeURIComponent(providerId)}/harness_config/select`, {
    data: {
      source_kind: "subscription",
      endpoint_id: null,
    },
    timeout: 30_000,
  });
  if (!response.ok()) {
    const detail = await response.text().catch(() => "");
    throw new Error(`failed to select subscription source for ${providerId} (${response.status()}): ${detail}`);
  }
};

const driveGoogleBackedBrowserLoginWithCredentials = async ({
  page,
  authUrl,
  email,
  password,
  providerLabel,
  timeoutMs = DEFAULT_TIMEOUT_MS,
  pollMs = DEFAULT_POLL_MS,
  onState,
}: GoogleDriveOptions): Promise<Record<string, unknown>> => {
  const normalizedEmail = readString(email);
  const normalizedPassword = password;
  if (!normalizedEmail) throw new Error(`${providerLabel} Google email is required`);
  if (!normalizedPassword) throw new Error(`${providerLabel} Google password is required`);

  let activePage = page;
  await activePage.goto(authUrl, { waitUntil: "domcontentloaded", timeout: timeoutMs });
  const startedAt = Date.now();
  let lastState: VisibleDomState | null = null;
  let lastDebugSnapshot = "";
  const progress: DriveProgress = {
    clickedGoogleEntry: false,
    selectedAccount: false,
    usedEmail: false,
    usedPassword: false,
    grantedConsent: false,
  };

  while (Date.now() - startedAt <= timeoutMs) {
    const preferredPage = resolvePreferredAuthPage(activePage);
    if (preferredPage && preferredPage !== activePage) {
      activePage = preferredPage;
      logBrowserAuthDebug(providerLabel, "switched_preferred_auth_page", {
        url: sanitizeAuthUrl(activePage.url()),
      });
    }
    if (activePage.isClosed()) {
      const replacementPage = resolvePreferredAuthPage(activePage);
      if (replacementPage) {
        activePage = replacementPage;
        logBrowserAuthDebug(providerLabel, "switched_closed_page_context");
      } else {
        throw new Error(`${providerLabel} browser auth page closed before completion`);
      }
    }

    await activePage.waitForLoadState("domcontentloaded", { timeout: 5_000 }).catch(() => {});
    let state: VisibleDomState;
    try {
      state = await readVisibleDomState(activePage);
    } catch (error) {
      if (isTransientNavigationStateReadError(error)) {
        await waitMs(pollMs);
        continue;
      }
      if (isClosedPageReadError(error)) {
        const replacementPage = resolvePreferredAuthPage(activePage);
        if (replacementPage) {
          activePage = replacementPage;
          logBrowserAuthDebug(providerLabel, "recovered_closed_page_context");
          await waitMs(pollMs);
          continue;
        }
      }
      throw error;
    }

    lastState = state;
    const debugSnapshot = JSON.stringify({
      url: sanitizeAuthUrl(state.url),
      title: state.title,
      inputs: state.inputs.map((entry) => ({ type: entry.type, name: entry.name, label: entry.label })),
      buttons: state.buttons.slice(0, 8),
      progress,
    });
    if (debugSnapshot !== lastDebugSnapshot) {
      logBrowserAuthDebug(providerLabel, "state", JSON.parse(debugSnapshot));
      lastDebugSnapshot = debugSnapshot;
    }

    const inputs = Array.isArray(state.inputs) ? state.inputs : [];
    const hasEmailInput = inputs.some((entry) =>
      entry.type === "email" || entry.autocomplete === "email" || entry.name.includes("email") || entry.label.includes("email"));
    const hasPasswordInput = inputs.some((entry) =>
      entry.type === "password" || entry.autocomplete.includes("password") || entry.label.includes("password"));
    const hasOtpInput = inputs.some((entry) =>
      entry.autocomplete === "one-time-code"
        || entry.inputmode === "numeric"
        || entry.name.includes("otp")
        || entry.name.includes("code")
        || entry.label.includes("code")
        || entry.label.includes("verification"));
    const isGoogleHost = readStateHost(state).includes("accounts.google.");

    if (typeof onState === "function") {
      const outcome = await onState({
        page: activePage,
        state,
        progress: { ...progress },
        hasEmailInput,
        hasPasswordInput,
        hasOtpInput,
        isGoogleHost,
      });
      if (outcome && "done" in outcome && outcome.done) {
        return {
          providerLabel,
          progress: { ...progress },
          ...outcome.result,
        };
      }
      if (outcome && "handled" in outcome && outcome.handled) {
        if (outcome.page && !outcome.page.isClosed()) {
          activePage = outcome.page;
        }
        await waitMs(pollMs);
        continue;
      }
    }

    const blockedReason = classifyGoogleChallenge({ state, hasOtpInput });
    if (blockedReason) {
      throw new Error(`${providerLabel} Google OAuth blocked by ${blockedReason}: ${JSON.stringify(summarizeState(state))}`);
    }

    if (!isGoogleHost && !progress.clickedGoogleEntry) {
      const nextPage = await clickVisibleTextActionAndObserveNextPage(activePage, [
        "continue with google",
        "sign in with google",
        "continue to google",
      ]);
      if (nextPage) {
        progress.clickedGoogleEntry = true;
        activePage = nextPage;
        logBrowserAuthDebug(providerLabel, "clicked_google_entry", summarizeState(state));
        await waitMs(pollMs);
        continue;
      }
    }

    if (isGoogleHost && GOOGLE_ERROR_PATTERNS.some((pattern) => stateMentions(state, pattern))) {
      throw new Error(`${providerLabel} Google auth page reported an error: ${JSON.stringify(summarizeState(state))}`);
    }

    if (hasEmailInput && (isGoogleHost || !progress.clickedGoogleEntry)) {
      if (!progress.usedEmail) {
        const filled = await fillBrowserAuthField(activePage, "email", normalizedEmail);
        if (!filled) {
          throw new Error(`failed to fill ${providerLabel} Google email`);
        }
        progress.usedEmail = true;
        logBrowserAuthDebug(providerLabel, "submitted_email", { email: normalizedEmail });
      }
      if (isGoogleHost) {
        const identifierValue = await activePage.locator("input[name='identifier']").inputValue().catch(() => "");
        logBrowserAuthDebug(providerLabel, "google_identifier_value", {
          value: identifierValue,
          length: identifierValue.length,
        });
        if (await submitFocusedAuthFieldWithEnter(activePage, "email")) {
          logBrowserAuthDebug(providerLabel, "submitted_identifier_with_enter");
          await waitMs(pollMs);
          continue;
        }
        await activePage.keyboard.press("Tab").catch(() => {});
      }
      if (isGoogleHost && await submitGoogleSignInStep(activePage, "identifier")) {
        logBrowserAuthDebug(providerLabel, "submitted_identifier_step");
        await waitMs(pollMs);
        continue;
      }
      await submitVisibleAuthStep(activePage, ["next", "continue", "sign in"]);
      await waitMs(pollMs);
      continue;
    }

    if (hasPasswordInput && (isGoogleHost || !progress.clickedGoogleEntry)) {
      if (!progress.usedPassword) {
        const filled = await fillBrowserAuthField(activePage, "password", normalizedPassword);
        if (!filled) {
          throw new Error(`failed to fill ${providerLabel} Google password`);
        }
        progress.usedPassword = true;
        logBrowserAuthDebug(providerLabel, "submitted_password");
      }
      if (isGoogleHost) {
        const passwordValue = await activePage.locator("input[type='password']").inputValue().catch(() => "");
        logBrowserAuthDebug(providerLabel, "google_password_value", {
          length: passwordValue.length,
        });
        if (await submitFocusedAuthFieldWithEnter(activePage, "password")) {
          logBrowserAuthDebug(providerLabel, "submitted_password_with_enter");
          await waitMs(pollMs);
          continue;
        }
        await activePage.keyboard.press("Tab").catch(() => {});
      }
      if (isGoogleHost && await submitGoogleSignInStep(activePage, "password")) {
        logBrowserAuthDebug(providerLabel, "submitted_password_step");
        await waitMs(pollMs);
        continue;
      }
      await submitVisibleAuthStep(activePage, ["next", "continue", "sign in"]);
      await waitMs(pollMs);
      continue;
    }

    if (isGoogleHost && /\/accountchooser\b/i.test(state.url) && !hasEmailInput && !hasPasswordInput) {
      const selected = await clickGoogleAccountChooserSelection(activePage, normalizedEmail);
      if (selected) {
        progress.selectedAccount = true;
        logBrowserAuthDebug(providerLabel, "selected_google_account_chooser_entry");
        await waitMs(pollMs);
        continue;
      }
    }

    if (isGoogleHost && stateMentions(state, /allow|continue|accept|agree|continue as|sign in/i)) {
      const granted = await submitVisibleAuthStep(activePage, ["allow", "continue", "accept", "agree", "continue as", "sign in"]);
      if (granted) {
        progress.grantedConsent = true;
        logBrowserAuthDebug(providerLabel, "granted_consent", summarizeState(state));
        await waitMs(pollMs);
        continue;
      }
    }

    if (!hasEmailInput && !hasPasswordInput) {
      const canSelectKnownAccount = stateMentions(state, new RegExp(escapeRegExp(normalizedEmail), "i"));
      if (canSelectKnownAccount) {
        const choseExistingAccount = await clickVisibleTextAction(activePage, [normalizedEmail]);
        if (choseExistingAccount) {
          progress.selectedAccount = true;
          logBrowserAuthDebug(providerLabel, "selected_existing_account", { email: normalizedEmail });
          await waitMs(pollMs);
          continue;
        }
      }
    }

    await waitMs(pollMs);
  }

  throw new Error(`timed out driving ${providerLabel} Google OAuth browser flow: ${JSON.stringify(summarizeState(lastState ?? {
    url: "",
    title: "",
    bodyText: "",
    inputs: [],
    buttons: [],
  }))}`);
};

export async function completeClaudeOauthWithGoogleBrowserCredentials(
  opts: ClaudeBrowserOauthOptions,
): Promise<BrowserAuthSuccess> {
  const timeoutMs = readBrowserAuthTimeoutMs(opts.timeoutMs ?? DEFAULT_TIMEOUT_MS);
  const pollMs = opts.pollMs ?? DEFAULT_POLL_MS;
  const claudePostGoogleSettleMs = readClaudePostGoogleSettleMs();
  let firstPostGoogleClaudePageAt: number | null = null;
  let callbackCodeSubmitted = false;
  let callbackSubmittedAt = 0;
  const page = await opts.context.newPage();
  try {
    const login = await startClaudeLogin(opts.request, opts.label);
    const authUrl = login.authUrl || await waitForClaudeLoginAuthUrl(opts.request, login.loginId, timeoutMs, pollMs);

    await driveGoogleBackedBrowserLoginWithCredentials({
      page,
      authUrl,
      email: opts.email,
      password: opts.password,
      providerLabel: "claude",
      timeoutMs,
      pollMs,
      onState: async ({ page: activePage, state, progress }) => {
        const isClaudeHost = readStateHost(state).includes("claude.ai");
        const isClaudeLoginShell = isClaudeHost
          && /\/login\b/i.test(state.url)
          && stateMentions(state, /continue with google|continue with email|continue with sso/i);
        const claudeState = classifyClaudeLoginState({
          state,
          clickedGoogleEntry: progress.clickedGoogleEntry,
        });
        if (claudeState) {
          throw new Error(`claude browser login blocked by ${claudeState}: ${JSON.stringify(summarizeState(state))}`);
        }
        if (progress.grantedConsent && isClaudeLoginShell) {
          const now = Date.now();
          firstPostGoogleClaudePageAt ??= now;
          const elapsedMs = now - firstPostGoogleClaudePageAt;
          if (elapsedMs < claudePostGoogleSettleMs) {
            logBrowserAuthDebug("claude", "waiting_for_post_google_session_settle", {
              elapsedMs,
              settleMs: claudePostGoogleSettleMs,
              state: summarizeState(state),
            });
            return { handled: true };
          }
        } else if (!isClaudeLoginShell) {
          firstPostGoogleClaudePageAt = null;
        }
        if (/selectAccount=true/i.test(state.url) && stateMentions(state, /continue with google/i)) {
          const nextPage = await clickVisibleTextActionAndObserveNextPage(activePage, [
            "continue with google",
            "sign in with google",
            "continue to google",
          ]);
          if (!nextPage) {
            throw new Error(`Claude select-account page did not expose a usable Google continuation: ${JSON.stringify(summarizeState(state))}`);
          }
          return {
            handled: true,
            page: nextPage,
          };
        }
        if (isClaudeHost && /\/oauth\/authorize\b/i.test(state.url) && stateMentions(state, /authorize|decline|switch account/i)) {
          const approved = await clickVisibleTextAction(activePage, ["authorize"]);
          if (!approved) {
            throw new Error(`Claude authorize page did not expose a usable approve CTA: ${JSON.stringify(summarizeState(state))}`);
          }
          return { handled: true };
        }
        if (/\/cli_landing\b/i.test(state.url) || stateMentions(state, /try claude code/i)) {
          const clicked = await clickVisibleTextAction(activePage, ["try claude code"]);
          if (!clicked) {
            throw new Error(`Claude CLI landing page did not expose a usable CTA: ${JSON.stringify(summarizeState(state))}`);
          }
          return { handled: true };
        }
        if (progress.clickedGoogleEntry && !readStateHost(state).includes("accounts.google.")) {
          const status = asRecord(await getClaudeLogin(opts.request, login.loginId));
          const normalizedStatus = readString(status.status).toLowerCase();
          if (normalizedStatus === "success") {
            return {
              done: true,
              result: {
                status: "backend_success",
                finalUrl: sanitizeAuthUrl(state.url),
              },
            };
          }
          if (normalizedStatus === "failed" || normalizedStatus === "timeout") {
            throw new Error(`claude login ${normalizedStatus}: ${readString(status.error) || JSON.stringify(status)}`);
          }
        }
        const isClaudeCallbackPage = /\/oauth\/code\/(callback|success)\b/i.test(state.url)
          || ((readStateHost(state).includes("platform.claude.com")
              || readStateHost(state).includes("console.anthropic.com"))
            && stateMentions(state, /copy code|you can close this tab|login successful|connected to claude code/i));
        if (isClaudeCallbackPage) {
          if (callbackCodeSubmitted) {
            const status = asRecord(await getClaudeLogin(opts.request, login.loginId));
            const normalizedStatus = readString(status.status).toLowerCase();
            if (normalizedStatus === "success") {
              return {
                done: true,
                result: {
                  status: "backend_success",
                  finalUrl: sanitizeAuthUrl(state.url),
                },
              };
            }
            if (normalizedStatus === "failed" || normalizedStatus === "timeout") {
              throw new Error(`claude login ${normalizedStatus}: ${readString(status.error) || JSON.stringify(status)}`);
            }
            if (callbackSubmittedAt > 0 && Date.now() - callbackSubmittedAt >= 15_000) {
              throw new Error(`Claude callback code was submitted but login stayed pending: ${JSON.stringify({
                status: normalizedStatus || "pending",
                error: readString(status.error),
                state: summarizeState(state),
              })}`);
            }
            return { handled: true };
          }
          let callbackCode = extractClaudeCallbackCodeFromUrl(state.url);
          if (!callbackCode && stateMentions(state, /copy code/i) && process.platform === "darwin") {
            clearMacClipboard();
            const copied = await clickVisibleTextAction(activePage, ["copy code"]);
            if (copied) {
              await waitMs(500);
              callbackCode = readMacClipboardText();
            }
          }
          if (callbackCode) {
            logBrowserAuthDebug("claude", "captured_callback_code", { length: callbackCode.length });
            await completeClaudeLogin(opts.request, login.loginId, callbackCode);
            callbackCodeSubmitted = true;
            callbackSubmittedAt = Date.now();
            return { handled: true };
          }
          throw new Error(`Claude callback page did not expose an OAuth code: ${JSON.stringify(summarizeState(state))}`);
        }
        return null;
      },
    });
    const accountId = await waitForClaudeLoginSuccess(opts.request, login.loginId, timeoutMs, pollMs);
    await selectSubscriptionSource(opts.request, "claude-crp");
    return {
      loginId: login.loginId,
      accountId,
      authUrl,
    };
  } finally {
    await page.close().catch(() => {});
  }
}
