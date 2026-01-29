import React from "react";
import ReactDOM from "react-dom/client";
import { applyContextTheme } from "@ctx/design/web";
import App from "./App";
import { initLoadTestTelemetry } from "./utils/loadTestTelemetry";
import { initWalRecorder } from "./utils/walRecorder";
import { initTheme } from "./utils/theme";
import { authToken, getDaemonBaseUrl, setDaemonAuthToken, setDaemonBaseUrl } from "./api/client";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

const primeAuthSession = () => {
  if (typeof window === "undefined") return;
  const params = new URLSearchParams(window.location.search);
  const token = params.get("token");
  if (token) {
    setDaemonAuthToken(token);
    params.delete("token");
    const next =
      window.location.pathname +
      (params.toString() ? `?${params.toString()}` : "") +
      window.location.hash;
    window.history.replaceState({}, "", next);
  }
  if (import.meta.env.DEV) {
    const envToken = import.meta.env.VITE_CTX_AUTH_TOKEN;
    const envDaemonUrl = import.meta.env.VITE_CTX_DAEMON_URL;
    if (envToken && !authToken()) {
      setDaemonAuthToken(envToken);
    }
    if (envDaemonUrl && !getDaemonBaseUrl()) {
      setDaemonBaseUrl(envDaemonUrl, true);
    }
  }
};

primeAuthSession();
initTheme();
applyContextTheme();
initLoadTestTelemetry();
const wal = initWalRecorder();

const app = wal?.onRender ? (
  <React.Profiler id="App" onRender={wal.onRender}>
    <App />
  </React.Profiler>
) : (
  <App />
);

ReactDOM.createRoot(document.getElementById("root")!).render(<React.StrictMode>{app}</React.StrictMode>);
