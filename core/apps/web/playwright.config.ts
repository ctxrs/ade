import { defineConfig } from "playwright/test";
import crypto from "crypto";
import os from "os";
import { execSync } from "child_process";

const HOST = "127.0.0.1";
const resolvePort = (): string => {
  if (process.env.CTX_E2E_PORT) {
    return process.env.CTX_E2E_PORT;
  }
  try {
    const port = execSync(
      "node -e \"const net=require('net');const s=net.createServer();s.listen(0,'127.0.0.1',()=>{const p=s.address().port;s.close(()=>process.stdout.write(String(p)));});\""
    )
      .toString()
      .trim();
    if (port) return port;
  } catch {
    // Fall back to a fixed port if the helper fails.
  }
  return "4401";
};
const PORT = resolvePort();
const baseURL = `http://${HOST}:${PORT}`;
const dataDir =
  process.env.CTX_E2E_DATA_DIR ?? `${os.tmpdir()}/ctx-e2e-${process.pid}`;
if (!process.env.CTX_E2E_DATA_DIR) {
  process.env.CTX_E2E_DATA_DIR = dataDir;
}
if (!process.env.CTX_E2E_PORT) {
  process.env.CTX_E2E_PORT = PORT;
}
const cargoHash = crypto
  .createHash("sha1")
  .update(process.cwd())
  .digest("hex")
  .slice(0, 10);
const cargoTargetDir =
  process.env.CTX_E2E_CARGO_TARGET_DIR ?? `${os.tmpdir()}/ctx-e2e-cargo-${cargoHash}`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: Number(process.env.PW_WORKERS ?? 1),
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
      `bash -lc "rm -rf ${dataDir} && pnpm -C apps/web build && CTX_SHOW_FAKE_PROVIDER=1 CARGO_TARGET_DIR=${cargoTargetDir} cargo run --locked --manifest-path Cargo.toml -p ctx-http --bin ctx -- serve --bind ${HOST}:${PORT} --data-dir ${dataDir}"`,
    cwd: "../..",
    reuseExistingServer: false,
    timeout: 300_000,
  },
});
