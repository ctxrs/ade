import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { BrowserRouter, Navigate, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { appendDesktopLog, getDaemonConnectionReadiness, openLogsFolder } from "./api/client";
import { useDaemonConnection } from "./api/useDaemonConnection";
import DaemonAvailabilityOverlay from "./components/DaemonAvailabilityOverlay";
import StorageGuardBanner from "./components/StorageGuardBanner";
import UpdateNoticeBanner from "./components/UpdateNoticeBanner";
import LauncherPage from "./pages/LauncherPage";
import WorkbenchPage from "./pages/WorkbenchPage";
import CursorDiffDemoPage from "./pages/CursorDiffDemoPage";
import GeometryHarnessPage from "./pages/GeometryHarnessPage";
import ProvidersPage from "./pages/ProvidersPage";
import DiagnosticsPage from "./pages/DiagnosticsPage";
import SettingsPage from "./pages/SettingsPage";
import WorkspaceSetupPage from "./pages/WorkspaceSetupPage";
import { SessionSupervisorProvider } from "./state/sessionSupervisor";
import { SettingsStoreProvider } from "./state/settingsStore";
import { getClientSettingsState, loadClientSettings, subscribeClientSettings } from "./state/clientSettings";
import { setUiDiagnosticPersistenceSink, type UiDiagnosticEvent } from "./state/diagnosticsChannel";
import { loadLauncherRecents } from "./state/launcherRecentsStore";
import { preloadHarnessLogos } from "./utils/harnessCatalog";
import { refreshUpdateCheck } from "./utils/updateNotice";
import {
  desktopCheckAppUpdate,
  desktopListen,
  desktopOpenLauncherInNewWindow,
  desktopOpenWorkspaceSetupInNewWindow,
  desktopSetDockRecentLocalWorkspaces,
  desktopSetMenuState,
  desktopSetWindowTitle,
  isDesktopApp,
  openExternalLink,
} from "./utils/desktop";
import { initializeAppForegroundTracking } from "./utils/windowFocus";
import {
  consumePendingDownloadAttributionId,
  getPendingDownloadAttributionId,
  initAnalytics,
  setAnalyticsEnabled,
  trackAppOpened,
} from "./utils/analytics";
import { computeAnalyticsCaptureEnabled } from "./utils/analytics/runtimePolicy";
import {
  buildDesktopUpdateMenuPatch,
  buildDesktopMenuBaseState,
  DESKTOP_UPDATE_MENU_STATE_EVENT,
  DESKTOP_MENU_ACTION_EVENT,
  isDesktopUpdateMenuState,
  isDesktopMenuCommandId,
  REQUEST_UPDATE_CHECK_EVENT,
  REQUEST_UPDATE_RESTART_EVENT,
  WEB_MENU_TRACE_EVENT,
  WEB_MENU_COMMAND_EVENT,
  WEB_MENU_STATE_EVENT,
  type DesktopMenuActionEventPayload,
  type DesktopMenuItemState,
  type DesktopUpdateMenuState,
  type DesktopUpdateMenuStateDetail,
  type WebMenuTraceDetail,
  type WebMenuCommandDetail,
  type WebMenuStateDetail,
} from "./utils/desktopMenuCommands";
import {
  WORKBENCH_TASK_IDLE_EVENT,
  writeUpdaterRefreshBroadcast,
  type WorkbenchTaskIdleDetail,
} from "./utils/updaterEvents";
import {
  noteDesktopDaemonReady,
  noteDesktopFirstPaint,
  noteDesktopRendererPing,
  noteDesktopRendererTimeout,
  noteDesktopWindowCreated,
} from "./state/foregroundFreshnessTelemetry";

function settingsTargetForPath(pathname: string): string {
  if (pathname.startsWith("/workspaces/")) {
    const wsId = pathname.split("/")[2];
    if (wsId) {
      return `/settings?ws=${encodeURIComponent(wsId)}`;
    }
  }
  return "/settings";
}

const RUNTIME_DIAGNOSTIC_DEDUPE_WINDOW_MS = 15_000;

type WindowWithDesktopStartup = Window & {
  __CTX_DESKTOP_STARTUP__?: {
    windowCreatedAtMs?: unknown;
    windowLabel?: unknown;
    startPath?: unknown;
  };
};

const safeJsonStringify = (value: unknown): string => {
  try {
    return JSON.stringify(value);
  } catch {
    return "[unserializable]";
  }
};

const shouldPersistRuntimeDiagnostic = (event: UiDiagnosticEvent): boolean =>
  event.source === "runtime" && (event.severity === "error" || event.fatal === true);

const shouldPersistUiDiagnostic = (event: UiDiagnosticEvent): boolean => {
  if (shouldPersistRuntimeDiagnostic(event)) return true;
  if (event.source === "foreground_freshness" && event.severity !== "info") return true;
  if (event.source === "desktop_startup") return true;
  return false;
};

const buildRuntimeDiagnosticLogLine = (event: UiDiagnosticEvent): string => {
  const context =
    event.context && Object.keys(event.context).length > 0
      ? ` context=${safeJsonStringify(event.context)}`
      : "";
  return `ui_runtime: code=${event.code} severity=${event.severity} fatal=${event.fatal === true ? "true" : "false"} message=${event.message}${context}`;
};

const buildUiDiagnosticLogLine = (event: UiDiagnosticEvent): string => {
  if (event.source === "runtime") {
    return buildRuntimeDiagnosticLogLine(event);
  }
  const context =
    event.context && Object.keys(event.context).length > 0
      ? ` context=${safeJsonStringify(event.context)}`
      : "";
  return `${event.source}: code=${event.code} severity=${event.severity} message=${event.message}${context}`;
};

function DesktopSettingsListener() {
  const navigate = useNavigate();
  const location = useLocation();
  const locationRef = useRef(location);

  useEffect(() => {
    locationRef.current = location;
  }, [location]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    let active = true;
    let unlisten: (() => void) | null = null;
    desktopListen("desktop_open_settings", () => {
      const target = settingsTargetForPath(locationRef.current.pathname);
      navigate(target);
    })
      .then((fn) => {
        if (!active) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, [navigate]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const handler = (event: Event) => {
      const custom = event as CustomEvent<{ target?: unknown }>;
      const detailTarget = custom.detail?.target;
      if (typeof detailTarget === "string" && detailTarget.startsWith("/settings")) {
        navigate(detailTarget);
        return;
      }
      const target = settingsTargetForPath(locationRef.current.pathname);
      navigate(target);
    };
    window.addEventListener("ctx:open-settings", handler as EventListener);
    return () => {
      window.removeEventListener("ctx:open-settings", handler as EventListener);
    };
  }, [navigate]);

  return null;
}

function ClientSettingsBootstrap() {
  useEffect(() => {
    loadClientSettings().catch((err) => {
      console.warn("client settings bootstrap failed", err);
    });
  }, []);

  return null;
}

function AppForegroundBootstrap() {
  useEffect(() => {
    initializeAppForegroundTracking();
  }, []);

  return null;
}

function DesktopMenuBridge() {
  const navigate = useNavigate();
  const location = useLocation();
  const patchRef = useRef<DesktopMenuItemState[]>([]);
  const updateMenuStateRef = useRef<DesktopUpdateMenuState>("check");
  const updateMenuPatchRef = useRef<DesktopMenuItemState>(buildDesktopUpdateMenuPatch("check"));
  const emitMenuTrace = (detail: WebMenuTraceDetail) => {
    window.dispatchEvent(
      new CustomEvent<WebMenuTraceDetail>(WEB_MENU_TRACE_EVENT, {
        detail,
      }),
    );
  };

  const pushMenuState = useRef(() => {});
  pushMenuState.current = () => {
    if (!isDesktopApp()) return;
    const merged = new Map<string, DesktopMenuItemState>();
    for (const item of buildDesktopMenuBaseState(location.pathname)) {
      merged.set(item.id, { ...item });
    }
    for (const patch of patchRef.current) {
      const prev = merged.get(patch.id);
      merged.set(patch.id, { ...(prev ?? { id: patch.id }), ...patch });
    }
    const updatePatch = updateMenuPatchRef.current;
    const prev = merged.get(updatePatch.id);
    merged.set(updatePatch.id, { ...(prev ?? { id: updatePatch.id }), ...updatePatch });
    void desktopSetMenuState(Array.from(merged.values())).catch(() => {});
  };

  useEffect(() => {
    patchRef.current = [];
    pushMenuState.current();
  }, [location.pathname]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const title = (typeof document !== "undefined" ? document.title : "").trim() || "ctx";
    void desktopSetWindowTitle(title).catch(() => {});
  }, [location.pathname]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    let cancelled = false;
    loadLauncherRecents()
      .then((entries) => {
        if (cancelled) return;
        const localEntries = entries.flatMap((entry) =>
          entry.kind === "local"
            ? [{ label: entry.label, root_path: entry.root_path }]
            : [],
        );
        void desktopSetDockRecentLocalWorkspaces(localEntries).catch(() => {});
      })
      .catch(() => {
        if (cancelled) return;
        void desktopSetDockRecentLocalWorkspaces([]).catch(() => {});
      });
    return () => {
      cancelled = true;
    };
  }, [location.pathname]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const onMenuState = (event: Event) => {
      const custom = event as CustomEvent<WebMenuStateDetail>;
      const detail = custom.detail;
      if (!detail || !Array.isArray(detail.items)) return;

      if (detail.replace === false) {
        const next = new Map<string, DesktopMenuItemState>();
        for (const item of patchRef.current) next.set(item.id, item);
        for (const item of detail.items) next.set(item.id, item);
        patchRef.current = Array.from(next.values());
      } else {
        patchRef.current = detail.items;
      }
      pushMenuState.current();
    };

    window.addEventListener(WEB_MENU_STATE_EVENT, onMenuState as EventListener);
    return () => {
      window.removeEventListener(WEB_MENU_STATE_EVENT, onMenuState as EventListener);
    };
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const onDesktopUpdateMenuState = (event: Event) => {
      const custom = event as CustomEvent<DesktopUpdateMenuStateDetail>;
      const nextState = custom.detail?.state;
      if (!isDesktopUpdateMenuState(nextState)) return;
      updateMenuStateRef.current = nextState;
      updateMenuPatchRef.current = buildDesktopUpdateMenuPatch(nextState);
      pushMenuState.current();
    };

    window.addEventListener(DESKTOP_UPDATE_MENU_STATE_EVENT, onDesktopUpdateMenuState as EventListener);
    return () => {
      window.removeEventListener(DESKTOP_UPDATE_MENU_STATE_EVENT, onDesktopUpdateMenuState as EventListener);
    };
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    let active = true;
    let unlisten: (() => void) | null = null;
    desktopListen<DesktopMenuActionEventPayload>(DESKTOP_MENU_ACTION_EVENT, (payload) => {
      if (!payload || !isDesktopMenuCommandId(payload.commandId)) return;
      const { commandId } = payload;
      switch (commandId) {
        case "file.new-workspace":
          void desktopOpenWorkspaceSetupInNewWindow().catch(() => {});
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "open-workspace-setup-window" });
          return;
        case "file.new-window":
          void desktopOpenLauncherInNewWindow().catch(() => {});
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "open-launcher-window" });
          return;
        case "go.workspace-setup":
          navigate("/workspace-setup");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-workspace-setup" });
          return;
        case "go.launcher":
          navigate("/");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-launcher" });
          return;
        case "go.settings":
          navigate(settingsTargetForPath(location.pathname));
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-settings" });
          return;
        case "go.diagnostics":
          navigate("/diagnostics");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-diagnostics" });
          return;
        case "go.agent-harnesses":
          navigate("/settings#agent_harnesses");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-agent-harnesses" });
          return;
        case "help.keyboard-shortcuts":
          navigate(settingsTargetForPath(location.pathname));
          emitMenuTrace({
            commandId,
            layer: "app",
            status: "handled",
            note: "navigate-settings-keyboard-shortcuts",
          });
          return;
        case "help.check-for-updates":
          if (updateMenuStateRef.current === "restart") {
            window.dispatchEvent(new Event(REQUEST_UPDATE_RESTART_EVENT));
            emitMenuTrace({
              commandId,
              layer: "app",
              status: "handled",
              note: "request-update-restart",
            });
            return;
          }
          if (updateMenuStateRef.current === "downloading") {
            emitMenuTrace({
              commandId,
              layer: "app",
              status: "ignored",
              note: "update-download-in-progress",
            });
            return;
          }
          window.dispatchEvent(new Event(REQUEST_UPDATE_CHECK_EVENT));
          writeUpdaterRefreshBroadcast("menu-check-for-updates");
          void refreshUpdateCheck({ force: true }).catch(() => {});
          void desktopCheckAppUpdate("stable").catch(() => {});
          emitMenuTrace({
            commandId,
            layer: "app",
            status: "handled",
            note: "trigger-silent-update-check",
          });
          return;
        case "help.open-logs-folder":
          void openLogsFolder().catch(() => {});
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "open-logs-folder" });
          return;
        default:
          break;
      }

      window.dispatchEvent(
        new CustomEvent<WebMenuCommandDetail>(WEB_MENU_COMMAND_EVENT, {
          detail: { commandId },
        }),
      );
      emitMenuTrace({ commandId, layer: "app", status: "forwarded" });
    })
      .then((fn) => {
        if (!active) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch(() => {});
    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, [location.pathname, navigate]);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const onMenuCommand = (event: Event) => {
      const custom = event as CustomEvent<WebMenuCommandDetail>;
      const detail = custom.detail;
      if (!detail || !isDesktopMenuCommandId(detail.commandId)) return;
      if (detail.commandId !== "help.report-issue") return;
      void openExternalLink("https://github.com/ctxrs/ctx/issues/new").catch(() => {});
      emitMenuTrace({
        commandId: detail.commandId,
        layer: "app",
        status: "handled",
        note: "open-report-issue-url",
      });
    };
    window.addEventListener(WEB_MENU_COMMAND_EVENT, onMenuCommand as EventListener);
    return () => {
      window.removeEventListener(WEB_MENU_COMMAND_EVENT, onMenuCommand as EventListener);
    };
  }, []);

  return null;
}

