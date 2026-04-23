import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import {
  applyAppImageUpdate,
  downloadAppImageUpdate,
  type UpdateCheck,
} from "../../api/client";
import {
  desktopApplyAppUpdate,
  desktopGetAppUpdateState,
  desktopRestartApp,
  isDesktopApp,
  type DesktopAppUpdateStateResp,
} from "../../utils/desktop";
import {
  DESKTOP_UPDATE_MENU_STATE_EVENT,
  REQUEST_UPDATE_CHECK_EVENT,
  REQUEST_UPDATE_RESTART_EVENT,
  type DesktopUpdateMenuState,
  type DesktopUpdateMenuStateDetail,
} from "../../utils/desktopMenuCommands";
import { readCachedUpdateCheck, refreshUpdateCheck } from "../../utils/updateNotice";
import {
  UPDATER_REFRESH_BROADCAST_STORAGE_KEY,
  writeUpdaterRefreshBroadcast,
} from "../../utils/updaterEvents";
import {
  IDLE_UPDATE_VERSION_STORAGE_KEY,
  POLL_INTERVAL_MS,
  PROMPT_SNOOZE_MS,
  PROMPT_SNOOZE_STORAGE_KEY,
  RESTART_READY_MESSAGE,
} from "./constants";
import {
  initialNoticeUiState,
  noticeUiReducer,
  type UpdateApplySource,
} from "./state";
import {
  clearRestartRequiredVersion,
  readIdleUpdateVersions,
  readPromptSnoozeByVersion,
  readRestartRequiredVersion,
  writeIdleUpdateVersions,
  writePromptSnoozeByVersion,
  writeRestartRequiredVersion,
} from "./storage";
import {
  areUpdateChecksEqual,
  deriveBaseUrlFromEndpoint,
  getInPlaceCapability,
  isCurrentVersionAtOrAbove,
  isForcedUpdate,
  messageFromUnknownError,
  normalizeOptionalString,
} from "./version";

export type UpdateNoticeBannerProps = {
  allTasksIdle?: boolean;
};

export type UpdateNoticeBannerModel = {
  applyingUpdate: boolean;
  effectiveError: string | null;
  forcedUpdate: boolean;
  latest: string;
  latestKnownVersion: string;
  minimumSupportedVersion: string;
  releaseNotesUrl: string;
  restartRequired: boolean;
  shouldRenderBanner: boolean;
  showInfoModal: boolean;
  showUpdateActions: boolean;
  snackbarTitle: string;
  updateActionDisabled: boolean;
  updateActionLabel: string;
  updateError: string | null;
  updateInfo: UpdateCheck | null;
  updateStatus: string | null;
  dismissForLater: () => void;
  onForcedUpdateNow: () => void;
  onRestartNow: () => void;
  onUpdateNow: () => void;
  openInfoModal: () => void;
  closeInfoModal: () => void;
  requestUpdateOnNextIdle: () => void;
};

