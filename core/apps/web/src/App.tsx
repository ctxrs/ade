import { useEffect, useRef } from "react";
import { BrowserRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { appendDesktopLog, openLogsFolder } from "./api/client";
import DaemonAvailabilityOverlay from "./components/DaemonAvailabilityOverlay";
import LauncherPage from "./pages/LauncherPage";
import AppSettingsPage from "./pages/AppSettingsPage";
import WorkspacesPage from "./pages/WorkspacesPage";
import WorkbenchPage from "./pages/WorkbenchPage";
import CursorDiffDemoPage from "./pages/CursorDiffDemoPage";
import ProvidersPage from "./pages/ProvidersPage";
import DiagnosticsPage from "./pages/DiagnosticsPage";
import SettingsPage from "./pages/SettingsPage";
import WorkspaceSetupPage from "./pages/WorkspaceSetupPage";
import CrashCoursePage from "./pages/CrashCoursePage";
import { SessionSupervisorProvider } from "./state/sessionSupervisor";
import { SettingsStoreProvider, useSettingsSnapshot } from "./state/settingsStore";
import { preloadHarnessLogos } from "./utils/harnessCatalog";
import { refreshUpdateCheck } from "./utils/updateNotice";
import {
  desktopListen,
  desktopOpenWorkspaceSetupInNewWindow,
  desktopOpenWorkspaceInNewWindow,
  desktopSetMenuState,
  desktopSetWindowTitle,
  isDesktopApp,
  openExternalLink,
} from "./utils/desktop";
import { initAnalytics, setAnalyticsEnabled, trackAppOpened } from "./utils/analytics";
import { computeAnalyticsCaptureEnabled } from "./utils/analytics/runtimePolicy";
import {
  buildDesktopMenuBaseState,
  DESKTOP_MENU_ACTION_EVENT,
  isDesktopMenuCommandId,
  parseWorkspaceIdFromPathname,
  WEB_MENU_TRACE_EVENT,
  WEB_MENU_COMMAND_EVENT,
  WEB_MENU_STATE_EVENT,
  type DesktopMenuActionEventPayload,
  type DesktopMenuItemState,
  type WebMenuTraceDetail,
  type WebMenuCommandDetail,
  type WebMenuStateDetail,
} from "./utils/desktopMenuCommands";

function settingsTargetForPath(pathname: string): string {
  if (pathname.startsWith("/workspaces/")) {
    const wsId = pathname.split("/")[2];
    if (wsId) {
      return `/settings?ws=${encodeURIComponent(wsId)}`;
    }
  }
  return "/settings";
}

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

function DesktopMenuBridge() {
  const navigate = useNavigate();
  const location = useLocation();
  const patchRef = useRef<DesktopMenuItemState[]>([]);
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
        case "go.workspace-setup":
          navigate("/workspace-setup");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-workspace-setup" });
          return;
        case "file.open-workspaces":
        case "go.workspaces":
          navigate("/workspaces");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-workspaces" });
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
        case "help.diagnostics":
          navigate("/diagnostics");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-diagnostics" });
          return;
        case "go.agent-harnesses":
          navigate("/settings#agent_harnesses");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-agent-harnesses" });
          return;
        case "help.crash-course":
        case "help.keyboard-shortcuts":
          navigate("/crash-course");
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "navigate-crash-course" });
          return;
        case "help.open-logs-folder":
          void openLogsFolder().catch(() => {});
          emitMenuTrace({ commandId, layer: "app", status: "handled", note: "open-logs-folder" });
          return;
        case "file.open-workspace-new-window": {
          const wsId = parseWorkspaceIdFromPathname(location.pathname);
          if (wsId) {
            void desktopOpenWorkspaceInNewWindow(wsId).catch(() => {});
            emitMenuTrace({ commandId, layer: "app", status: "handled", note: "open-workspace-window" });
          } else {
            emitMenuTrace({ commandId, layer: "app", status: "ignored", note: "workspace-missing" });
          }
          return;
        }
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
      void openExternalLink("https://github.com/context-labs/ctx/issues/new").catch(() => {});
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
  const snapshot = useSettingsSnapshot();
  const appOpenedSentRef = useRef(false);

  useEffect(() => {
    const enabled = computeAnalyticsCaptureEnabled({
      settingsLoaded: snapshot.loaded,
      telemetryEnabled: snapshot.settings?.telemetry?.enabled ?? true,
      isDev: import.meta.env.DEV,
      devCaptureFlag: import.meta.env.VITE_POSTHOG_CAPTURE_IN_DEV,
    });
    setAnalyticsEnabled(enabled);
    if (!appOpenedSentRef.current && enabled) {
      appOpenedSentRef.current = true;
      trackAppOpened();
    }
  }, [snapshot.loaded, snapshot.settings?.telemetry?.enabled]);

  return null;
}

export default function App() {
  useEffect(() => {
    appendDesktopLog("ui: app loaded").catch(() => {});
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
          <AnalyticsSettingsBridge />
          <DesktopSettingsListener />
          <DesktopMenuBridge />
          <Routes>
            <Route path="/" element={<LauncherPage />} />
            <Route path="/crash-course" element={<CrashCoursePage />} />
            <Route path="/workspace-setup" element={<WorkspaceSetupPage />} />
            <Route path="/app-settings" element={<AppSettingsPage />} />
            <Route path="/workspaces" element={<WorkspacesPage />} />
            <Route path="/settings" element={<SettingsPage />} />
            <Route path="/providers" element={<ProvidersPage />} />
            <Route path="/diagnostics" element={<DiagnosticsPage />} />
            <Route path="/workspaces/:id" element={<WorkbenchPage />} />
            <Route path="/__cursor_diff_demo" element={<CursorDiffDemoPage />} />
          </Routes>
          <DaemonAvailabilityOverlay />
        </BrowserRouter>
      </SettingsStoreProvider>
    </SessionSupervisorProvider>
  );
}
