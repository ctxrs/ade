import { defineConfig } from "playwright/test";
import os from "os";

const HOST = "127.0.0.1";
const PORT = process.env.CTX_E2E_PORT ?? process.env.CONTEXT_E2E_PORT ?? "4401";
const baseURL = `http://${HOST}:${PORT}`;
const dataDir =
  process.env.CTX_E2E_DATA_DIR ??
  process.env.CONTEXT_E2E_DATA_DIR ??
  `${os.tmpdir()}/ctx-e2e-${process.pid}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  use: {
    baseURL,
    headless: true,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  webServer: {
    url: baseURL,
    command:
      `bash -lc "rm -rf ${dataDir} && pnpm -C apps/web build && CTX_SHOW_FAKE_PROVIDER=1 CONTEXT_SHOW_FAKE_PROVIDER=1 cargo run -p ctx-http --bin ctx -- serve --bind ${HOST}:${PORT} --data-dir ${dataDir}"`,
    cwd: "../..",
    reuseExistingServer: false,
    timeout: 300_000,
  },
});
