import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { Info, X } from "lucide-react";
import { applyAppImageUpdate, downloadAppImageUpdate, type UpdateCheck } from "../api/client";
import {
  desktopApplyAppUpdate,
  desktopGetAppUpdateState,
  desktopRestartApp,
  isDesktopApp,
  type DesktopAppUpdateStateResp,
} from "../utils/desktop";
import { readCachedUpdateCheck, refreshUpdateCheck } from "../utils/updateNotice";
import { REQUEST_UPDATE_CHECK_EVENT } from "../utils/desktopMenuCommands";
import {
  UPDATER_REFRESH_BROADCAST_STORAGE_KEY,
  writeUpdaterRefreshBroadcast,
} from "../utils/updaterEvents";

const PROMPT_SNOOZE_STORAGE_KEY = "ctx_update_prompt_next_allowed_at_v1";
const IDLE_UPDATE_VERSION_STORAGE_KEY = "ctx_update_prompt_idle_versions_v1";
const AUTO_APPLY_ON_LAUNCH_STORAGE_KEY = "ctx_update_auto_apply_on_launch_v1";
const RESTART_REQUIRED_VERSION_STORAGE_KEY = "ctx_update_restart_required_version_v1";
const POLL_INTERVAL_MS = 60 * 60 * 1000;
const PROMPT_SNOOZE_MS = 24 * 60 * 60 * 1000;
const RESTART_READY_MESSAGE = "Update takes ~1 second and preserves data. Active agents will be paused.";

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
  status: string | null;
  infoModalOpen: boolean;
};

type NoticeUiAction =
  | { type: "apply_started" }
  | { type: "apply_failed"; message: string }
  | { type: "restart_failed"; message: string }
  | { type: "apply_completed" }
  | { type: "check_failed"; message: string }
  | { type: "check_recovered" }
  | { type: "restart_required"; message: string }
  | { type: "info_opened" }
  | { type: "info_closed" };

const initialNoticeUiState: NoticeUiState = {
  phase: "ready",
  error: null,
  status: null,
  infoModalOpen: false,
};

