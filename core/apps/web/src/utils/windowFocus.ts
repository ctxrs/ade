export const APP_FOREGROUND_STORAGE_KEY = "ctx.app-foreground-window.v1";

const APP_FOREGROUND_MARKER_STALE_MS = 15_000;
const APP_FOREGROUND_MARKER_REFRESH_MS = 5_000;
const foregroundWindowId =
  typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
    ? crypto.randomUUID()
    : `ctx-window-${Math.random().toString(36).slice(2)}`;

let foregroundListenersInstalled = false;

type ForegroundMarker = {
  updatedAtMs: number;
  windowId: string;
};

const canUseDocument = (): boolean => typeof window !== "undefined" && typeof document !== "undefined";

const currentWindowInForeground = (): boolean => {
  if (typeof document === "undefined") return true;
  const visibility = document.visibilityState;
  if (visibility && visibility !== "visible") return false;
  if (typeof document.hasFocus === "function") {
    return document.hasFocus();
  }
  return true;
};

const readForegroundMarker = (): ForegroundMarker | null => {
  if (!canUseDocument()) return null;
  try {
    const raw = window.localStorage.getItem(APP_FOREGROUND_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<ForegroundMarker>;
    const windowId = String(parsed.windowId ?? "").trim();
    const updatedAtMs = Number(parsed.updatedAtMs ?? NaN);
    if (!windowId || !Number.isFinite(updatedAtMs)) {
      window.localStorage.removeItem(APP_FOREGROUND_STORAGE_KEY);
      return null;
    }
    if (Date.now() - updatedAtMs > APP_FOREGROUND_MARKER_STALE_MS) {
      window.localStorage.removeItem(APP_FOREGROUND_STORAGE_KEY);
      return null;
    }
    return { windowId, updatedAtMs };
  } catch {
    return null;
  }
};

const writeForegroundMarker = (foreground: boolean): void => {
  if (!canUseDocument()) return;
  try {
    if (foreground) {
      const marker: ForegroundMarker = {
        windowId: foregroundWindowId,
        updatedAtMs: Date.now(),
      };
      window.localStorage.setItem(APP_FOREGROUND_STORAGE_KEY, JSON.stringify(marker));
      return;
    }
    const marker = readForegroundMarker();
    if (marker?.windowId === foregroundWindowId) {
      window.localStorage.removeItem(APP_FOREGROUND_STORAGE_KEY);
    }
  } catch {
    // Ignore storage failures so focus checks keep working in-memory.
  }
};

const syncForegroundMarkerFromDocument = (): void => {
  writeForegroundMarker(currentWindowInForeground());
};

const installForegroundListeners = (): void => {
  if (foregroundListenersInstalled || !canUseDocument()) return;
  foregroundListenersInstalled = true;
  window.addEventListener("focus", syncForegroundMarkerFromDocument);
  window.addEventListener("blur", syncForegroundMarkerFromDocument);
  document.addEventListener("visibilitychange", syncForegroundMarkerFromDocument);
  window.addEventListener("beforeunload", () => {
    writeForegroundMarker(false);
  });
  window.setInterval(() => {
    if (!currentWindowInForeground()) return;
    writeForegroundMarker(true);
  }, APP_FOREGROUND_MARKER_REFRESH_MS);
  syncForegroundMarkerFromDocument();
};

export function initializeAppForegroundTracking(): void {
  if (typeof document === "undefined") return;
  installForegroundListeners();
}

export function isAppInForeground(): boolean {
  if (typeof document === "undefined") return true;
  initializeAppForegroundTracking();
  if (currentWindowInForeground()) {
    writeForegroundMarker(true);
    return true;
  }
  return readForegroundMarker() !== null;
}
