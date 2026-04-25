import type { DesktopConnectionInfo } from "../utils/desktop";
import {
  cloneDaemonTargetScope,
  createBrowserDaemonTargetScope,
  createDesktopLocalDaemonTargetScope,
  daemonTargetScopeFromDesktopConnectionInfo,
  deserializeDaemonTargetScope,
  sameDaemonTargetScope,
  serializeDaemonTargetScope,
  type DaemonTargetScope,
} from "../state/scopeIdentity";

export type DaemonConnection = {
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  runId: string | null;
  source?: string | null;
  targetScope?: DaemonTargetScope | null;
};

export type DaemonConnectionUpdate = {
  baseUrl?: string | null;
  wsBaseUrl?: string | null;
  authToken?: string | null;
  runId?: string | null;
  source?: string | null;
  targetScope?: DaemonTargetScope | null;
};

export type SetDaemonConnectionOptions = {
  persistBaseUrl?: boolean;
  clearPersistedBaseUrl?: boolean;
};

export type DaemonConnectionReadiness = {
  hasBaseUrl: boolean;
  hasAuthToken: boolean;
  isReady: boolean;
  missing: "base" | "auth" | null;
};

type StoredDaemonConnectionV1 = {
  v: 1;
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  source?: string | null;
  targetScope?: string | null;
};

type PersistedDaemonBaseV1 = {
  v: 1;
  baseUrl: string | null;
  wsBaseUrl: string | null;
  targetScope?: string | null;
};

type ParsedStoredDaemonConnection = {
  baseUrl: string | null;
  wsBaseUrl: string | null;
  authToken: string | null;
  source: string | null;
  targetScope: DaemonTargetScope | null;
};

type ParsedPersistedDaemonBase = {
  baseUrl: string | null;
  wsBaseUrl: string | null;
  targetScope: DaemonTargetScope | null;
};

