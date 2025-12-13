import { useEffect } from "react";
import { BrowserRouter, Route, Routes } from "react-router-dom";
import { appendDesktopLog } from "./api/client";
import WorkspacesPage from "./pages/WorkspacesPage";
import WorkspacePage from "./pages/WorkspacePage";
import TaskPage from "./pages/TaskPage";
import SessionPage from "./pages/SessionPage";
import ProvidersPage from "./pages/ProvidersPage";
import DiagnosticsPage from "./pages/DiagnosticsPage";

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
    <BrowserRouter>
      <Routes>
        <Route path="/" element={<WorkspacesPage />} />
        <Route path="/providers" element={<ProvidersPage />} />
        <Route path="/diagnostics" element={<DiagnosticsPage />} />
        <Route path="/workspaces/:id" element={<WorkspacePage />} />
        <Route path="/tasks/:id" element={<TaskPage />} />
        <Route path="/sessions/:id" element={<SessionPage />} />
      </Routes>
    </BrowserRouter>
  );
}
