import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import {
  getInstall,
  getSettings,
  getTitleGenerationLocalStatus,
} from "../api/client";

const TITLE_GEN_DISMISSED_INSTALLS_KEY = "wb.title_generation.dismissed_install_ids.v1";
const TITLE_GEN_STATUS_POLL_MS = 2500;
const TITLE_GEN_INSTALL_POLL_MS = 900;

type TitleGenInstallBannerState = {
  installId: string;
  status: "running" | "failed";
  pct: number | null;
  stage: string | null;
  message: string | null;
  error: string | null;
};

const loadDismissedInstallIds = (): Set<string> => {
  try {
    const raw = localStorage.getItem(TITLE_GEN_DISMISSED_INSTALLS_KEY);
    if (!raw) return new Set<string>();
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set<string>();
    const ids = parsed.filter((value): value is string => typeof value === "string" && value.trim().length > 0);
    return new Set(ids);
  } catch {
    return new Set<string>();
  }
};

const saveDismissedInstallIds = (ids: Set<string>) => {
  try {
    localStorage.setItem(TITLE_GEN_DISMISSED_INSTALLS_KEY, JSON.stringify(Array.from(ids)));
  } catch {
    // Best effort only.
  }
};

const clampPct = (value: number): number => Math.max(0, Math.min(100, Math.round(value)));

const pctFromInstallInfo = (bytes: number | undefined, totalBytes: number | undefined): number | null => {
  if (typeof bytes !== "number" || typeof totalBytes !== "number" || totalBytes <= 0) return null;
  return clampPct((bytes / totalBytes) * 100);
};

export function TitleGenerationInstallBanner() {
  const [state, setState] = useState<TitleGenInstallBannerState | null>(null);
  const activeInstallIdRef = useRef<string | null>(null);
  const dismissedInstallIdsRef = useRef<Set<string>>(new Set<string>());
  const installPollTimerRef = useRef<number | null>(null);
  const statusPollTimerRef = useRef<number | null>(null);

  const clearInstallPoll = useCallback(() => {
    if (installPollTimerRef.current) {
      window.clearTimeout(installPollTimerRef.current);
      installPollTimerRef.current = null;
    }
  }, []);

  const clearStatusPoll = useCallback(() => {
    if (statusPollTimerRef.current) {
      window.clearTimeout(statusPollTimerRef.current);
      statusPollTimerRef.current = null;
    }
  }, []);

  const attachInstall = useCallback(async (installId: string) => {
    if (!installId.trim()) return;
    if (dismissedInstallIdsRef.current.has(installId)) return;
    if (activeInstallIdRef.current === installId && installPollTimerRef.current) return;
    activeInstallIdRef.current = installId;
    clearInstallPoll();

    const poll = async () => {
      if (activeInstallIdRef.current !== installId) return;
      try {
        const info = await getInstall(installId);
        if (activeInstallIdRef.current !== installId) return;
        const pct = pctFromInstallInfo(info.last_event?.bytes, info.last_event?.total_bytes);
        const stage = typeof info.last_event?.stage === "string" ? info.last_event.stage : null;
        const message = typeof info.last_event?.message === "string" ? info.last_event.message : null;
        if (info.state === "running") {
          setState({
            installId,
            status: "running",
            pct,
            stage,
            message,
            error: null,
          });
          installPollTimerRef.current = window.setTimeout(() => {
            poll().catch(() => {});
          }, TITLE_GEN_INSTALL_POLL_MS);
          return;
        }
        clearInstallPoll();
        activeInstallIdRef.current = null;
        if (info.state === "failed") {
          setState({
            installId,
            status: "failed",
            pct,
            stage,
            message,
            error: info.error ?? "Local title model install failed.",
          });
          return;
        }
        setState(null);
      } catch {
        if (activeInstallIdRef.current !== installId) return;
        installPollTimerRef.current = window.setTimeout(() => {
          poll().catch(() => {});
        }, TITLE_GEN_INSTALL_POLL_MS);
      }
    };

    await poll();
  }, [clearInstallPoll]);

  const refresh = useCallback(async () => {
    try {
      const settings = await getSettings();
      if (settings.title_generation?.mode !== "local") {
        clearInstallPoll();
        activeInstallIdRef.current = null;
        setState(null);
        return;
      }
      const status = await getTitleGenerationLocalStatus();
      if (status.install_running && typeof status.install_id === "string" && status.install_id.trim()) {
        await attachInstall(status.install_id);
        return;
      }
      if (activeInstallIdRef.current && !status.install_running) {
        clearInstallPoll();
        activeInstallIdRef.current = null;
      }
      if (status.ready) {
        setState(null);
      }
    } catch {
      // Keep silent; this banner is best effort.
    }
  }, [attachInstall, clearInstallPoll]);

  useEffect(() => {
    dismissedInstallIdsRef.current = loadDismissedInstallIds();
    void refresh();
    const pollLoop = () => {
      void refresh().finally(() => {
        statusPollTimerRef.current = window.setTimeout(pollLoop, TITLE_GEN_STATUS_POLL_MS);
      });
    };
    statusPollTimerRef.current = window.setTimeout(pollLoop, TITLE_GEN_STATUS_POLL_MS);
    return () => {
      clearInstallPoll();
      clearStatusPoll();
      activeInstallIdRef.current = null;
    };
  }, [clearInstallPoll, clearStatusPoll, refresh]);

  const dismiss = useCallback(() => {
    if (state?.installId) {
      dismissedInstallIdsRef.current.add(state.installId);
      saveDismissedInstallIds(dismissedInstallIdsRef.current);
    }
    if (activeInstallIdRef.current === state?.installId) {
      activeInstallIdRef.current = null;
      clearInstallPoll();
    }
    setState(null);
  }, [clearInstallPoll, state]);

  if (!state) return null;

  const progressLabel = state.pct == null ? "Downloading…" : `Downloading… ${state.pct}%`;
  const title = state.status === "failed"
    ? "Local session titling install failed."
    : "Session titling model download in progress.";
  const subtitle = state.status === "failed"
    ? state.error ?? "Review daemon logs for more details."
    : (state.message ?? state.stage ?? "The model will be ready automatically when download completes.");

  return (
    <div className="wb-snackbar wb-snackbar-titlegen" role="status" aria-live="polite">
      <div className="wb-snackbar-body">
        <div className="wb-snackbar-title">{title}</div>
        <div className="wb-snackbar-subtitle">{subtitle}</div>
        {state.status === "running" ? (
          <div className="wb-snackbar-progress-block">
            <div className="wb-snackbar-progress-label">{progressLabel}</div>
            <div
              className="wb-snackbar-progress"
              role="progressbar"
              aria-label="Session titling install progress"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={state.pct ?? undefined}
            >
              <div
                className={`wb-snackbar-progress-fill${state.pct == null ? " is-indeterminate" : ""}`}
                style={state.pct == null ? undefined : { width: `${state.pct}%` }}
              />
            </div>
          </div>
        ) : null}
      </div>
      <button type="button" className="wb-snackbar-close" onClick={dismiss} aria-label="Dismiss">
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  );
}
