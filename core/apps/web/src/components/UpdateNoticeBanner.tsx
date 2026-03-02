import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { Info, X } from "lucide-react";
import { applyAppImageUpdate, downloadAppImageUpdate, type UpdateCheck } from "../api/client";
import { desktopApplyAppUpdate, desktopRestartApp, isDesktopApp } from "../utils/desktop";
import { readCachedUpdateCheck, refreshUpdateCheck, writeCachedUpdateCheck } from "../utils/updateNotice";

const PROMPT_SNOOZE_STORAGE_KEY = "ctx_update_prompt_next_allowed_at_v1";
const IDLE_UPDATE_VERSION_STORAGE_KEY = "ctx_update_prompt_idle_versions_v1";
const AUTO_APPLY_ON_LAUNCH_STORAGE_KEY = "ctx_update_auto_apply_on_launch_v1";
const RESTART_REQUIRED_VERSION_STORAGE_KEY = "ctx_update_restart_required_version_v1";
const POLL_INTERVAL_MS = 60 * 60 * 1000;
const PROMPT_SNOOZE_MS = 24 * 60 * 60 * 1000;

const readVersionSet = (key: string): Set<string> => {
  if (typeof window === "undefined") return new Set<string>();
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return new Set<string>();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set<string>();
    const versions = parsed.map((value) => String(value).trim()).filter(Boolean);
    return new Set<string>(versions);
  } catch {
    return new Set<string>();
  }
};

const readIdleUpdateVersions = (): Set<string> => readVersionSet(IDLE_UPDATE_VERSION_STORAGE_KEY);

const writeVersionSet = (key: string, versions: Set<string>) => {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(key, JSON.stringify(Array.from(versions.values())));
  } catch {
    // ignore local storage failures
  }
};

const writeIdleUpdateVersions = (versions: Set<string>) =>
  writeVersionSet(IDLE_UPDATE_VERSION_STORAGE_KEY, versions);

const shouldAutoApplyOnLaunch = (): boolean => {
  if (typeof window === "undefined") return true;
  try {
    const raw = String(window.localStorage.getItem(AUTO_APPLY_ON_LAUNCH_STORAGE_KEY) ?? "").trim().toLowerCase();
    return !(raw === "0" || raw === "false" || raw === "off");
  } catch {
    return true;
  }
};

const readRestartRequiredVersion = (): string => {
  if (typeof window === "undefined") return "";
  try {
    return String(window.sessionStorage.getItem(RESTART_REQUIRED_VERSION_STORAGE_KEY) ?? "").trim();
  } catch {
    return "";
  }
};

const writeRestartRequiredVersion = (version: string): void => {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.setItem(RESTART_REQUIRED_VERSION_STORAGE_KEY, version);
  } catch {
    // ignore storage failures
  }
};

const clearRestartRequiredVersion = (): void => {
  if (typeof window === "undefined") return;
  try {
    window.sessionStorage.removeItem(RESTART_REQUIRED_VERSION_STORAGE_KEY);
  } catch {
    // ignore storage failures
  }
};

const readPromptSnoozeByVersion = (): Record<string, number> => {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(PROMPT_SNOOZE_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const next: Record<string, number> = {};
    for (const [key, value] of Object.entries(parsed)) {
      const version = String(key || "").trim();
      if (!version) continue;
      const millis = Number(value);
      if (!Number.isFinite(millis) || millis <= 0) continue;
      next[version] = Math.floor(millis);
    }
    return next;
  } catch {
    return {};
  }
};

const writePromptSnoozeByVersion = (value: Record<string, number>) => {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(PROMPT_SNOOZE_STORAGE_KEY, JSON.stringify(value));
  } catch {
    // ignore local storage failures
  }
};

const SEMVER_RE =
  /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

type ParsedSemVer = {
  major: number;
  minor: number;
  patch: number;
  prerelease: string[];
};

type UpdateApplySource = "manual" | "launch_auto" | "idle" | "forced";

type NoticePhase = "ready" | "applying" | "restart_required";

type NoticeUiState = {
  phase: NoticePhase;
  error: string | null;
  infoModalOpen: boolean;
};