function AnalyticsSettingsBridge() {
  const clientSettingsState = useSyncExternalStore(
    subscribeClientSettings,
    getClientSettingsState,
    getClientSettingsState,
  );
  const appOpenedSentRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    const enabled = computeAnalyticsCaptureEnabled({
      settingsLoaded: clientSettingsState.loaded,
      telemetryEnabled: clientSettingsState.settings.telemetry.clientEnabled,
      isDev: import.meta.env.DEV,
      devCaptureFlag: import.meta.env.VITE_POSTHOG_CAPTURE_IN_DEV,
    });
    setAnalyticsEnabled(enabled);
    if (!appOpenedSentRef.current && enabled) {
      void (async () => {
        let downloadId: string | null = null;
        try {
          downloadId = await getPendingDownloadAttributionId();
        } catch {
          downloadId = null;
        }
        if (cancelled) return;
        if (appOpenedSentRef.current) return;
        appOpenedSentRef.current = true;
        trackAppOpened(downloadId ? { downloadId } : undefined);
        if (downloadId) {
          void consumePendingDownloadAttributionId().catch(() => {});
        }
      })();
    }
    return () => {
      cancelled = true;
    };
  }, [clientSettingsState.loaded, clientSettingsState.settings.telemetry.clientEnabled]);

  return null;
}

