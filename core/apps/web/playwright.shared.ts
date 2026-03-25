import { defineConfig, type PlaywrightTestConfig } from "playwright/test";
import crypto from "crypto";
import fs from "fs";
import net from "net";
import os from "os";
import path from "path";
import { fileURLToPath } from "url";
import { parseBoolishString } from "./src/utils/boolish";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const HOST = "127.0.0.1";
const DEFAULT_PORT = 4401;

const parseBool = (value?: string) => parseBoolishString(value) === true;

const hasValue = (value?: string) => Boolean(value?.trim());

const repoHash = crypto.createHash("sha1").update(path.resolve(__dirname, "../..")).digest("hex").slice(0, 10);

const resolveWorkers = (defaultWorkers: number | undefined) => {
  const raw = String(process.env.CTX_E2E_WORKERS ?? process.env.PW_WORKERS ?? "").trim();
  if (!raw) return defaultWorkers;
  if (raw.toLowerCase() === "auto") return undefined;
  const n = Number(raw);
  return Number.isFinite(n) && n > 0 ? n : defaultWorkers;
};

const resolveWebServerStdio = () => {
  const raw = String(process.env.CTX_E2E_WEB_SERVER_STDIO ?? "").trim().toLowerCase();
  if (!raw) return {} as const;
  if (raw === "ignore") return { stdout: "ignore" as const, stderr: "ignore" as const };
  if (raw === "pipe") return { stdout: "pipe" as const, stderr: "pipe" as const };
  return {} as const;
};

export const resolvePlaywrightCargoTargetDir = (env: NodeJS.ProcessEnv) => {
  const configured = String(env.CTX_E2E_CARGO_TARGET_DIR ?? env.CARGO_TARGET_DIR ?? "").trim();
  if (configured) {
    return path.isAbsolute(configured)
      ? configured
      : path.resolve(__dirname, "../..", configured);
  }
  return path.join(os.homedir(), ".cache", "cargo", "ctx-monorepo", `e2e-${repoHash}`);
};

const resolvePort = async (reuseExistingServer: boolean): Promise<number> => {
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

export type E2ESuiteProfile =
  | "all"
  | "premerge_required"
  | "release_required"
  | "cross_platform"
  | "visual"
  | "soak"
  | "load";

export async function createCtxPlaywrightConfig(
  profile: E2ESuiteProfile,
): Promise<PlaywrightTestConfig> {
  const profileSlug = profile.replace(/[^a-z0-9_-]/gi, "-").toLowerCase();
  const reuseRequested = parseBool(process.env.CTX_E2E_REUSE_SERVER);
  const reuseExistingServer =
    profile === "premerge_required" || profile === "release_required" ? false : reuseRequested;
  const skipWebBuild = parseBool(process.env.CTX_E2E_SKIP_WEB_BUILD);

  const PORT = await resolvePort(reuseExistingServer);
  process.env.CTX_E2E_PORT = String(PORT);
  const baseURL = `http://${HOST}:${PORT}`;
  const readinessURL = `${baseURL}/api/health`;

  const dataDir =
    process.env.CTX_E2E_DATA_DIR ?? `${os.tmpdir()}/ctx-e2e-${profileSlug}-${process.pid}`;
  const AUTH_TOKEN = process.env.CTX_E2E_AUTH_TOKEN ?? "ctx-e2e-auth-token";
  process.env.CTX_E2E_DATA_DIR ??= dataDir;
  process.env.CTX_E2E_AUTH_TOKEN ??= AUTH_TOKEN;
  const defaultBundleDir = path.resolve(__dirname, "../desktop/src-tauri/bundles");
  const bundleManifestPath = path.join(defaultBundleDir, "manifest.json");
  const resolvedBundleDir =
    (process.env.CTX_BUNDLE_DIR ?? "").trim()
    || (fs.existsSync(bundleManifestPath) ? defaultBundleDir : "");
  if (resolvedBundleDir) {
    process.env.CTX_BUNDLE_DIR = resolvedBundleDir;
    process.env.CTX_E2E_BUNDLED_ONLY ??= "1";
  }

  const docsMirrorBin = path.resolve(__dirname, "e2e/fixtures/ctx-docs-mirror-fixture.sh");
  const cargoTargetDir = resolvePlaywrightCargoTargetDir(process.env);

  const outputDir = path.resolve(__dirname, `e2e/test-results/${profileSlug}`);
  const reportDir = path.resolve(__dirname, `e2e/playwright-report/${profileSlug}`);
  const primaryReporter = process.env.CTX_E2E_REPORTER ?? "dot";
  const argosEnabled =
    parseBool(process.env.CTX_E2E_ARGOS) || hasValue(process.env.ARGOS_TOKEN);
  const reporter: PlaywrightTestConfig["reporter"] = [
    [primaryReporter],
    ["html", { outputFolder: reportDir, open: "never" }],
  ];
  if (argosEnabled) {
    reporter.push([
      "@argos-ci/playwright/reporter",
      {
        uploadToArgos: true,
        buildName: `ctx-web-${profileSlug}`,
      },
    ]);
  }
  const webServerEnv = {
    ...process.env,
    CTX_E2E_DATA_DIR: dataDir,
    CTX_E2E_AUTH_TOKEN: AUTH_TOKEN,
    CTX_E2E_SKIP_WEB_BUILD: skipWebBuild ? "1" : "0",
    CTX_E2E_HOST: HOST,
    CTX_E2E_PORT: String(PORT),
    CTX_DOCS_MIRROR_BIN: docsMirrorBin,
    CTX_E2E_CARGO_TARGET_DIR: cargoTargetDir,
    CARGO_TARGET_DIR: cargoTargetDir,
    CTX_EXECUTION_MODE: "host",
    CTX_SHOW_FAKE_PROVIDER: "1",
    CTX_DEV_MODE: "1",
    CTX_STORAGE_BACKEND: "sqlite",
    ...(process.platform === "win32" ? {} : { SHELL: "/bin/sh" }),
  };

  return defineConfig({
    testDir: "./e2e",
    timeout: 60_000,
    workers: resolveWorkers(1),
    outputDir,
    reporter,
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
      url: readinessURL,
      command: "node apps/web/scripts/start-e2e-server.mjs",
      cwd: "../..",
      env: webServerEnv,
      reuseExistingServer,
      timeout: 1_200_000,
      ...resolveWebServerStdio(),
    },
  });
}