type NoticeUiAction =
  | { type: "apply_started" }
  | { type: "apply_failed"; message: string }
  | { type: "apply_completed" }
  | { type: "restart_required"; message: string }
  | { type: "info_opened" }
  | { type: "info_closed" };

const initialNoticeUiState: NoticeUiState = {
  phase: "ready",
  error: null,
  infoModalOpen: false,
};

const noticeUiReducer = (state: NoticeUiState, action: NoticeUiAction): NoticeUiState => {
  switch (action.type) {
    case "apply_started":
      return { ...state, phase: "applying", error: null };
    case "apply_failed":
      return { ...state, phase: "ready", error: action.message };
    case "apply_completed":
      return { ...state, phase: "ready", error: null };
    case "restart_required":
      return { ...state, phase: "restart_required", error: action.message };
    case "info_opened":
      return { ...state, infoModalOpen: true };
    case "info_closed":
      return { ...state, infoModalOpen: false };
    default:
      return state;
  }
};

const normalizeOptionalString = (value: string | null | undefined): string =>
  String(value ?? "").trim();

const areUpdateChecksEqual = (left: UpdateCheck | null, right: UpdateCheck | null): boolean => {
  if (left === right) return true;
  if (!left || !right) return false;
  return (
    left.channel === right.channel &&
    left.base_url === right.base_url &&
    normalizeOptionalString(left.platform) === normalizeOptionalString(right.platform) &&
    left.current_version === right.current_version &&
    normalizeOptionalString(left.latest_version) === normalizeOptionalString(right.latest_version) &&
    normalizeOptionalString(left.min_supported_version) === normalizeOptionalString(right.min_supported_version) &&
    left.update_available === right.update_available &&
    (left.platform_supported ?? null) === (right.platform_supported ?? null) &&
    (left.in_place_update_supported ?? null) === (right.in_place_update_supported ?? null) &&
    normalizeOptionalString(left.in_place_update_reason) === normalizeOptionalString(right.in_place_update_reason)
  );
};

const parseSemVer = (value: string): ParsedSemVer | null => {
  const trimmed = String(value || "").trim();
  const match = trimmed.match(SEMVER_RE);
  if (!match) return null;
  const prerelease = match[4] ? match[4].split(".") : [];
  return {
    major: Number(match[1]),
    minor: Number(match[2]),
    patch: Number(match[3]),
    prerelease,
  };
};

const getInPlaceCapability = (info: UpdateCheck | null): { supported: boolean; reason: string } => {
  if (!info) return { supported: false, reason: "" };
  const reason = normalizeOptionalString(info.in_place_update_reason);
  return {
    supported: info.in_place_update_supported === true,
    reason,
  };
};

const isNumericIdentifier = (value: string): boolean => /^[0-9]+$/.test(value);

const comparePrereleaseIdentifier = (left: string, right: string): number => {
  const leftNumeric = isNumericIdentifier(left);
  const rightNumeric = isNumericIdentifier(right);
  if (leftNumeric && rightNumeric) {
    const leftNum = Number(left);
    const rightNum = Number(right);
    if (leftNum < rightNum) return -1;
    if (leftNum > rightNum) return 1;
    return 0;
  }
  if (leftNumeric && !rightNumeric) return -1;
  if (!leftNumeric && rightNumeric) return 1;
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
};

const compareVersions = (left: string, right: string): number | null => {
  const leftVer = parseSemVer(left);
  const rightVer = parseSemVer(right);
  if (!leftVer || !rightVer) return null;
  if (leftVer.major !== rightVer.major) return leftVer.major < rightVer.major ? -1 : 1;
  if (leftVer.minor !== rightVer.minor) return leftVer.minor < rightVer.minor ? -1 : 1;
  if (leftVer.patch !== rightVer.patch) return leftVer.patch < rightVer.patch ? -1 : 1;

  const leftPre = leftVer.prerelease;
  const rightPre = rightVer.prerelease;
  if (leftPre.length === 0 && rightPre.length === 0) return 0;
  if (leftPre.length === 0) return 1;
  if (rightPre.length === 0) return -1;

  const len = Math.max(leftPre.length, rightPre.length);
  for (let i = 0; i < len; i += 1) {
    const a = leftPre[i];
    const b = rightPre[i];
    if (a === undefined) return -1;
    if (b === undefined) return 1;
    const cmp = comparePrereleaseIdentifier(a, b);
    if (cmp !== 0) return cmp;
  }
  return 0;
};