type DesktopDaemonConnectionInfoLike = {
  kind?: DesktopConnectionInfo["kind"] | null;
  base_url?: string | null;
  token?: string | null;
  host?: string | null;
  user?: string | null;
  remote_port?: number | null;
  remote_data_dir?: string | null;
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

const cloneNullableTargetScope = (scope: DaemonTargetScope | null | undefined): DaemonTargetScope | null =>
  scope ? cloneDaemonTargetScope(scope) : null;

const sameNullableTargetScope = (
  lhs: DaemonTargetScope | null | undefined,
  rhs: DaemonTargetScope | null | undefined,
): boolean => {
  if (lhs === rhs) return true;
  if (!lhs || !rhs) return false;
  return sameDaemonTargetScope(lhs, rhs);
};

const inferLegacyDaemonTargetScope = (
  baseUrl: string | null,
  source: string | null | undefined,
): DaemonTargetScope | null => {
  if (!baseUrl) return null;
  if (source === "desktop" || isDesktopWindow()) {
    return createDesktopLocalDaemonTargetScope();
  }
  return createBrowserDaemonTargetScope(baseUrl);
};

const parseStoredTargetScope = (
  value: unknown,
  fallbackBaseUrl: string | null,
  fallbackSource: string | null | undefined,
): DaemonTargetScope | null | undefined => {
  if (value === undefined) {
    return inferLegacyDaemonTargetScope(fallbackBaseUrl, fallbackSource);
  }
  if (value === null) {
    return fallbackBaseUrl ? undefined : null;
  }
  if (typeof value !== "string") return undefined;
  const targetScope = deserializeDaemonTargetScope(value);
  return targetScope ?? undefined;
};

const daemonTargetScopeFromDesktopConnectionLike = (
  info: DesktopDaemonConnectionInfoLike | null | undefined,
): DaemonTargetScope | null => {
  if (!info) return null;
  const fromBridge = info.kind
    ? daemonTargetScopeFromDesktopConnectionInfo({
        kind: info.kind,
        host: info.host,
        user: info.user,
        remote_port: info.remote_port,
        remote_data_dir: info.remote_data_dir,
      })
    : null;
  if (fromBridge) return fromBridge;
  return info.base_url ? createDesktopLocalDaemonTargetScope() : null;
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

const parseStoredConnection = (value: string | null): ParsedStoredDaemonConnection | null => {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    if (!isRecord(parsed)) return null;
    if (parsed.v !== 1) return null;
    const baseUrl = normalizeDaemonBaseUrl(String(parsed.baseUrl ?? ""));
    const wsBaseUrl = normalizeDaemonWsBaseUrl(parsed.wsBaseUrl as string | null | undefined, baseUrl);
    const authToken = normalizeToken(parsed.authToken as string | null | undefined);
    const source = normalizeToken(parsed.source as string | null | undefined);
    const targetScope = parseStoredTargetScope(parsed.targetScope, baseUrl, source);
    if (targetScope === undefined) return null;
    return {
      baseUrl,
      wsBaseUrl,
      authToken,
      source,
      targetScope,
    };
  } catch {
    return null;
  }
};

const parsePersistedBase = (value: string | null): ParsedPersistedDaemonBase | null => {
  if (!value) return null;
  try {
    const parsed = JSON.parse(value) as unknown;
    if (!isRecord(parsed)) return null;
    if (parsed.v !== 1) return null;
    const baseUrl = normalizeDaemonBaseUrl(String(parsed.baseUrl ?? ""));
    const wsBaseUrl = normalizeDaemonWsBaseUrl(parsed.wsBaseUrl as string | null | undefined, baseUrl);
    const targetScope = parseStoredTargetScope(parsed.targetScope, baseUrl, "persisted_base");
    if (targetScope === undefined) return null;
    return {
      baseUrl,
      wsBaseUrl,
      targetScope,
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
    targetScope: connection.targetScope ? serializeDaemonTargetScope(connection.targetScope) : null,
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
    targetScope: connection.targetScope ? serializeDaemonTargetScope(connection.targetScope) : null,
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
      targetScope: cloneNullableTargetScope(canonical.targetScope),
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
    targetScope: cloneNullableTargetScope(persisted?.targetScope ?? null),
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
          targetScope: createBrowserDaemonTargetScope(sameOrigin),
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
  && (a.source ?? null) === (b.source ?? null)
  && sameNullableTargetScope(a.targetScope, b.targetScope);

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
  return {
    ...state,
    targetScope: cloneNullableTargetScope(state.targetScope),
  };
};

export const getDaemonConnectionReadiness = (
  connection: Pick<DaemonConnection, "baseUrl" | "authToken"> = getDaemonConnection(),
): DaemonConnectionReadiness => {
  const hasBaseUrl = Boolean(connection.baseUrl);
  const hasAuthToken = Boolean(connection.authToken);
  return {
    hasBaseUrl,
    hasAuthToken,
    isReady: hasBaseUrl && hasAuthToken,
    missing: !hasBaseUrl ? "base" : !hasAuthToken ? "auth" : null,
  };
};

export const hasReadyDaemonConnection = (
  connection: Pick<DaemonConnection, "baseUrl" | "authToken"> = getDaemonConnection(),
): boolean => getDaemonConnectionReadiness(connection).isReady;

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
  const nextTargetScope = update.targetScope !== undefined
    ? cloneNullableTargetScope(update.targetScope)
    : update.baseUrl !== undefined
      ? nextBase
        ? isDesktopWindow() && current.targetScope && current.targetScope.kind !== "browser"
          ? cloneNullableTargetScope(current.targetScope)
          : createBrowserDaemonTargetScope(nextBase)
        : null
      : cloneNullableTargetScope(current.targetScope);
  const next: DaemonConnection = {
    baseUrl: nextBase,
    wsBaseUrl: nextWs,
    authToken: update.authToken !== undefined ? normalizeToken(update.authToken) : current.authToken,
    runId: update.runId !== undefined ? normalizeRunId(update.runId) : current.runId,
    source: update.source !== undefined ? normalizeToken(update.source) : current.source ?? null,
    targetScope: nextTargetScope,
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
      targetScope: null,
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

const getBrowserSameOriginBaseUrl = (): string | null => {
  if (typeof window === "undefined" || isDesktopWindow()) return null;
  const protocol = String(window.location.protocol || "").toLowerCase();
  if (protocol !== "http:" && protocol !== "https:") return null;
  return normalizeDaemonBaseUrl(window.location.origin);
};

const stripQueryTokenFromLocation = (): void => {
  const params = new URLSearchParams(window.location.search);
  if (!params.has("token")) return;
  params.delete("token");
  const next =
    window.location.pathname
    + (params.toString() ? `?${params.toString()}` : "")
    + window.location.hash;
  window.history.replaceState({}, "", next);
};

const consumeFragmentTokenFromLocation = (): { token: string | null; hadTokenParam: boolean } => {
  const rawHash = String(window.location.hash || "").replace(/^#/, "");
  if (!rawHash) {
    return { token: null, hadTokenParam: false };
  }
  const normalizedHash = rawHash.startsWith("?") ? rawHash.slice(1) : rawHash;
  const params = new URLSearchParams(normalizedHash);
  if (!params.has("token")) {
    return { token: null, hadTokenParam: false };
  }
  const token = normalizeToken(params.get("token"));
  params.delete("token");
  const nextHash = params.toString();
  const next =
    window.location.pathname
    + window.location.search
    + (nextHash ? `#${nextHash}` : "");
  window.history.replaceState({}, "", next);
  return { token, hadTokenParam: Boolean(token) };
};

export const bootstrapDaemonConnectionFromRuntime = () => {
  if (typeof window === "undefined") return;
  stripQueryTokenFromLocation();
  const { token: tokenFromFragment, hadTokenParam } = consumeFragmentTokenFromLocation();
  const envToken = import.meta.env.DEV ? normalizeToken(import.meta.env.VITE_CTX_AUTH_TOKEN) : null;
  const envDaemonUrl = import.meta.env.DEV
    ? normalizeDaemonBaseUrl(import.meta.env.VITE_CTX_DAEMON_URL ?? null)
    : null;
  if (tokenFromFragment) {
    const sameOriginBaseUrl = getBrowserSameOriginBaseUrl();
    const current = getDaemonConnection();
    const shouldResetBrowserBaseFromToken =
      !envDaemonUrl
      && Boolean(sameOriginBaseUrl)
      && current.targetScope?.kind === "browser"
      && current.baseUrl !== sameOriginBaseUrl;
    setDaemonConnection(
      {
        baseUrl: shouldResetBrowserBaseFromToken ? sameOriginBaseUrl : undefined,
        authToken: tokenFromFragment,
        source: "url_token",
      },
      shouldResetBrowserBaseFromToken ? { clearPersistedBaseUrl: true } : undefined,
    );
  }

  if (import.meta.env.DEV) {
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
    const sameOriginBaseUrl = getBrowserSameOriginBaseUrl();
    if (sameOriginBaseUrl) {
      setDaemonConnection({ baseUrl: sameOriginBaseUrl, source: "same_origin_bootstrap" });
    }
  }
};

export const applyDesktopDaemonConnection = (
  info: DesktopDaemonConnectionInfoLike | null | undefined,
): DaemonConnection => {
  return setDaemonConnection(
    {
      baseUrl: info?.base_url ?? null,
      authToken: info?.token ?? null,
      source: "desktop",
      targetScope: daemonTargetScopeFromDesktopConnectionLike(info),
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
