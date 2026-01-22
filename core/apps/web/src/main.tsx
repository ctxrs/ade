import React from "react";
import ReactDOM from "react-dom/client";
import { applyContextTheme } from "@ctx/design/web";
import App from "./App";
import { initLoadTestTelemetry } from "./utils/loadTestTelemetry";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

applyContextTheme();
initLoadTestTelemetry();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
