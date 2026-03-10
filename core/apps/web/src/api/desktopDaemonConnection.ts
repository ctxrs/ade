import { desktopConnectLocal, desktopGetConnection, isDesktopApp, type DesktopConnectionInfo } from "../utils/desktop";
import { emitUiDiagnostic, normalizeDiagnosticErrorMessage } from "../state/diagnosticsChannel";
import {
  applyDesktopDaemonConnection,
  getDaemonConnection,
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

let desktopSyncInFlight: Promise<DesktopDaemonConnectionSyncResult> | null = null;
let desktopLastSyncAtMs = 0;

const hasDesktopDataPlaneConnection = (connection: DaemonConnection): boolean =>
  Boolean(connection.baseUrl && connection.authToken);

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
  if (!info) return true;
  if (info.kind === "ssh") return false;
  return !info.base_url;
};

export const syncDesktopDaemonConnectionFromBridge = async (
  opts?: DesktopDaemonConnectionSyncOptions,
): Promise<DesktopDaemonConnectionSyncResult> => {
  if (!isDesktopApp()) return makeDesktopSyncResult(null, null);
  const now = Date.now();
  const current = getDaemonConnection();
  if (
    !opts?.force
    && hasDesktopDataPlaneConnection(current)
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
    force: !hasDesktopDataPlaneConnection(current),
    connectLocalWhenMissing: opts?.connectLocalWhenMissing ?? true,
    reason: opts?.reason ?? "desktop_transport_bootstrap",
  });
  if (hasDesktopDataPlaneConnection(synced.connection)) {
    return synced.connection;
  }
  throw new Error(synced.error ?? "Desktop daemon connection is not configured.");
};