const isForcedUpdate = (info: UpdateCheck | null): boolean => {
  if (!info) return false;
  if (!info.update_available) return false;
  if (info.platform_supported === false) return false;
  const current = String(info.current_version ?? "").trim();
  const minimum = String(info.min_supported_version ?? "").trim();
  const latest = String(info.latest_version ?? "").trim();
  if (!latest) return false;
  if (!current || !minimum) return false;
  return compareVersions(current, minimum) === -1;
};

const isCurrentVersionAtOrAbove = (currentVersion: string, requiredVersion: string): boolean => {
  const cmp = compareVersions(currentVersion, requiredVersion);
  if (cmp === null) return currentVersion === requiredVersion;
  return cmp >= 0;
};

type UpdateNoticeBannerProps = {
  allTasksIdle?: boolean;
};

export default function UpdateNoticeBanner({ allTasksIdle = true }: UpdateNoticeBannerProps) {
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(() => readCachedUpdateCheck());
  const [promptSnoozeByVersion, setPromptSnoozeByVersion] = useState<Record<string, number>>(
    () => readPromptSnoozeByVersion(),
  );
  const [idleUpdateVersions, setIdleUpdateVersions] = useState<Set<string>>(() => readIdleUpdateVersions());
  const [uiState, dispatchUi] = useReducer(noticeUiReducer, initialNoticeUiState);
  const [restartingApp, setRestartingApp] = useState(false);
  const launchAutoApplyAttemptedRef = useRef(false);
  const applyInFlightRef = useRef(false);
  const updateInfoRef = useRef<UpdateCheck | null>(updateInfo);
  const isDesktop = isDesktopApp();
  const autoApplyOnLaunchEnabled = shouldAutoApplyOnLaunch();

  useEffect(() => {
    updateInfoRef.current = updateInfo;
  }, [updateInfo]);

  const latest = (updateInfo?.latest_version ?? "").trim() || "unknown";
  const latestKnownVersion = (updateInfo?.latest_version ?? "").trim();
  const minimumSupportedVersion = (updateInfo?.min_supported_version ?? "").trim();
  const nextPromptAtMs = latestKnownVersion ? Number(promptSnoozeByVersion[latestKnownVersion] ?? 0) : 0;
  const nowMs = Date.now();
  const inPlaceCapability = getInPlaceCapability(updateInfo);
  const canApplyFromCurrentClient = isDesktop || inPlaceCapability.supported;
  const forcedUpdateNeedsManualInstall = isForcedUpdate(updateInfo) && !canApplyFromCurrentClient;
  const shouldShow = Boolean(updateInfo?.update_available) && nowMs >= nextPromptAtMs;
  const forcedUpdate = isForcedUpdate(updateInfo) && canApplyFromCurrentClient;
  const applyingUpdate = uiState.phase === "applying";
  const restartRequired = uiState.phase === "restart_required";
  const showInfoModal = uiState.infoModalOpen;
  const updateError = uiState.error;
  const effectiveError =
    updateError ||
    (forcedUpdateNeedsManualInstall
      ? inPlaceCapability.reason ||
        "This version is no longer supported on this install path. Install the latest version from release notes."
      : null);

  const snoozeVersionPrompt = useCallback((version: string) => {
    if (!version) return;
    setPromptSnoozeByVersion((prev) => {
      const next = {
        ...prev,
        [version]: Date.now() + PROMPT_SNOOZE_MS,
      };
      writePromptSnoozeByVersion(next);
      return next;
    });
  }, []);

  const dismissForLater = useCallback(() => {
    if (!latestKnownVersion) return;
    snoozeVersionPrompt(latestKnownVersion);
  }, [latestKnownVersion, snoozeVersionPrompt]);

  const clearVersionFlags = useCallback((version: string) => {
    if (!version) return;
    setPromptSnoozeByVersion((prev) => {
      if (!(version in prev)) return prev;
      const next = { ...prev };
      delete next[version];
      writePromptSnoozeByVersion(next);
      return next;
    });
    setIdleUpdateVersions((prev) => {
      if (!prev.has(version)) return prev;
      const next = new Set(prev);
      next.delete(version);
      writeIdleUpdateVersions(next);
      return next;
    });
  }, []);

  const setRestartRequiredVersionState = useCallback((version: string) => {
    if (!version) return;
    writeRestartRequiredVersion(version);
  }, []);

  const clearRestartRequiredVersionState = useCallback(() => {
    clearRestartRequiredVersion();
  }, []);

  const reconcileRestartRequiredState = useCallback(
    (info: UpdateCheck | null): boolean => {
      const pendingRestartVersion = readRestartRequiredVersion();
      if (!pendingRestartVersion) return false;
      const currentVersion = String(info?.current_version ?? "").trim();
      if (currentVersion && isCurrentVersionAtOrAbove(currentVersion, pendingRestartVersion)) {
        clearRestartRequiredVersionState();
        dispatchUi({ type: "apply_completed" });
        return false;
      }
      dispatchUi({
        type: "restart_required",
        message: "Update installed. Relaunch the app to complete the update.",
      });
      return true;
    },
    [clearRestartRequiredVersionState],
  );

  const refresh = useCallback(
    async (force = false): Promise<UpdateCheck | null> => {
      const info = await refreshUpdateCheck(force ? { force: true } : undefined);
      if (info) {
        updateInfoRef.current = info;
        setUpdateInfo((prev) => (areUpdateChecksEqual(prev, info) ? prev : info));
      }
      reconcileRestartRequiredState(info);
      return info;
    },
    [reconcileRestartRequiredState],
  );

  const applyUpdateNow = useCallback(
    async (version: string, source: UpdateApplySource = "manual"): Promise<boolean> => {
      if (!version || applyInFlightRef.current) return false;
      const recoverIdleFailure = () => {
        if (source === "idle") clearVersionFlags(version);
      };
      const markRestartRequired = (message: string | null | undefined) => {
        clearVersionFlags(version);
        const restartVersion = String(updateInfoRef.current?.latest_version ?? version).trim();
        if (restartVersion) setRestartRequiredVersionState(restartVersion);
        dispatchUi({
          type: "restart_required",
          message: message || "Update installed. Relaunch the app to complete the update.",
        });
      };
      applyInFlightRef.current = true;
      dispatchUi({ type: "apply_started" });
      try {
        if (isDesktop) {
          const resp = await desktopApplyAppUpdate("stable");
          if (!resp.applied && !resp.needs_restart) {
            dispatchUi({ type: "apply_failed", message: resp.message || "Update did not apply." });
            recoverIdleFailure();
            return false;
          }
          if (resp.needs_restart) {
            markRestartRequired(resp.message);
            return true;
          }
        } else {
          const capability = getInPlaceCapability(updateInfoRef.current);
          if (!capability.supported) {
            dispatchUi({
              type: "apply_failed",
              message:
                capability.reason ||
                "This install cannot update in place. Install the latest version from release notes, then relaunch.",
            });
            recoverIdleFailure();
            return false;
          }
          const downloadResp = await downloadAppImageUpdate("stable");
          if (!downloadResp.can_apply_in_place) {
            dispatchUi({
              type: "apply_failed",
              message: "This install cannot update in place. Install the latest version from release notes, then relaunch.",
            });
            recoverIdleFailure();
            return false;
          }
          const resp = await applyAppImageUpdate();
          if (!resp.applied) {
            dispatchUi({ type: "apply_failed", message: resp.message || "Update did not apply." });
            recoverIdleFailure();
            return false;
          }
          markRestartRequired(resp.message);
          return true;
        }
        clearVersionFlags(version);
        clearRestartRequiredVersionState();
        snoozeVersionPrompt(version);
        const source = updateInfoRef.current;
        if (source) {
          const next = {
            ...source,
            current_version: source.latest_version ?? source.current_version,
            update_available: false,
          };
          updateInfoRef.current = next;
          setUpdateInfo(next);
          writeCachedUpdateCheck(next);
        }
        dispatchUi({ type: "apply_completed" });
        return true;
      } catch (err: unknown) {
        dispatchUi({ type: "apply_failed", message: err instanceof Error ? err.message : "Failed to apply update." });
        recoverIdleFailure();
        return false;
      } finally {
        applyInFlightRef.current = false;
      }
    },
    [clearRestartRequiredVersionState, clearVersionFlags, isDesktop, setRestartRequiredVersionState, snoozeVersionPrompt],
  );

  useEffect(() => {
    let cancelled = false;
    const runInitialCheck = async () => {
      const pendingRestartVersion = readRestartRequiredVersion();
      if (pendingRestartVersion) {
        setRestartRequiredVersionState(pendingRestartVersion);
        dispatchUi({
          type: "restart_required",
          message: "Update installed. Relaunch the app to complete the update.",
        });
      }
      // Startup must always perform a real update check request.
      const info = await refresh(true);
      if (cancelled) return;
      const hasPendingRestart = Boolean(readRestartRequiredVersion());
      if (!launchAutoApplyAttemptedRef.current) {
        launchAutoApplyAttemptedRef.current = true;
        const version = String(info?.latest_version ?? "").trim();
        if (
          autoApplyOnLaunchEnabled &&
          isDesktop &&
          !hasPendingRestart &&
          info?.update_available &&
          version &&
          !isForcedUpdate(info)
        ) {
          await applyUpdateNow(version, "launch_auto");
        }
      }
    };
    void runInitialCheck();
    const intervalId = window.setInterval(() => {
      void refresh(true);
    }, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [applyUpdateNow, autoApplyOnLaunchEnabled, isDesktop, refresh, setRestartRequiredVersionState]);

  useEffect(() => {
    if (forcedUpdate) return;
    if (!allTasksIdle) return;
    if (!latestKnownVersion) return;
    if (!updateInfo?.update_available) return;
    if (!idleUpdateVersions.has(latestKnownVersion)) return;
    void applyUpdateNow(latestKnownVersion, "idle");
  }, [allTasksIdle, applyUpdateNow, forcedUpdate, idleUpdateVersions, latestKnownVersion, updateInfo?.update_available]);

  const onUpdateNow = useCallback(() => {
    if (!latestKnownVersion) return;
    void applyUpdateNow(latestKnownVersion, "manual");
  }, [applyUpdateNow, latestKnownVersion]);

  const onRestartNow = useCallback(() => {
    if (!isDesktop || restartingApp) return;
    setRestartingApp(true);
    void desktopRestartApp()
      .catch((err: unknown) => {
        dispatchUi({
          type: "apply_failed",
          message: err instanceof Error ? err.message : "Failed to restart app.",
        });
      })
      .finally(() => {
        setRestartingApp(false);
      });
  }, [isDesktop, restartingApp]);

  const requestUpdateOnNextIdle = useCallback(() => {
    if (latestKnownVersion) {
      setIdleUpdateVersions((prev) => {
        const next = new Set(prev);
        next.add(latestKnownVersion);
        writeIdleUpdateVersions(next);
        return next;
      });
      snoozeVersionPrompt(latestKnownVersion);
    }
  }, [latestKnownVersion, snoozeVersionPrompt]);

  const releaseNotesUrl = useMemo(
    () => `https://ctx.rs/release-notes/${encodeURIComponent(latest)}`,
    [latest],
  );

  const onForcedUpdateNow = useCallback(() => {
    const version = latestKnownVersion || minimumSupportedVersion || latest;
    if (!version) return;
    void applyUpdateNow(version, "forced");
  }, [applyUpdateNow, latest, latestKnownVersion, minimumSupportedVersion]);

  const shouldRenderBanner = shouldShow || applyingUpdate || restartRequired || forcedUpdateNeedsManualInstall;
  if (!forcedUpdate && !shouldRenderBanner && !showInfoModal) return null;
  const restartActionEnabled = restartRequired && isDesktop;
  const updateActionDisabled = applyingUpdate || (restartRequired && (!restartActionEnabled || restartingApp));
  const updateActionLabel = applyingUpdate
    ? "Updating..."
    : restartRequired
      ? restartingApp
        ? "Restarting..."
        : "Restart app to finish update"
      : "Update Now";

  return (
    <>
      {forcedUpdate ? (
        <div className="daemon-overlay wb-update-required-overlay" role="dialog" aria-modal="true" aria-label="Update required">
          <div className="daemon-overlay-card wb-update-required-card">
            <div className="daemon-overlay-eyebrow">Update required</div>
            <h2>Update required to continue</h2>
            <p className="daemon-overlay-body">
              This version is no longer supported. Install the latest update now to continue using ctx.
            </p>
            <div className="daemon-overlay-target">
              Installed: <span className="daemon-overlay-mono">{updateInfo?.current_version ?? "unknown"}</span>
            </div>
            <div className="daemon-overlay-target">
              Minimum supported: <span className="daemon-overlay-mono">{minimumSupportedVersion || "unknown"}</span>
            </div>
            {latestKnownVersion ? (
              <div className="daemon-overlay-target">
                Latest: <span className="daemon-overlay-mono">{latestKnownVersion}</span>
              </div>
            ) : null}
            {updateError ? <div className="daemon-overlay-error">{updateError}</div> : null}
            <div className="daemon-overlay-actions">
              <button
                type="button"
                className="daemon-overlay-button"
                disabled={updateActionDisabled}
                onClick={restartRequired ? onRestartNow : onForcedUpdateNow}
              >
                {updateActionLabel}
              </button>
            </div>
          </div>
        </div>
      ) : shouldRenderBanner ? (
        <div className="wb-snackbar wb-update-snackbar" role="status" aria-live="polite" data-testid="update-available-snackbar">
          <div className="wb-snackbar-body wb-update-snackbar-body">
            <div className="wb-snackbar-title wb-update-snackbar-title-row">
              <span>Update available: {latest}.</span>
              <button
                type="button"
                className="wb-update-snackbar-info-btn"
                aria-label="Learn about update timing"
                title="Learn about update timing"
                onClick={() => dispatchUi({ type: "info_opened" })}
              >
                <Info size={14} aria-hidden="true" />
              </button>
            </div>
            <div className="wb-snackbar-subtitle">
              <a
                href={releaseNotesUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="wb-update-release-notes-link"
              >
                View release notes
              </a>
            </div>
            {effectiveError ? <div className="wb-snackbar-error">{effectiveError}</div> : null}
          </div>
          <div className="wb-snackbar-actions wb-update-snackbar-actions">
            <button
              type="button"
              className="wb-snackbar-btn"
              disabled={updateActionDisabled}
              onClick={restartRequired ? onRestartNow : onUpdateNow}
            >
              {updateActionLabel}
            </button>
            <button
              type="button"
              className="wb-snackbar-btn wb-snackbar-btn-secondary"
              disabled={applyingUpdate || restartRequired}
              onClick={requestUpdateOnNextIdle}
            >
              Update on Next Idle
            </button>
          </div>
          <button
            type="button"
            className="wb-snackbar-close"
            onClick={dismissForLater}
            aria-label="Dismiss update notice"
            disabled={applyingUpdate || restartRequired}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      ) : null}
      {showInfoModal ? (
        <div
          className="modal-overlay"
          role="dialog"
          aria-modal="true"
          aria-label="Update timing info"
          onClick={() => dispatchUi({ type: "info_closed" })}
        >
          <div className="modal wb-update-info-modal" onClick={(event) => event.stopPropagation()}>
            <h3>Update timing</h3>
            <p>
              Updating immediately will interrupt any actively running tasks. All your work will be saved, but you&apos;ll have to
              resume any agents directly.
            </p>
            <p>
              If you select to update on next idle, the app will wait for the next time that all tasks are idle and then trigger
              the update.
            </p>
            <div className="modal-actions">
              <button type="button" className="wb-snackbar-btn" onClick={() => dispatchUi({ type: "info_closed" })}>
                Close
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}
