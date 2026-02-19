export type DaemonConnection = {
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  runId: string | null;
  source?: string | null;
};

export type DaemonConnectionUpdate = {
  baseUrl?: string | null;
  wsBaseUrl?: string | null;
  authToken?: string | null;
  runId?: string | null;
  source?: string | null;
};

export type SetDaemonConnectionOptions = {
  persistBaseUrl?: boolean;
  clearPersistedBaseUrl?: boolean;
};

type StoredDaemonConnectionV1 = {
  v: 1;
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  source?: string | null;
};

type PersistedDaemonBaseV1 = {
  v: 1;
  baseUrl: string | null;
  wsBaseUrl: string | null;
};

type DaemonConnectionListener = (connection: DaemonConnection) => void;

const SESSION_CONNECTION_KEY = "ctxDaemonConnectionV1";
const LOCAL_PERSISTED_BASE_KEY = "ctxDaemonConnectionBaseV1";
const RUN_ID_KEY = "ctxRunId";

const listeners = new Set<DaemonConnectionListener>();

const isRecord = (value: unknown): value is Record<string, unknown> =>
  Boolean(value) && typeof value === "object";

type TauriGlobals = {
  __TAURI_INTERNALS__?: unknown;
  __TAURI__?: unknown;
};

const isDesktopWindow = (): boolean => {
  try {
    const g = globalThis as typeof globalThis & TauriGlobals;
    return Boolean(g.__TAURI_INTERNALS__ || g.__TAURI__);
  } catch {
    return false;
  }
};

const normalizeToken = (value: string | null | undefined): string | null => {
  if (value === null || value === undefined) return null;
  const trimmed = String(value).trim();
  return trimmed ? trimmed : null;
};

const trimTrailingSlash = (value: string): string => value.replace(/\/+$/, "");

const parseUrlSafely = (value: string): URL | null => {
  try {
    return new URL(value);
  } catch {
    return null;
  }
};

