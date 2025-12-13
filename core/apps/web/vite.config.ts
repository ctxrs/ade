import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const daemonUrl = process.env.CONTEXT_DAEMON_URL ?? "http://127.0.0.1:4399";

export default defineConfig({
  plugins: [react()],
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: "./vitest.setup.ts",
    exclude: ["e2e/**", "node_modules/**"],
  },
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: daemonUrl,
        changeOrigin: true,
        ws: true,
      },
    },
  },
});
