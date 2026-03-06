const fs = require("node:fs");
const path = require("node:path");

const { daemonJson } = require("./daemon.cjs");

const DEFAULT_POLL_MS = 750;
const DEFAULT_TIMEOUT_MS = 5 * 60_000;
const DEFAULT_URL_FALLBACK_GRACE_MS = 1200;

const PROVIDER_OAUTH_DESCRIPTORS = Object.freeze({
  codex: {
    providerId: "codex",
    startPath: "/api/providers/codex/accounts/login/start",
    statusPath: (loginId) => `/api/providers/codex/accounts/login/${encodeURIComponent(loginId)}`,
    accountsPath: "/api/providers/codex/accounts",
    loginIdField: "account_id",
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

const hasBrowserExecute = () => Boolean(global.browser && typeof global.browser.execute === "function");

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
  const result = await browser.execute(async (authUrl) => {
    try {
      const mod = await import("@tauri-apps/plugin-shell");
      await mod.open(authUrl);
      return { ok: true };
    } catch (error) {
      return { ok: false, error: String(error) };
    }
  }, target);
  if (!result || result.ok !== true) {
    throw new Error(readString(result?.error) || "failed to open external auth URL");
  }
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
    await openExternalUrlViaDesktop(href);
    recordStage(
      "open_auth_url",
      "opened auth URL through desktop shell path",
      { auth_url: href },
      { desktop_open_probe: probeInfo },
    );
  };

  return {
    startProviderLogin,
    awaitLoginUrl,
    awaitLoginTerminal,
    assertAccountActivated,
    openAuthUrl,
    installDesktopOpenExternalProbe,
    readDesktopOpenExternalProbe,
    resetDesktopOpenExternalProbe,
  };
};

module.exports = {
  PROVIDER_OAUTH_DESCRIPTORS,
  sanitizeAuthUrl,
  redactPayload,
  normalizeLoginStartPayload,
  normalizeLoginStatusPayload,
  normalizeAccountsPayload,
  isTerminalStatus,
  chooseObservedAuthUrl,
  createProviderOAuthHarness,
};
