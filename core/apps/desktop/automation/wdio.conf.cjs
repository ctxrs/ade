const path = require("path");
const fs = require("fs");
const { spawnSync, spawn } = require("child_process");

const { waitTestRunnerBackendReady } = require("@crabnebula/test-runner-backend");
const { waitTauriDriverReady } = require("@crabnebula/tauri-driver");

const ROOT = path.resolve(__dirname, "..");
const APP_PATH = process.env.CTX_DESKTOP_APP_PATH ||
  path.resolve(ROOT, "src-tauri/target/debug/bundle/macos/ctx.app");
const WORKSPACE_PATH = process.env.CTX_AUTOMATION_WORKSPACE_PATH ||
  "/Users/example-user/code/ctx-monorepo";

const TAURI_DRIVER_PORT = Number(process.env.TAURI_DRIVER_PORT || 4444);
const TEST_BACKEND_PORT = Number(process.env.TAURI_TEST_BACKEND_PORT || 3000);

const buildAppIfMissing = () => {
  if (fs.existsSync(APP_PATH)) return;
  const result = spawnSync(
    "pnpm",
    ["tauri", "build", "--debug", "--no-bundle", "--", "--features", "automation"],
    { stdio: "inherit", cwd: path.resolve(ROOT, "src-tauri"), shell: true },
  );
  if (result.status !== 0) {
    throw new Error("Failed to build the Tauri app for automation.");
  }
};

let backendProcess = null;
let driverProcess = null;

exports.config = {
  runner: "local",
  framework: "mocha",
  reporters: ["spec"],
  specs: [path.resolve(__dirname, "specs/**/*.spec.cjs")],
  mochaOpts: {
    timeout: 120000,
  },
  logLevel: "info",
  maxInstances: 1,
  capabilities: [
    {
      browserName: "tauri",
      "tauri:options": {
        application: APP_PATH,
      },
    },
  ],
  port: TAURI_DRIVER_PORT,
  path: "/",
  automationProtocol: "webdriver",
  beforeSession: () => {
    process.env.CTX_AUTOMATION_WORKSPACE_PATH = WORKSPACE_PATH;
  },
  onPrepare: async () => {
    if (process.platform === "darwin" && !process.env.CN_API_KEY) {
      throw new Error("CN_API_KEY is required for CrabNebula WebDriver on macOS.");
    }
    buildAppIfMissing();

    backendProcess = spawn("pnpm", ["exec", "test-runner-backend"], {
      stdio: "inherit",
      cwd: ROOT,
      env: {
        ...process.env,
        TEST_RUNNER_BACKEND_PORT: String(TEST_BACKEND_PORT),
      },
    });
    await waitTestRunnerBackendReady();

    driverProcess = spawn("pnpm", ["exec", "tauri-driver"], {
      stdio: "inherit",
      cwd: ROOT,
      env: {
        ...process.env,
        TAURI_DRIVER_PORT: String(TAURI_DRIVER_PORT),
        REMOTE_WEBDRIVER_URL: `http://127.0.0.1:${TEST_BACKEND_PORT}`,
      },
    });
    await waitTauriDriverReady();
  },
  onComplete: () => {
    if (driverProcess) {
      driverProcess.kill();
      driverProcess = null;
    }
    if (backendProcess) {
      backendProcess.kill();
      backendProcess = null;
    }
  },
};
