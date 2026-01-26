import { useEffect } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { appendDesktopLog, getDaemonBaseUrl, setDaemonBaseUrl } from "./api/client";
import DaemonAvailabilityOverlay from "./components/DaemonAvailabilityOverlay";
import LauncherPage from "./pages/LauncherPage";
import AppSettingsPage from "./pages/AppSettingsPage";
import WorkspacesPage from "./pages/WorkspacesPage";
import WorkbenchPage from "./pages/WorkbenchPage";
import CursorDiffDemoPage from "./pages/CursorDiffDemoPage";
import ProvidersPage from "./pages/ProvidersPage";
import DiagnosticsPage from "./pages/DiagnosticsPage";
import SettingsPage from "./pages/SettingsPage";
import { SessionSupervisorProvider } from "./state/sessionSupervisor";
import { SettingsStoreProvider } from "./state/settingsStore";
import { preloadHarnessLogos } from "./utils/harnessCatalog";

export default function App() {
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const token = params.get("token");
    if (token) {
      sessionStorage.setItem("ctxAuthToken", token);
      params.delete("token");
      const next =
        window.location.pathname +
        (params.toString() ? `?${params.toString()}` : "") +
        window.location.hash;
      window.history.replaceState({}, "", next);
    }
    const envToken = import.meta.env.VITE_CTX_AUTH_TOKEN;
    const envDaemonUrl = import.meta.env.VITE_CTX_DAEMON_URL;
    if (!envToken && !envDaemonUrl) return;
    const host = typeof window === "undefined" ? "" : String(window.location.hostname ?? "").toLowerCase();
    const loopback = host === "localhost" || host === "::1" || host.startsWith("127.");
    if (!loopback) return;

    if (envToken && !sessionStorage.getItem("ctxAuthToken")) {
      sessionStorage.setItem("ctxAuthToken", envToken);
    }
    if (envDaemonUrl && !getDaemonBaseUrl()) {
      setDaemonBaseUrl(envDaemonUrl, true);
    }
  }, []);

  useEffect(() => {
    appendDesktopLog("ui: app loaded").catch(() => {});
  }, []);

  useEffect(() => {
    preloadHarnessLogos();
  }, []);

  return (
    <SessionSupervisorProvider>
      <SettingsStoreProvider>
        <BrowserRouter>
          <Routes>
            <Route path="/" element={<LauncherPage />} />
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
