import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import mkcert from "vite-plugin-mkcert";

const daemonUrl = process.env.CTX_DAEMON_URL ?? "http://127.0.0.1:4399";
const httpsHosts = String(process.env.CTX_DEV_HTTPS_HOSTS ?? "")
  .split(",")
  .map((host) => host.trim())
  .filter(Boolean);

export default defineConfig({
  plugins: [react(), mkcert(httpsHosts.length > 0 ? { hosts: httpsHosts } : undefined)],
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: "./vitest.setup.ts",
    exclude: ["e2e/**", "node_modules/**"],
  },
  server: {
    host: "0.0.0.0",
    port: 5173,
    https: true,
    proxy: {
      "/api": {
        target: daemonUrl,
        changeOrigin: true,
        ws: true,
      },
    },
  },
  preview: {
    host: "0.0.0.0",
    https: true,
  },
});