export const deriveDaemonWsBaseUrl = (baseUrl: string | null): string | null => {
  if (!baseUrl) return null;
  const trimmed = trimTrailingSlash(baseUrl);
  if (!trimmed) return null;
  if (trimmed.startsWith("ws://") || trimmed.startsWith("wss://")) return trimmed;
  if (trimmed.startsWith("https://")) return trimmed.replace(/^https:\/\//, "wss://");
  if (trimmed.startsWith("http://")) return trimmed.replace(/^http:\/\//, "ws://");
  return null;
};

export const normalizeDaemonBaseUrl = (value: string | null | undefined): string | null => {
  if (value === null || value === undefined) return null;
  const trimmed = String(value).trim();
  if (!trimmed) return null;

  // Compatibility: allow ws/wss values and normalize them to http/https.
  if (trimmed.startsWith("ws://")) {
    return trimmed.replace(/^ws:\/\//, "http://").replace(/\/+$/, "");
  }
  if (trimmed.startsWith("wss://")) {
    return trimmed.replace(/^wss:\/\//, "https://").replace(/\/+$/, "");
  }

  const parsed = parseUrlSafely(trimmed);
  if (!parsed) return null;
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
  return trimTrailingSlash(parsed.toString());
};

export const normalizeDaemonWsBaseUrl = (
  value: string | null | undefined,
  baseUrlForFallback?: string | null,
): string | null => {
  if (value === null || value === undefined) {
    return deriveDaemonWsBaseUrl(baseUrlForFallback ?? null);
  }
  const trimmed = String(value).trim();
  if (!trimmed) return deriveDaemonWsBaseUrl(baseUrlForFallback ?? null);

  const parsed = parseUrlSafely(trimmed);
  if (!parsed) {
    return deriveDaemonWsBaseUrl(baseUrlForFallback ?? null);
  }
  if (parsed.protocol === "ws:" || parsed.protocol === "wss:") {
    return trimTrailingSlash(parsed.toString());
  }
  if (parsed.protocol === "http:") {
    parsed.protocol = "ws:";
    return trimTrailingSlash(parsed.toString());
  }
  if (parsed.protocol === "https:") {
    parsed.protocol = "wss:";
    return trimTrailingSlash(parsed.toString());
  }
  return deriveDaemonWsBaseUrl(baseUrlForFallback ?? null);
};

const normalizeRunId = (value: string | null | undefined): string | null => {
  if (value === null || value === undefined) return null;
  const trimmed = String(value).trim();
  return trimmed ? trimmed : null;
};

const readSession = (key: string): string | null => {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
};

const writeSession = (key: string, value: string | null) => {
  try {
    if (value === null) {
      sessionStorage.removeItem(key);
    } else {
      sessionStorage.setItem(key, value);
    }
  } catch {
    // ignore
  }
};

const readLocal = (key: string): string | null => {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
};

const writeLocal = (key: string, value: string | null) => {
  try {
    if (value === null) {
      localStorage.removeItem(key);
    } else {
      localStorage.setItem(key, value);
    }
  } catch {
    // ignore
  }
};

const parseStoredConnection = (value: string | null): StoredDaemonConnectionV1 | null => {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    if (!isRecord(parsed)) return null;
    if (parsed.v !== 1) return null;
    const baseUrl = normalizeDaemonBaseUrl(String(parsed.baseUrl ?? ""));
    const wsBaseUrl = normalizeDaemonWsBaseUrl(parsed.wsBaseUrl as string | null | undefined, baseUrl);
    const authToken = normalizeToken(parsed.authToken as string | null | undefined);
    const source = normalizeToken(parsed.source as string | null | undefined);
    return {
      v: 1,
      baseUrl,
      wsBaseUrl,
      authToken,
      source,
    };
  } catch {
    return null;
  }
};

const parsePersistedBase = (value: string | null): PersistedDaemonBaseV1 | null => {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    if (!isRecord(parsed)) return null;
    if (parsed.v !== 1) return null;
    const baseUrl = normalizeDaemonBaseUrl(String(parsed.baseUrl ?? ""));
    const wsBaseUrl = normalizeDaemonWsBaseUrl(parsed.wsBaseUrl as string | null | undefined, baseUrl);
    return {
      v: 1,
      baseUrl,
      wsBaseUrl,
    };
  } catch {
    return null;
  }
};

const readRunId = (): string | null => normalizeRunId(readSession(RUN_ID_KEY));

const writeCanonicalSession = (connection: DaemonConnection) => {
  const serialized: StoredDaemonConnectionV1 = {
    v: 1,
    baseUrl: connection.baseUrl,
    wsBaseUrl: connection.wsBaseUrl,
    authToken: connection.authToken,
    source: connection.source ?? null,
  };
  writeSession(SESSION_CONNECTION_KEY, JSON.stringify(serialized));
};

const persistBaseIfRequested = (
  connection: DaemonConnection,
  opts?: SetDaemonConnectionOptions,
) => {
  if (opts?.clearPersistedBaseUrl) {
    writeLocal(LOCAL_PERSISTED_BASE_KEY, null);
    return;
  }
  if (!opts?.persistBaseUrl) return;
  if (!connection.baseUrl) {
    writeLocal(LOCAL_PERSISTED_BASE_KEY, null);
    return;
  }
  const persisted: PersistedDaemonBaseV1 = {
    v: 1,
    baseUrl: connection.baseUrl,
    wsBaseUrl: connection.wsBaseUrl,
  };
  writeLocal(LOCAL_PERSISTED_BASE_KEY, JSON.stringify(persisted));
};

const initialConnection = (): DaemonConnection => {
  const canonical = parseStoredConnection(readSession(SESSION_CONNECTION_KEY));
  if (canonical) {
    return {
      baseUrl: canonical.baseUrl,
      wsBaseUrl: canonical.wsBaseUrl,
      authToken: canonical.authToken,
      runId: readRunId(),
      source: canonical.source ?? null,
    };
  }

  const persisted = parsePersistedBase(readLocal(LOCAL_PERSISTED_BASE_KEY));
  const baseUrl = persisted?.baseUrl ?? null;
  const wsBaseUrl = normalizeDaemonWsBaseUrl(persisted?.wsBaseUrl ?? null, baseUrl);
  const restored: DaemonConnection = {
    baseUrl,
    wsBaseUrl,
    authToken: null,
    runId: readRunId(),
    source: baseUrl ? "persisted_base" : null,
  };
  if (baseUrl) {
    writeCanonicalSession(restored);
    return restored;
  }
  // Desktop windows do not use same-origin daemon routing; base URL must be supplied by
  // desktop bridge connection state to keep main-thread and worker clients aligned.
  if (!isDesktopWindow() && typeof window !== "undefined") {
    const protocol = String(window.location.protocol || "").toLowerCase();
    if (protocol === "http:" || protocol === "https:") {
      const sameOrigin = normalizeDaemonBaseUrl(window.location.origin);
      if (sameOrigin) {
        const seeded: DaemonConnection = {
          baseUrl: sameOrigin,
          wsBaseUrl: deriveDaemonWsBaseUrl(sameOrigin),
          authToken: null,
          runId: readRunId(),
          source: "same_origin_bootstrap",
        };
        writeCanonicalSession(seeded);
        return seeded;
      }
    }
  }
  return restored;
};

let state: DaemonConnection = initialConnection();

const areSameConnection = (a: DaemonConnection, b: DaemonConnection): boolean =>
  a.baseUrl === b.baseUrl
  && a.wsBaseUrl === b.wsBaseUrl
  && a.authToken === b.authToken
  && a.runId === b.runId
  && (a.source ?? null) === (b.source ?? null);

const notifyListeners = () => {
  if (listeners.size === 0) return;
  const snapshot = getDaemonConnection();
  for (const listener of listeners) {
    listener(snapshot);
  }
};

export const getDaemonConnection = (): DaemonConnection => {
  const runId = readRunId();
  if (state.runId !== runId) {
    state = { ...state, runId };
  }
  return { ...state };
};

export const subscribeDaemonConnection = (listener: DaemonConnectionListener): (() => void) => {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
};

export const setDaemonConnection = (
  update: DaemonConnectionUpdate,
  opts?: SetDaemonConnectionOptions,
): DaemonConnection => {
  const current = getDaemonConnection();
  const nextBase = update.baseUrl !== undefined
    ? normalizeDaemonBaseUrl(update.baseUrl)
    : current.baseUrl;
  const nextWs = update.wsBaseUrl !== undefined
    ? normalizeDaemonWsBaseUrl(update.wsBaseUrl, nextBase)
    : update.baseUrl !== undefined
      ? deriveDaemonWsBaseUrl(nextBase)
      : current.wsBaseUrl;
  const next: DaemonConnection = {
    baseUrl: nextBase,
    wsBaseUrl: nextWs,
    authToken: update.authToken !== undefined ? normalizeToken(update.authToken) : current.authToken,
    runId: update.runId !== undefined ? normalizeRunId(update.runId) : current.runId,
    source: update.source !== undefined ? normalizeToken(update.source) : current.source ?? null,
  };

  writeCanonicalSession(next);
  persistBaseIfRequested(next, opts);

  if (!areSameConnection(current, next)) {
    state = next;
    notifyListeners();
  }
  return getDaemonConnection();
};

export const clearDaemonConnection = (opts?: SetDaemonConnectionOptions): DaemonConnection => {
  return setDaemonConnection(
    {
      baseUrl: null,
      wsBaseUrl: null,
      authToken: null,
      source: "cleared",
    },
    {
      persistBaseUrl: opts?.persistBaseUrl,
      clearPersistedBaseUrl: opts?.clearPersistedBaseUrl ?? true,
    },
  );
};

const isLoopbackHost = (host: string): boolean => {
  const normalized = host.replace(/^\[|\]$/g, "").toLowerCase();
  return normalized === "localhost" || normalized === "::1" || normalized.startsWith("127.");
};

export const bootstrapDaemonConnectionFromRuntime = () => {
  if (typeof window === "undefined") return;
  const params = new URLSearchParams(window.location.search);
  const tokenFromQuery = normalizeToken(params.get("token"));
  const hadTokenParam = Boolean(tokenFromQuery);
  if (tokenFromQuery) {
    setDaemonConnection({ authToken: tokenFromQuery, source: "url_token" });
    params.delete("token");
    const next =
      window.location.pathname
      + (params.toString() ? `?${params.toString()}` : "")
      + window.location.hash;
    window.history.replaceState({}, "", next);
  }

  if (import.meta.env.DEV) {
    const envToken = normalizeToken(import.meta.env.VITE_CTX_AUTH_TOKEN);
    const envDaemonUrl = normalizeDaemonBaseUrl(import.meta.env.VITE_CTX_DAEMON_URL ?? null);
    if (envToken || envDaemonUrl) {
      const host = String(window.location.hostname ?? "").toLowerCase();
      if (isLoopbackHost(host)) {
        setDaemonConnection(
          {
            // URL query token has highest precedence for this load.
            authToken: envToken && !hadTokenParam ? envToken : undefined,
            // In dev loopback mode, explicit env daemon URL is authoritative.
            baseUrl: envDaemonUrl ?? undefined,
            source: "dev_env",
          },
          { persistBaseUrl: Boolean(envDaemonUrl) },
        );
      }
    }
  }

  const latest = getDaemonConnection();
  // In browser mode, same-origin /api is valid. In desktop mode, daemon origin comes from
  // bridge-managed connection state instead of the webview origin.
  if (!latest.baseUrl && !isDesktopWindow()) {
    const protocol = String(window.location.protocol || "").toLowerCase();
    if (protocol === "http:" || protocol === "https:") {
      setDaemonConnection({ baseUrl: window.location.origin, source: "same_origin_bootstrap" });
    }
  }
};

export const applyDesktopDaemonConnection = (
  info: { base_url?: string | null; token?: string | null },
): DaemonConnection => {
  return setDaemonConnection(
    {
      baseUrl: info.base_url ?? null,
      authToken: info.token ?? null,
      source: "desktop",
    },
    { persistBaseUrl: true },
  );
};

export const getDaemonWsUrl = (path: string, query?: URLSearchParams): string => {
  const connection = getDaemonConnection();
  if (!connection.wsBaseUrl) {
    throw new Error("Daemon websocket base URL is not configured.");
  }
  const prefix = connection.wsBaseUrl.replace(/\/+$/, "");
  const pathname = path.startsWith("/") ? path : `/${path}`;
  const qs = query?.toString();
  return qs ? `${prefix}${pathname}?${qs}` : `${prefix}${pathname}`;
};

export const getDaemonHttpUrl = (path: string): string => {
  const connection = getDaemonConnection();
  if (!connection.baseUrl) return path;
  if (path.startsWith("http://") || path.startsWith("https://")) return path;
  const prefix = connection.baseUrl.replace(/\/+$/, "");
  const pathname = path.startsWith("/") ? path : `/${path}`;
  return `${prefix}${pathname}`;
};