const noticeUiReducer = (state: NoticeUiState, action: NoticeUiAction): NoticeUiState => {
  switch (action.type) {
    case "apply_started":
      return { ...state, phase: "applying", error: null, status: null };
    case "apply_failed":
      return { ...state, phase: "ready", error: action.message, status: null };
    case "restart_failed":
      return {
        ...state,
        phase: "restart_required",
        error: action.message,
        status: RESTART_READY_MESSAGE,
      };
    case "apply_completed":
      return { ...state, phase: "ready", error: null, status: null };
    case "check_failed":
      if (state.phase === "restart_required") return state;
      return { ...state, phase: "ready", error: action.message, status: null };
    case "check_recovered":
      if (state.phase !== "ready") return state;
      if (!state.error) return state;
      return { ...state, error: null };
    case "restart_required":
      return { ...state, phase: "restart_required", error: null, status: action.message };
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

const messageFromUnknownError = (err: unknown, fallback: string): string => {
  if (err instanceof Error) {
    const message = String(err.message ?? "").trim();
    return message || fallback;
  }
  if (typeof err === "string") {
    const message = err.trim();
    return message || fallback;
  }
  if (err && typeof err === "object") {
    const withMessage = err as { message?: unknown };
    if (typeof withMessage.message === "string") {
      const message = withMessage.message.trim();
      if (message) return message;
    }
    try {
      const encoded = JSON.stringify(err);
      if (typeof encoded === "string" && encoded.trim()) return encoded;
    } catch {
      // ignore serialization failures
    }
  }
  return fallback;
};

const deriveBaseUrlFromEndpoint = (endpoint: string): string => {
  const trimmed = String(endpoint ?? "").trim();
  if (!trimmed) return "";
  try {
    const url = new URL(trimmed);
    const marker = "/releases/";
    const idx = url.pathname.indexOf(marker);
    if (idx >= 0) {
      url.pathname = url.pathname.slice(0, idx);
    } else {
      url.pathname = "";
    }
    url.search = "";
    url.hash = "";
    return `${url.origin}${url.pathname}`.replace(/\/+$/, "");
  } catch {
    return "";
  }
};

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
  const isDesktop = isDesktopApp();
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(() => (isDesktop ? null : readCachedUpdateCheck()));
  const [desktopNativeState, setDesktopNativeState] = useState<DesktopAppUpdateStateResp | null>(null);
  const [promptSnoozeByVersion, setPromptSnoozeByVersion] = useState<Record<string, number>>(
    () => readPromptSnoozeByVersion(),
  );
  const [idleUpdateVersions, setIdleUpdateVersions] = useState<Set<string>>(() => readIdleUpdateVersions());
  const [uiState, dispatchUi] = useReducer(noticeUiReducer, initialNoticeUiState);
  const [restartingApp, setRestartingApp] = useState(false);
  const launchAutoAppliedVersionRef = useRef<string>("");
  const applyInFlightRef = useRef(false);
  const updateInfoRef = useRef<UpdateCheck | null>(updateInfo);
  const nativeStateSignatureRef = useRef<string>("");
  const autoApplyOnLaunchEnabled = shouldAutoApplyOnLaunch();

  useEffect(() => {
    updateInfoRef.current = updateInfo;
  }, [updateInfo]);

  const latest = (updateInfo?.latest_version ?? "").trim() || "unknown";
  const latestKnownVersion = (updateInfo?.latest_version ?? "").trim();
  const minimumSupportedVersion = (updateInfo?.min_supported_version ?? "").trim();
  const nextPromptAtMs = latestKnownVersion ? Number(promptSnoozeByVersion[latestKnownVersion] ?? 0) : 0;
  const nowMs = Date.now();
  const desktopPhase = normalizeOptionalString(desktopNativeState?.phase).toLowerCase();
  const desktopStagedReady = isDesktop && (
    desktopNativeState?.staged === true
    || desktopPhase === "staged_ready"
    || (Boolean(desktopNativeState) && !desktopPhase && Boolean(updateInfo?.update_available))
  );
  const desktopStaging = isDesktop && desktopPhase === "staging";
  const inPlaceCapability = getInPlaceCapability(updateInfo);
  const canApplyFromCurrentClient = isDesktop || inPlaceCapability.supported;
  const forcedUpdateNeedsManualInstall = isForcedUpdate(updateInfo) && !canApplyFromCurrentClient;
  const shouldShow = isDesktop
    ? uiState.phase === "restart_required"
    : Boolean(updateInfo?.update_available) && nowMs >= nextPromptAtMs;
  const forcedUpdate = isForcedUpdate(updateInfo) && canApplyFromCurrentClient;
  const applyingUpdate = uiState.phase === "applying";
  const restartRequired = uiState.phase === "restart_required";
  const showInfoModal = uiState.infoModalOpen;
  const updateError = uiState.error;
  const updateStatus = uiState.status;
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
    writeUpdaterRefreshBroadcast("dismiss-for-later");
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
        message: RESTART_READY_MESSAGE,
      });
      return true;
    },
    [clearRestartRequiredVersionState],
  );

  const refresh = useCallback(
    async (force = false): Promise<UpdateCheck | null> => {
      let info: UpdateCheck | null = null;
      if (isDesktop) {
        let daemonPolicy: UpdateCheck | null = null;
        try {
          daemonPolicy = await refreshUpdateCheck(force ? { force: true } : undefined);
        } catch {
          daemonPolicy = null;
        }
        try {
          const native = await desktopGetAppUpdateState("stable");
          setDesktopNativeState(native);
          const latestVersion = normalizeOptionalString(native.latest_version) || null;
          const nativeSignature = [
            normalizeOptionalString(native.current_version),
            normalizeOptionalString(native.latest_version),
            normalizeOptionalString(native.phase).toLowerCase(),
            native.restart_required ? "1" : "0",
            native.available ? "1" : "0",
            native.staged ? "1" : "0",
          ].join("|");
          if (nativeSignature !== nativeStateSignatureRef.current) {
            nativeStateSignatureRef.current = nativeSignature;
            writeUpdaterRefreshBroadcast("native-state-change");
          }
          info = {
            channel: "stable",
            base_url: deriveBaseUrlFromEndpoint(native.endpoint),
            platform:
              normalizeOptionalString(native.target)
              || null,
            current_version: normalizeOptionalString(native.current_version),
            latest_version: latestVersion,
            min_supported_version: normalizeOptionalString(daemonPolicy?.min_supported_version) || null,
            platform_supported: daemonPolicy?.platform_supported ?? true,
            in_place_update_supported: Boolean(native.configured),
            in_place_update_reason: native.configured
              ? normalizeOptionalString(native.last_error) || null
              : normalizeOptionalString(native.message) || "Native updater is not configured.",
            update_available: Boolean(native.available),
          };
          if (native.restart_required) {
            dispatchUi({
              type: "restart_required",
              message: RESTART_READY_MESSAGE,
            });
          } else {
            const nativePhase = normalizeOptionalString(native.phase).toLowerCase();
            const nativeLastError = normalizeOptionalString(native.last_error);
            const nativeMessage = normalizeOptionalString(native.message);
            if (nativeLastError) {
              dispatchUi({
                type: "check_failed",
                message: nativeLastError,
              });
            } else if (nativePhase === "failed" && nativeMessage) {
              dispatchUi({
                type: "check_failed",
                message: nativeMessage,
              });
            } else {
              dispatchUi({ type: "check_recovered" });
            }
          }
        } catch (err) {
          setDesktopNativeState(null);
          const reason = messageFromUnknownError(err, "Desktop updater check failed.");
          const previous = updateInfoRef.current;
          info = previous
            ? {
              ...previous,
              update_available: false,
              in_place_update_reason: reason,
            }
            : {
              channel: "stable",
              base_url: normalizeOptionalString(daemonPolicy?.base_url),
              platform:
                normalizeOptionalString(daemonPolicy?.platform)
                || null,
              current_version: normalizeOptionalString(daemonPolicy?.current_version),
              latest_version: null,
              min_supported_version: normalizeOptionalString(daemonPolicy?.min_supported_version) || null,
              platform_supported: daemonPolicy?.platform_supported ?? true,
              in_place_update_supported: false,
              in_place_update_reason: reason,
              update_available: false,
            };
            dispatchUi({
              type: "check_failed",
              message: reason,
            });
        }
      } else {
        info = await refreshUpdateCheck(force ? { force: true } : undefined);
      }
      if (info) {
        updateInfoRef.current = info;
        setUpdateInfo((prev) => (areUpdateChecksEqual(prev, info) ? prev : info));
      }
      const effectiveInfo = info ?? updateInfoRef.current;
      if (!isDesktop) {
        reconcileRestartRequiredState(effectiveInfo);
      }
      return effectiveInfo;
    },
    [isDesktop, reconcileRestartRequiredState],
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
          message: message || RESTART_READY_MESSAGE,
        });
        writeUpdaterRefreshBroadcast("apply-needs-restart");
      };
      applyInFlightRef.current = true;
      dispatchUi({ type: "apply_started" });
      try {
        if (isDesktop) {
          const resp = await desktopApplyAppUpdate("stable");
          if (resp.needs_restart) {
            markRestartRequired(resp.message);
            return true;
          }
          if (resp.applied || resp.up_to_date) {
            clearVersionFlags(version);
            clearRestartRequiredVersionState();
            await refresh(true);
            dispatchUi({ type: "apply_completed" });
            writeUpdaterRefreshBroadcast("apply-complete");
            return true;
          }
          dispatchUi({ type: "apply_failed", message: resp.message || "Update did not apply." });
          recoverIdleFailure();
          return false;
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
      } catch (err: unknown) {
        dispatchUi({ type: "apply_failed", message: messageFromUnknownError(err, "Failed to apply update.") });
        recoverIdleFailure();
        return false;
      } finally {
        applyInFlightRef.current = false;
      }
    },
    [
      clearRestartRequiredVersionState,
      clearVersionFlags,
      isDesktop,
      refresh,
      setRestartRequiredVersionState,
      snoozeVersionPrompt,
    ],
  );

  useEffect(() => {
    let cancelled = false;
    const runInitialCheck = async () => {
        if (!isDesktop) {
          const pendingRestartVersion = readRestartRequiredVersion();
          if (pendingRestartVersion) {
            setRestartRequiredVersionState(pendingRestartVersion);
            dispatchUi({
              type: "restart_required",
              message: RESTART_READY_MESSAGE,
            });
          }
        }
      // Startup must always perform a real update check request.
      await refresh(true);
      if (cancelled) return;
    };
    void runInitialCheck();
    const intervalId = window.setInterval(() => {
      void refresh(true);
    }, POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(intervalId);
    };
  }, [isDesktop, refresh, setRestartRequiredVersionState]);

  useEffect(() => {
    const onRequestUpdateCheck = () => {
      void refresh(true);
    };
    window.addEventListener(REQUEST_UPDATE_CHECK_EVENT, onRequestUpdateCheck as EventListener);
    return () => {
      window.removeEventListener(REQUEST_UPDATE_CHECK_EVENT, onRequestUpdateCheck as EventListener);
    };
  }, [refresh]);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.storageArea !== window.localStorage) return;
      if (event.key === UPDATER_REFRESH_BROADCAST_STORAGE_KEY) {
        void refresh(true);
        return;
      }
      if (event.key === PROMPT_SNOOZE_STORAGE_KEY) {
        setPromptSnoozeByVersion(readPromptSnoozeByVersion());
        return;
      }
      if (event.key === IDLE_UPDATE_VERSION_STORAGE_KEY) {
        setIdleUpdateVersions(readIdleUpdateVersions());
      }
    };
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener("storage", onStorage);
    };
  }, [refresh]);

  useEffect(() => {
    if (!isDesktop) return;
    if (!autoApplyOnLaunchEnabled) return;
    if (restartRequired) return;
    if (!desktopStagedReady) return;
    const version = latestKnownVersion;
    if (!version) return;
    if (isForcedUpdate(updateInfo)) return;
    if (readRestartRequiredVersion()) return;
    if (launchAutoAppliedVersionRef.current === version) return;
    launchAutoAppliedVersionRef.current = version;
    void applyUpdateNow(version, "launch_auto");
  }, [
    applyUpdateNow,
    autoApplyOnLaunchEnabled,
    desktopStagedReady,
    isDesktop,
    latestKnownVersion,
    restartRequired,
    updateInfo,
  ]);

  useEffect(() => {
    if (!isDesktop) return;
    if (!desktopStaging) return;
    const timer = window.setTimeout(() => {
      void refresh(true);
    }, 4000);
    return () => {
      window.clearTimeout(timer);
    };
  }, [desktopStaging, isDesktop, refresh]);

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
          type: "restart_failed",
          message: messageFromUnknownError(err, "Failed to restart app."),
        });
      })
      .finally(() => {
        setRestartingApp(false);
      });
  }, [isDesktop, restartingApp]);

  useEffect(() => {
    if (!isDesktop) return;
    if (!restartRequired) return;
    if (!allTasksIdle) return;
    if (restartingApp) return;
    const restartVersion = latestKnownVersion || readRestartRequiredVersion();
    if (!restartVersion) return;
    if (!idleUpdateVersions.has(restartVersion)) return;
    setIdleUpdateVersions((prev) => {
      if (!prev.has(restartVersion)) return prev;
      const next = new Set(prev);
      next.delete(restartVersion);
      writeIdleUpdateVersions(next);
      return next;
    });
    void onRestartNow();
  }, [
    allTasksIdle,
    idleUpdateVersions,
    isDesktop,
    latestKnownVersion,
    onRestartNow,
    restartRequired,
    restartingApp,
  ]);

  const requestUpdateOnNextIdle = useCallback(() => {
    const version = latestKnownVersion || readRestartRequiredVersion();
    if (version) {
      setIdleUpdateVersions((prev) => {
        const next = new Set(prev);
        next.add(version);
        writeIdleUpdateVersions(next);
        return next;
      });
      if (!restartRequired) {
        snoozeVersionPrompt(version);
      }
      writeUpdaterRefreshBroadcast("schedule-next-idle");
    }
  }, [latestKnownVersion, restartRequired, snoozeVersionPrompt]);

  const releaseNotesUrl = useMemo(
    () => `https://ctx.rs/release-notes/${encodeURIComponent(latest)}`,
    [latest],
  );

  const onForcedUpdateNow = useCallback(() => {
    const version = latestKnownVersion || minimumSupportedVersion || latest;
    if (!version) return;
    void applyUpdateNow(version, "forced");
  }, [applyUpdateNow, latest, latestKnownVersion, minimumSupportedVersion]);

  const shouldRenderBanner = forcedUpdateNeedsManualInstall || shouldShow || (!isDesktop && (applyingUpdate || restartRequired));
  if (!forcedUpdate && !shouldRenderBanner && !showInfoModal) return null;
  const restartActionEnabled = restartRequired && isDesktop;
  const updateActionDisabled = applyingUpdate || (restartRequired && (!restartActionEnabled || restartingApp));
  const updateActionLabel = applyingUpdate ? "Updating..." : "Update Now";

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
              <span>{`Update available: ${latest}.`}</span>
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
            {updateStatus ? <div className="wb-snackbar-subtitle">{updateStatus}</div> : null}
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
              disabled={applyingUpdate}
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
