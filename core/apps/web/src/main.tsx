import React from "react";
import ReactDOM from "react-dom/client";
import { applyContextTheme } from "@context/design/web";
import App from "./App";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

applyContextTheme();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
