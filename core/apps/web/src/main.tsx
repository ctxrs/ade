import React from "react";
import ReactDOM from "react-dom/client";
import { applyContextTheme } from "@ctx/design/web";
import App from "./App";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

applyContextTheme();

// Bootstrap auth token before React mounts so first-render data fetches are
// authenticated (avoids an initial 401 flash/race between effects).
if (typeof window !== "undefined") {
  const params = new URLSearchParams(window.location.search);
  const token = params.get("token");
  if (token) {
    try {
      sessionStorage.setItem("ctxAuthToken", token);
    } catch {
      // ignore
    }
    params.delete("token");
    const next =
      window.location.pathname +
      (params.toString() ? `?${params.toString()}` : "") +
      window.location.hash;
    window.history.replaceState({}, "", next);
  }
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
