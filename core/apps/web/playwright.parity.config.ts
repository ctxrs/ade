import { defineConfig } from "playwright/test";
import crypto from "crypto";
import os from "os";

const HOST = "127.0.0.1";
const PORT = process.env.CTX_E2E_PORT ?? "4403";
const baseURL = `http://${HOST}:${PORT}`;
const dataDir =
  process.env.CTX_E2E_DATA_DIR ?? `${os.tmpdir()}/ctx-e2e-parity-${process.pid}`;
const cargoHash = crypto
  .createHash("sha1")
  .update(process.cwd())
  .digest("hex")
  .slice(0, 10);
const cargoTargetDir =
  process.env.CTX_E2E_CARGO_TARGET_DIR ?? `${os.tmpdir()}/ctx-e2e-cargo-parity-${cargoHash}`;

// Keep the parity spec self-contained: it reads the daemon auth token from the
// same data dir that the webServer writes to.
process.env.CTX_E2E_PORT ??= PORT;
process.env.CTX_E2E_DATA_DIR ??= dataDir;
process.env.CTX_E2E_CARGO_TARGET_DIR ??= cargoTargetDir;

export default defineConfig({
  testDir: "./e2e",
  timeout: 300_000,
  workers: 1,
  retries: 0,
  use: {
    baseURL,
    headless: true,
    viewport: { width: 1280, height: 720 },
    deviceScaleFactor: 1,
    screenshot: "off",
    trace: "off",
    video: "off",
  },
  webServer: {
    url: baseURL,
    command:
      `bash -lc "rm -rf ${dataDir} && pnpm -C apps/web build && CTX_SHOW_FAKE_PROVIDER=1 CARGO_TARGET_DIR=${cargoTargetDir} cargo run --locked --manifest-path Cargo.toml -p ctx-http --bin ctx -- serve --bind ${HOST}:${PORT} --data-dir ${dataDir}"`,
    cwd: "../..",
    reuseExistingServer: false,
    timeout: 300_000,
  },
});