function GlobalUpdateNotice() {
  const location = useLocation();
  const [allTasksIdle, setAllTasksIdle] = useState(false);

  useEffect(() => {
    const onWorkbenchTaskIdle = (event: Event) => {
      const custom = event as CustomEvent<WorkbenchTaskIdleDetail>;
      const detail = custom.detail;
      if (!detail || typeof detail.allTasksIdle !== "boolean") return;
      setAllTasksIdle(detail.allTasksIdle);
    };
    window.addEventListener(WORKBENCH_TASK_IDLE_EVENT, onWorkbenchTaskIdle as EventListener);
    return () => {
      window.removeEventListener(WORKBENCH_TASK_IDLE_EVENT, onWorkbenchTaskIdle as EventListener);
    };
  }, []);

  useEffect(() => {
    if (!location.pathname.startsWith("/workspaces/")) {
      // Outside workbench, idle state is unknown; avoid auto-idle update actions.
      setAllTasksIdle(false);
    }
  }, [location.pathname]);

  return <UpdateNoticeBanner allTasksIdle={allTasksIdle} />;
}

export default function App() {
  const runtimeLogDedupRef = useRef<Map<string, number>>(new Map());
  const desktopFirstPaintLoggedRef = useRef(false);
  const desktopDaemonReadyLoggedRef = useRef(false);
  const daemonConnection = useDaemonConnection();

  useEffect(() => {
    appendDesktopLog("ui: app loaded").catch(() => {});
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    const startup = (window as WindowWithDesktopStartup).__CTX_DESKTOP_STARTUP__;
    const windowLabel =
      typeof startup?.windowLabel === "string" && startup.windowLabel.trim().length > 0
        ? startup.windowLabel.trim()
        : "unknown";
    const startupPath =
      typeof startup?.startPath === "string" && startup.startPath.trim().length > 0
        ? startup.startPath.trim()
        : window.location.pathname;
    const createdAtMs =
      typeof startup?.windowCreatedAtMs === "number" && Number.isFinite(startup.windowCreatedAtMs)
        ? startup.windowCreatedAtMs
        : null;
    if (createdAtMs !== null) {
      noteDesktopWindowCreated(createdAtMs);
    }
    noteDesktopRendererPing();
    void appendDesktopLog(
      `desktop_startup: renderer_ping label=${safeJsonStringify(windowLabel)} path=${safeJsonStringify(startupPath)}`,
    ).catch(() => {});
    const timeoutId = window.setTimeout(() => {
      if (desktopFirstPaintLoggedRef.current) return;
      noteDesktopRendererTimeout();
      void appendDesktopLog(
        `desktop_startup: renderer_timeout label=${safeJsonStringify(windowLabel)} path=${safeJsonStringify(window.location.pathname)} readyState=${safeJsonStringify(document.readyState)} visibilityState=${safeJsonStringify(document.visibilityState)}`,
        "error",
      ).catch(() => {});
    }, 1000);
    const rafId = window.requestAnimationFrame(() => {
      desktopFirstPaintLoggedRef.current = true;
      noteDesktopFirstPaint();
      void appendDesktopLog(
        `desktop_startup: first_paint label=${safeJsonStringify(windowLabel)} path=${safeJsonStringify(window.location.pathname)}`,
      ).catch(() => {});
    });
    return () => {
      window.clearTimeout(timeoutId);
      window.cancelAnimationFrame(rafId);
    };
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    if (!getDaemonConnectionReadiness(daemonConnection).isReady) return;
    if (desktopDaemonReadyLoggedRef.current) return;
    desktopDaemonReadyLoggedRef.current = true;
    const startup = (window as WindowWithDesktopStartup).__CTX_DESKTOP_STARTUP__;
    const windowLabel =
      typeof startup?.windowLabel === "string" && startup.windowLabel.trim().length > 0
        ? startup.windowLabel.trim()
        : "unknown";
    noteDesktopDaemonReady();
    void appendDesktopLog(
      `desktop_startup: daemon_ready label=${safeJsonStringify(windowLabel)} path=${safeJsonStringify(window.location.pathname)}`,
    ).catch(() => {});
  }, [daemonConnection]);

  useEffect(() => {
    if (!isDesktopApp()) {
      setUiDiagnosticPersistenceSink(null);
      return;
    }
    setUiDiagnosticPersistenceSink((event) => {
      if (!shouldPersistUiDiagnostic(event)) return;
      const key = `${event.code}|${event.message}`;
      const now = Date.now();
      const prev = runtimeLogDedupRef.current.get(key);
      if (typeof prev === "number" && now - prev < RUNTIME_DIAGNOSTIC_DEDUPE_WINDOW_MS) {
        return;
      }
      runtimeLogDedupRef.current.set(key, now);
      void appendDesktopLog(buildUiDiagnosticLogLine(event), event.severity).catch(() => {});
    });
    return () => {
      setUiDiagnosticPersistenceSink(null);
      runtimeLogDedupRef.current.clear();
    };
  }, []);

  useEffect(() => {
    preloadHarnessLogos();
  }, []);

  useEffect(() => {
    initAnalytics();
  }, []);

  useEffect(() => {
    refreshUpdateCheck().catch(() => {});
  }, []);

  return (
    <SessionSupervisorProvider>
      <SettingsStoreProvider>
        <BrowserRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
          <AppForegroundBootstrap />
          <ClientSettingsBootstrap />
          <AnalyticsSettingsBridge />
          <DesktopSettingsListener />
          <DesktopMenuBridge />
          <GlobalUpdateNotice />
          <Routes>
            <Route path="/" element={<LauncherPage />} />
            <Route path="/index.html" element={<Navigate replace to="/" />} />
            <Route path="/workspace-setup" element={<WorkspaceSetupPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="/providers" element={<ProvidersPage />} />
            <Route path="/diagnostics" element={<DiagnosticsPage />} />
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
            <Route path="/__cursor_diff_demo" element={<CursorDiffDemoPage />} />
            <Route path="/__geometry_harness" element={<GeometryHarnessPage />} />
          </Routes>
          <StorageGuardBanner />
          <DaemonAvailabilityOverlay />
        </BrowserRouter>
      </SettingsStoreProvider>
    </SessionSupervisorProvider>
  );
}
