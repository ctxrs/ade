import { defineConfig } from "playwright/test";
import crypto from "crypto";
import net from "net";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const HOST = "127.0.0.1";
const DEFAULT_PORT = 4401;
const parseBool = (value?: string) =>
  ["1", "true", "yes", "on"].includes(value?.toLowerCase() ?? "");
const reuseExistingServer = parseBool(process.env.CTX_E2E_REUSE_SERVER);
const skipWebBuild = parseBool(process.env.CTX_E2E_SKIP_WEB_BUILD);
const resolvePort = async () => {
  const requestedPort = Number(process.env.CTX_E2E_PORT);
  if (Number.isFinite(requestedPort) && requestedPort > 0) {
    return requestedPort;
  }
  if (reuseExistingServer) {
    return DEFAULT_PORT;
  }
  return await new Promise<number>((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, HOST, () => {
      const address = server.address();
      if (address && typeof address === "object") {
        const { port } = address;
        server.close(() => resolve(port));
        return;
      }
      server.close(() => reject(new Error("Failed to resolve free port")));
    });
  });
};
const PORT = await resolvePort();
process.env.CTX_E2E_PORT = String(PORT);
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
).replace(/"/g, '\\"')} }, null, 2)); fs.writeFileSync(path.join(dir, \\"settings.json\\"), JSON.stringify({ execution: { mode: \\"host\\" } }, null, 2));'`;
const docsMirrorBin = path.resolve(__dirname, "e2e/fixtures/ctx-docs-mirror-fixture.sh");
const docsMirrorBinEscaped = docsMirrorBin.replace(/"/g, '\\"');
const cargoTargetDir =
  process.env.CTX_E2E_CARGO_TARGET_DIR ??
  path.join(
    os.tmpdir(),
    `ctx-e2e-cargo-${crypto.createHash("sha1").update(path.resolve(__dirname, "../..")).digest("hex").slice(0, 10)}`,
  );
const outputDir = path.resolve(__dirname, "e2e/test-results");
const reportDir = path.resolve(__dirname, "e2e/playwright-report");
const primaryReporter = process.env.CTX_E2E_REPORTER ?? "dot";

const resolveWorkers = () => {
  const raw = String(process.env.CTX_E2E_WORKERS ?? process.env.PW_WORKERS ?? "").trim();
  if (!raw) return 1;
  if (raw.toLowerCase() === "auto") return undefined;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? n : 1;
};

const resolveWebServerStdio = () => {
  const raw = String(process.env.CTX_E2E_WEB_SERVER_STDIO ?? "").trim().toLowerCase();
  if (!raw) return {} as const;
  if (raw === "ignore") return { stdout: "ignore" as const, stderr: "ignore" as const };
  if (raw === "pipe") return { stdout: "pipe" as const, stderr: "pipe" as const };
  return {} as const;
};

const webBuildCommand = skipWebBuild ? "true" : "pnpm -C apps/web build";
const webServerCommand =
  `bash -lc "rm -rf ${dataDir} && ${authSetupCommand} && ${webBuildCommand} && ` +
  `CTX_DOCS_MIRROR_BIN=\\"${docsMirrorBinEscaped}\\" CARGO_TARGET_DIR=${cargoTargetDir} ` +
  `CTX_EXECUTION_MODE=host CTX_SHOW_FAKE_PROVIDER=1 CTX_STORAGE_BACKEND=sqlite ` +
  `cargo run -p ctx-http --bin ctx -- serve --bind ${HOST}:${PORT} --data-dir ${dataDir}"`;

export default defineConfig({
  testDir: "./e2e",
  timeout: 60_000,
  workers: resolveWorkers(),
  outputDir,
  reporter: [
    [primaryReporter],
    ["html", { outputFolder: reportDir, open: "never" }],
  ],
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
    command: webServerCommand,
    cwd: "../..",
    reuseExistingServer,
    timeout: 1_200_000,
    ...resolveWebServerStdio(),
  },
});
