import fs from "fs";
import os from "os";
import path from "path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import mkcert from "vite-plugin-mkcert";

type DaemonAuthFile = {
  token?: string;
  daemon_url?: string;
};

const loadDaemonAuth = (): DaemonAuthFile | null => {
  const dataDir = process.env.CTX_DATA_DIR ?? path.join(os.homedir(), ".ctx");
  const authPath = path.join(dataDir, "daemon_auth.json");
  try {
    const raw = fs.readFileSync(authPath, "utf8");
    return JSON.parse(raw) as DaemonAuthFile;
  } catch {
    return null;
  }
};
const httpsHosts = String(process.env.CTX_DEV_HTTPS_HOSTS ?? "")
  .split(",")
  .map((host) => host.trim())
  .filter(Boolean);

export default defineConfig(({ command }) => {
  const auth = command === "serve" ? loadDaemonAuth() : null;
  const daemonUrl =
    process.env.CTX_DAEMON_URL ?? auth?.daemon_url ?? "http://127.0.0.1:4399";
  const supabaseUrl = String(process.env.VITE_SUPABASE_URL ?? "").trim();
  const supabaseProxyTarget = String(process.env.CTX_SUPABASE_PROXY_TARGET ?? "").trim();
  const devPort = Number(process.env.CTX_WEB_PORT ?? 5173);
  const useHttps =
    process.env.CTX_DEV_HTTPS === "1"
      ? true
      : process.env.CTX_DEV_HTTP === "1"
        ? false
        : !supabaseUrl.startsWith("http://");

  if (command === "serve" && auth?.token) {
    process.env.VITE_CTX_AUTH_TOKEN ??= auth.token;
    process.env.VITE_CTX_DAEMON_URL ??= daemonUrl;
  }

  const supabaseProxy =
    supabaseProxyTarget.length > 0
      ? {
          "/auth": { target: supabaseProxyTarget, changeOrigin: true, secure: false },
          "/rest": { target: supabaseProxyTarget, changeOrigin: true, secure: false },
          "/storage": { target: supabaseProxyTarget, changeOrigin: true, secure: false },
          "/functions": { target: supabaseProxyTarget, changeOrigin: true, secure: false },
          "/realtime": { target: supabaseProxyTarget, changeOrigin: true, secure: false, ws: true },
          "/graphql": { target: supabaseProxyTarget, changeOrigin: true, secure: false },
        }
      : {};

  return {
    plugins: [
      react(),
      ...(useHttps
        ? [mkcert(httpsHosts.length > 0 ? { hosts: httpsHosts } : undefined)]
        : []),
    ],
    test: {
      globals: true,
      environment: "jsdom",
      setupFiles: "./vitest.setup.ts",
      exclude: ["e2e/**", "node_modules/**"],
    },
    server: {
      host: "0.0.0.0",
      port: Number.isFinite(devPort) ? devPort : 5173,
      strictPort: true,
      https: useHttps,
      proxy: {
        "/api": {
          target: daemonUrl,
          changeOrigin: true,
          ws: true,
          xfwd: true,
        },
        // Web sessions are served by the daemon; proxy for dev server parity.
        "/sessions": {
          target: daemonUrl,
          changeOrigin: true,
          ws: true,
          xfwd: true,
        },
        ...supabaseProxy,
      },
    },
    preview: {
      host: "0.0.0.0",
      https: useHttps,
    },
  };
});
