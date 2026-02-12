import { useEffect, useRef } from "react";
import { BrowserRouter, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { appendDesktopLog } from "./api/client";
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
import { SettingsStoreProvider } from "./state/settingsStore";
import { preloadHarnessLogos } from "./utils/harnessCatalog";
import { initStatsig } from "./utils/statsig";
import { refreshUpdateCheck } from "./utils/updateNotice";
import { desktopListen, isDesktopApp } from "./utils/desktop";

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
      const path = locationRef.current.pathname;
      let target = "/settings";
      if (path.startsWith("/workspaces/")) {
        const wsId = path.split("/")[2];
        if (wsId) {
          target = `/settings?ws=${encodeURIComponent(wsId)}`;
        }
      }
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
    initStatsig();
  }, []);

  useEffect(() => {
    refreshUpdateCheck().catch(() => {});
  }, []);

  return (
    <SessionSupervisorProvider>
      <SettingsStoreProvider>
        <BrowserRouter>
          <DesktopSettingsListener />
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
