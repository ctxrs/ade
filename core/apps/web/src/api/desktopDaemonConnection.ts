import { desktopConnectLocal, desktopGetConnection, isDesktopApp, type DesktopConnectionInfo } from "../utils/desktop";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "../state/diagnosticsChannel";
import {
  applyDesktopDaemonConnection,
  getDaemonConnection,
  hasReadyDaemonConnection,
  type DaemonConnection,
} from "./daemonConnection";

export type DesktopDaemonConnectionSyncResult = {
  connection: DaemonConnection;
  info: DesktopConnectionInfo | null;
  synced: boolean;
  error: string | null;
};

export type DesktopDaemonConnectionSyncOptions = {
  force?: boolean;
  connectLocalWhenMissing?: boolean;
  reason?: string;
};

const DESKTOP_DAEMON_SYNC_THROTTLE_MS = 1000;
const DESKTOP_LOCAL_AUTH_PROBE_TIMEOUT_MS = 5000;

let desktopSyncInFlight: Promise<DesktopDaemonConnectionSyncResult> | null = null;
let desktopLastSyncAtMs = 0;

const makeDesktopSyncResult = (
  info: DesktopConnectionInfo | null,
  error: string | null,
): DesktopDaemonConnectionSyncResult => ({
  connection: getDaemonConnection(),
  info,
  synced: Boolean(info),
  error,
});

const shouldConnectLocalWhenMissing = (
  info: DesktopConnectionInfo | null,
  connectLocalWhenMissing: boolean,
): boolean => {
  if (!connectLocalWhenMissing) return false;
  if (!info?.local_auto_bootstrap_allowed) return false;
  if (info.kind === "ssh") return false;
  return !info.base_url;
};

const shouldRepairExistingLocalDesktopTarget = (
  current: DaemonConnection,
  info: DesktopConnectionInfo | null,
): info is DesktopConnectionInfo & { kind: "local"; base_url: string } => {
  return Boolean(
    info
    && info.kind === "local"
    && info.base_url
    && current.targetScope?.kind === "desktop_local"
    && current.baseUrl === info.base_url,
  );
};

const shouldProbeExistingLocalDesktopAuth = (
  current: DaemonConnection,
  info: DesktopConnectionInfo | null,
): info is DesktopConnectionInfo & { kind: "local"; base_url: string; token: string } => {
  if (!shouldRepairExistingLocalDesktopTarget(current, info)) return false;
  return Boolean(info.token);
};

const probeDesktopLocalDaemonAuth = async (
  info: DesktopConnectionInfo & { base_url: string; token: string },
): Promise<boolean> => {
  const controller = new AbortController();
  const timeoutId = globalThis.setTimeout(() => {
    controller.abort();
  }, DESKTOP_LOCAL_AUTH_PROBE_TIMEOUT_MS);
  try {
    const baseUrl = info.base_url.replace(/\/+$/, "");
    const response = await fetch(`${baseUrl}/api/workspaces`, {
      method: "GET",
      headers: {
        authorization: `Bearer ${info.token}`,
      },
      signal: controller.signal,
    });
    return response.ok;
  } catch {
    return false;
  } finally {
    globalThis.clearTimeout(timeoutId);
  }
};

export const syncDesktopDaemonConnectionFromBridge = async (
  opts?: DesktopDaemonConnectionSyncOptions,
): Promise<DesktopDaemonConnectionSyncResult> => {
  if (!isDesktopApp()) return makeDesktopSyncResult(null, null);
  const now = Date.now();
  const current = getDaemonConnection();
  if (
    !opts?.force
    && hasReadyDaemonConnection(current)
    && now - desktopLastSyncAtMs < DESKTOP_DAEMON_SYNC_THROTTLE_MS
  ) {
    return makeDesktopSyncResult(null, null);
  }
  if (desktopSyncInFlight) return desktopSyncInFlight;
  const run = (async (): Promise<DesktopDaemonConnectionSyncResult> => {
    let info: DesktopConnectionInfo | null = null;
    let error: string | null = null;
    try {
      info = await desktopGetConnection();
      if (shouldConnectLocalWhenMissing(info, opts?.connectLocalWhenMissing ?? false)) {
        info = await desktopConnectLocal();
      } else if (shouldRepairExistingLocalDesktopTarget(current, info) && !info.token) {
        info = await desktopConnectLocal();
      } else if (shouldProbeExistingLocalDesktopAuth(current, info)) {
        const authOk = await probeDesktopLocalDaemonAuth(info);
        if (!authOk) {
          info = await desktopConnectLocal();
        }
      }
      applyDesktopDaemonConnection(info);
    } catch (err) {
      error = normalizeDiagnosticErrorMessage(err, "Desktop daemon connection sync failed.");
      if (opts?.reason) {
        emitUiDiagnostic({
          source: "api",
          code: "api.desktop_connection_sync_failed",
          severity: "warning",
          message: `Desktop daemon connection sync failed during ${opts.reason}.`,
          context: { reason: opts.reason, error },
        });
      }
    } finally {
      desktopLastSyncAtMs = Date.now();
    }
    return makeDesktopSyncResult(info, error);
  })();
  desktopSyncInFlight = run;
  try {
    return await run;
  } finally {
    if (desktopSyncInFlight === run) {
      desktopSyncInFlight = null;
    }
  }
};

export const ensureDesktopDaemonConnection = async (
  opts?: Omit<DesktopDaemonConnectionSyncOptions, "force">,
): Promise<DaemonConnection> => {
  const current = getDaemonConnection();
  if (!isDesktopApp()) return current;
  const synced = await syncDesktopDaemonConnectionFromBridge({
    force: !hasReadyDaemonConnection(current),
    connectLocalWhenMissing: opts?.connectLocalWhenMissing ?? true,
    reason: opts?.reason ?? "desktop_transport_bootstrap",
  });
  if (hasReadyDaemonConnection(synced.connection)) {
    return synced.connection;
  }
  throw new Error(synced.error ?? "Desktop daemon connection is not configured.");
};
