import { defineConfig } from "playwright/test";
import os from "os";

const HOST = "127.0.0.1";
const PORT = process.env.CTX_E2E_PORT ?? "4401";
const baseURL = `http://${HOST}:${PORT}`;
const dataDir =
  process.env.CTX_E2E_DATA_DIR ?? `${os.tmpdir()}/ctx-e2e-${process.pid}`;
const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";
process.env.CTX_E2E_DATA_DIR ??= dataDir;
process.env.CTX_E2E_AUTH_TOKEN ??= AUTH_TOKEN;
const authSetupCommand = `node -e 'const fs = require(\\"fs\\"); const path = require(\\"path\\"); const dir = ${JSON.stringify(
  dataDir,
).replace(/"/g, '\\"')}; fs.mkdirSync(dir, { recursive: true }); fs.writeFileSync(path.join(dir, \\"daemon_auth.json\\"), JSON.stringify({ token: ${JSON.stringify(
  AUTH_TOKEN,
).replace(/"/g, '\\"')} }, null, 2));'`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: Number(process.env.PW_WORKERS ?? 1),
  use: {
    baseURL,
    extraHTTPHeaders: {
      authorization: `Bearer ${AUTH_TOKEN}`,
    },
    headless: true,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  webServer: {
    url: baseURL,
    command:
      `bash -lc "rm -rf ${dataDir} && ${authSetupCommand} && pnpm -C apps/web build && CTX_SHOW_FAKE_PROVIDER=1 CTX_STORAGE_BACKEND=sqlite cargo run -p ctx-http --bin ctx -- serve --bind ${HOST}:${PORT} --data-dir ${dataDir}"`,
    cwd: "../..",
    reuseExistingServer: false,
    timeout: 300_000,
  },
});
