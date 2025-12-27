import { useEffect } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { appendDesktopLog } from "./api/client";
import LauncherPage from "./pages/LauncherPage";
import AppSettingsPage from "./pages/AppSettingsPage";
import WorkspacesPage from "./pages/WorkspacesPage";
import WorkspacePage from "./pages/WorkspacePage";
import WorkbenchPage from "./pages/WorkbenchPage";
import TaskPage from "./pages/TaskPage";
import SessionPage from "./pages/SessionPage";
import CursorDiffDemoPage from "./pages/CursorDiffDemoPage";
import ProvidersPage from "./pages/ProvidersPage";
import DiagnosticsPage from "./pages/DiagnosticsPage";
import SettingsPage from "./pages/SettingsPage";
import FilePreviewPage from "./pages/FilePreviewPage";
import { SessionSupervisorProvider } from "./state/sessionSupervisor";

export default function App() {
  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const token = params.get("token");
    if (token) {
      sessionStorage.setItem("contextAuthToken", token);
      params.delete("token");
      const next =
        window.location.pathname +
        (params.toString() ? `?${params.toString()}` : "") +
        window.location.hash;
      window.history.replaceState({}, "", next);
    }
  }, []);

  useEffect(() => {
    appendDesktopLog("ui: app loaded").catch(() => {});
  }, []);

  return (
    <SessionSupervisorProvider>
      <BrowserRouter>
        <Routes>
          <Route path="/" element={<LauncherPage />} />
          <Route path="/app-settings" element={<AppSettingsPage />} />
          <Route path="/workspaces" element={<WorkspacesPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="/providers" element={<ProvidersPage />} />
          <Route path="/diagnostics" element={<DiagnosticsPage />} />
          <Route path="/file" element={<FilePreviewPage />} />
          <Route path="/workspaces/:id" element={<WorkbenchPage />} />
          <Route path="/workspaces/:id/legacy" element={<WorkspacePage />} />
          <Route path="/tasks/:id" element={<TaskPage />} />
          <Route path="/sessions/:id" element={<SessionPage />} />
          <Route path="/__cursor_diff_demo" element={<CursorDiffDemoPage />} />
        </Routes>
      </BrowserRouter>
    </SessionSupervisorProvider>
  );
}