export function useUpdateNoticeBanner({
  allTasksIdle = true,
}: UpdateNoticeBannerProps): UpdateNoticeBannerModel {
  const isDesktop = isDesktopApp();
  const [updateInfo, setUpdateInfo] = useState<UpdateCheck | null>(() =>
    isDesktop ? null : readCachedUpdateCheck(),
  );
  const [desktopNativeState, setDesktopNativeState] =
    useState<DesktopAppUpdateStateResp | null>(null);
  const [promptSnoozeByVersion, setPromptSnoozeByVersion] = useState<
    Record<string, number>
  >(() => readPromptSnoozeByVersion());
  const [idleUpdateVersions, setIdleUpdateVersions] = useState<Set<string>>(() =>
    readIdleUpdateVersions(),
  );
  const [uiState, dispatchUi] = useReducer(noticeUiReducer, initialNoticeUiState);
  const [restartingApp, setRestartingApp] = useState(false);
  const [desktopRefreshGeneration, setDesktopRefreshGeneration] = useState(0);
  const applyInFlightRef = useRef(false);
  const manualCheckInFlightRef = useRef(false);
  const updateInfoRef = useRef<UpdateCheck | null>(updateInfo);
  const desktopNativeStateRef = useRef<DesktopAppUpdateStateResp | null>(desktopNativeState);
  const nativeStateSignatureRef = useRef<string>("");

  useEffect(() => {
    updateInfoRef.current = updateInfo;
  }, [updateInfo]);

  useEffect(() => {
    desktopNativeStateRef.current = desktopNativeState;
  }, [desktopNativeState]);

  const latest = (updateInfo?.latest_version ?? "").trim() || "unknown";
  const latestKnownVersion = (updateInfo?.latest_version ?? "").trim();
  const minimumSupportedVersion = (updateInfo?.min_supported_version ?? "").trim();
  const nextPromptAtMs = latestKnownVersion
    ? Number(promptSnoozeByVersion[latestKnownVersion] ?? 0)
    : 0;
  const nowMs = Date.now();
  const desktopPhase = normalizeOptionalString(desktopNativeState?.phase).toLowerCase();
  const desktopStagedReady =
    isDesktop &&
    (desktopNativeState?.staged === true || desktopPhase === "staged_ready");
  const desktopStaging = isDesktop && desktopPhase === "staging";
  const manualTransient =
    uiState.phase === "checking" ||
    uiState.phase === "manual_installing" ||
    uiState.phase === "up_to_date" ||
    uiState.phase === "manual_failed";
  const inPlaceCapability = getInPlaceCapability(updateInfo);
  const canApplyFromCurrentClient = isDesktop || inPlaceCapability.supported;
  const forcedUpdateNeedsManualInstall =
    isForcedUpdate(updateInfo) && !canApplyFromCurrentClient;
  const shouldShow = isDesktop
    ? uiState.phase === "restart_required" || manualTransient
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
  const desktopUpdateMenuState: DesktopUpdateMenuState = restartRequired
    ? "restart"
    : desktopStaging ||
        (isDesktop &&
          (applyingUpdate ||
            uiState.phase === "checking" ||
            uiState.phase === "manual_installing"))
      ? "downloading"
      : "check";

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
      if (
        currentVersion &&
        isCurrentVersionAtOrAbove(currentVersion, pendingRestartVersion)
      ) {
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
          const native = await desktopGetAppUpdateState();
          desktopNativeStateRef.current = native;
          setDesktopNativeState(native);
          setDesktopRefreshGeneration((prev) => prev + 1);
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
            channel: normalizeOptionalString(daemonPolicy?.channel) || "stable",
            base_url: deriveBaseUrlFromEndpoint(native.endpoint),
            platform: normalizeOptionalString(native.target) || null,
            current_version: normalizeOptionalString(native.current_version),
            latest_version: latestVersion,
            min_supported_version:
              normalizeOptionalString(daemonPolicy?.min_supported_version) || null,
            platform_supported: daemonPolicy?.platform_supported ?? true,
            in_place_update_supported: Boolean(native.configured),
            in_place_update_reason: native.configured
              ? normalizeOptionalString(native.last_error) || null
              : normalizeOptionalString(native.message) ||
                "Native updater is not configured.",
            update_available: Boolean(
              native.available ||
                normalizeOptionalString(native.phase).toLowerCase() === "staging" ||
                normalizeOptionalString(native.phase).toLowerCase() === "staged_ready",
            ),
          };
          if (native.restart_required) {
            if (latestVersion) {
              setRestartRequiredVersionState(latestVersion);
            }
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
          desktopNativeStateRef.current = null;
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
                channel: normalizeOptionalString(daemonPolicy?.channel) || "stable",
                base_url: normalizeOptionalString(daemonPolicy?.base_url),
                platform: normalizeOptionalString(daemonPolicy?.platform) || null,
                current_version: normalizeOptionalString(daemonPolicy?.current_version),
                latest_version: null,
                min_supported_version:
                  normalizeOptionalString(daemonPolicy?.min_supported_version) || null,
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
    async (
      version: string,
      source: UpdateApplySource = "manual",
    ): Promise<boolean> => {
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
          const resp = await desktopApplyAppUpdate();
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
          dispatchUi({
            type: "apply_failed",
            message: resp.message || "Update did not apply.",
          });
          recoverIdleFailure();
          return false;
        }

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
        const updateChannel =
          normalizeOptionalString(updateInfoRef.current?.channel) || undefined;
        const downloadResp = await downloadAppImageUpdate(updateChannel);
        if (!downloadResp.can_apply_in_place) {
          dispatchUi({
            type: "apply_failed",
            message:
              "This install cannot update in place. Install the latest version from release notes, then relaunch.",
          });
          recoverIdleFailure();
          return false;
        }
        const resp = await applyAppImageUpdate(updateChannel);
        if (!resp.applied) {
          dispatchUi({
            type: "apply_failed",
            message: resp.message || "Update did not apply.",
          });
          recoverIdleFailure();
          return false;
        }
        markRestartRequired(resp.message);
        return true;
      } catch (err: unknown) {
        dispatchUi({
          type: "apply_failed",
          message: messageFromUnknownError(err, "Failed to apply update."),
        });
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
      if (manualCheckInFlightRef.current) return;
      manualCheckInFlightRef.current = true;
      dispatchUi({ type: "manual_check_started" });
      void (async () => {
        try {
          const info = await refresh(true);
          const native = desktopNativeStateRef.current;
          if (native?.restart_required) return;
          const nativePhase = normalizeOptionalString(native?.phase).toLowerCase();
          const nativeError = normalizeOptionalString(native?.last_error || native?.message);
          if (native && !native.configured) {
            dispatchUi({
              type: "manual_failed",
              message: nativeError || "Native updater is not configured.",
            });
            return;
          }
          const refreshError =
            !native && !info?.update_available
              ? normalizeOptionalString(info?.in_place_update_reason)
              : "";
          if (refreshError) {
            dispatchUi({ type: "manual_failed", message: refreshError });
            return;
          }
          if (nativeError && nativePhase === "failed") {
            dispatchUi({ type: "manual_failed", message: nativeError });
            return;
          }
          if (
            nativePhase === "staging" ||
            nativePhase === "staged_ready" ||
            info?.update_available
          ) {
            dispatchUi({
              type: "manual_installing",
              message: "Update found. Installing in background...",
            });
            return;
          }
          dispatchUi({
            type: "manual_up_to_date",
            message: "You're up to date.",
          });
        } catch (err: unknown) {
          dispatchUi({
            type: "manual_failed",
            message: messageFromUnknownError(err, "Update check failed."),
          });
        } finally {
          manualCheckInFlightRef.current = false;
        }
      })();
    };
    window.addEventListener(
      REQUEST_UPDATE_CHECK_EVENT,
      onRequestUpdateCheck as EventListener,
    );
    return () => {
      window.removeEventListener(
        REQUEST_UPDATE_CHECK_EVENT,
        onRequestUpdateCheck as EventListener,
      );
    };
  }, [refresh]);

  useEffect(() => {
    if (uiState.phase !== "up_to_date" && uiState.phase !== "manual_failed") return;
    const timer = window.setTimeout(() => {
      dispatchUi({ type: "apply_completed" });
    }, 5000);
    return () => {
      window.clearTimeout(timer);
    };
  }, [uiState.phase]);

  useEffect(() => {
    if (!isDesktop) return;
    window.dispatchEvent(
      new CustomEvent<DesktopUpdateMenuStateDetail>(
        DESKTOP_UPDATE_MENU_STATE_EVENT,
        {
          detail: { state: desktopUpdateMenuState },
        },
      ),
    );
  }, [desktopUpdateMenuState, isDesktop]);

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
    if (restartRequired) return;
    if (!desktopStagedReady) return;
    const version = latestKnownVersion;
    if (!version) return;
    if (isForcedUpdate(updateInfo)) return;
    if (readRestartRequiredVersion()) return;
    void applyUpdateNow(version, "desktop_auto");
  }, [
    applyUpdateNow,
    desktopRefreshGeneration,
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
  }, [
    allTasksIdle,
    applyUpdateNow,
    forcedUpdate,
    idleUpdateVersions,
    latestKnownVersion,
    updateInfo?.update_available,
  ]);

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
    const onRequestUpdateRestart = () => {
      onRestartNow();
    };
    window.addEventListener(
      REQUEST_UPDATE_RESTART_EVENT,
      onRequestUpdateRestart as EventListener,
    );
    return () => {
      window.removeEventListener(
        REQUEST_UPDATE_RESTART_EVENT,
        onRequestUpdateRestart as EventListener,
      );
    };
  }, [onRestartNow]);

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

  const shouldRenderBanner =
    forcedUpdateNeedsManualInstall ||
    shouldShow ||
    (!isDesktop && (applyingUpdate || restartRequired));
  const restartActionEnabled = restartRequired && isDesktop;
  const updateActionDisabled =
    applyingUpdate || (restartRequired && (!restartActionEnabled || restartingApp));
  const updateActionLabel = restartRequired
    ? "Relaunch"
    : applyingUpdate
      ? "Updating..."
      : "Update Now";
  const snackbarTitle = restartRequired
    ? `Ready to relaunch: ${latest}.`
    : uiState.phase === "checking"
      ? "Checking for updates..."
      : uiState.phase === "manual_installing"
        ? "Update found. Installing in background..."
        : uiState.phase === "up_to_date"
          ? "You're up to date."
          : uiState.phase === "manual_failed"
            ? "Update check failed."
            : `Update available: ${latest}.`;
  const showUpdateActions =
    restartRequired || (!isDesktop && Boolean(updateInfo?.update_available));

  return {
    applyingUpdate,
    effectiveError,
    forcedUpdate,
    latest,
    latestKnownVersion,
    minimumSupportedVersion,
    releaseNotesUrl,
    restartRequired,
    shouldRenderBanner,
    showInfoModal,
    showUpdateActions,
    snackbarTitle,
    updateActionDisabled,
    updateActionLabel,
    updateError,
    updateInfo,
    updateStatus,
    dismissForLater,
    onForcedUpdateNow,
    onRestartNow,
    onUpdateNow,
    openInfoModal: () => dispatchUi({ type: "info_opened" }),
    closeInfoModal: () => dispatchUi({ type: "info_closed" }),
    requestUpdateOnNextIdle,
  };
}
