#!/usr/bin/env node

const childProcess = require("node:child_process");
const os = require("node:os");
const path = require("node:path");

const { resolveLaunchMode } = require("./desktop_mode.cjs");

const coreRoot = path.resolve(__dirname, "..");
const daemonBind = process.env.CTX_DAEMON_BIND || "127.0.0.1:4399";
const daemonUrl = process.env.CTX_DAEMON_URL || `http://${daemonBind}`;
const dataDir = process.env.CTX_DATA_DIR || path.join(os.homedir(), ".ctx");

const children = [];
let shuttingDown = false;

const stopAll = (signal = "SIGTERM") => {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) {
    if (!child.killed) {
      try {
        child.kill(signal);
      } catch (_) {}
    }
  }
};

const spawn = (command, args, env) => {
  const child = childProcess.spawn(command, args, {
    cwd: coreRoot,
    env,
    stdio: "inherit",
  });
  children.push(child);
  return child;
};

const main = () => {
  const mode = resolveLaunchMode({ surface: "daemon-web" });
  console.log(
    `desktop_mode_start: channel=${mode.channel} profile=${mode.profile} surface=${mode.surface}`,
  );
  const sharedEnv = {
    ...process.env,
    CTX_DESKTOP_CHANNEL: mode.channel,
    CTX_RUNTIME_PROFILE: mode.profile,
    CTX_LAUNCH_SURFACE: mode.surface,
    CTX_DATA_DIR: dataDir,
  };

  const daemon = spawn(
    "cargo",
    ["run", "-p", "ctx-http", "--", "serve", "--bind", daemonBind, "--data-dir", dataDir],
    sharedEnv,
  );

  const web = spawn("pnpm", ["-C", "apps/web", "dev"], {
    ...sharedEnv,
    CTX_DAEMON_URL: daemonUrl,
  });

  daemon.on("exit", (code, signal) => {
    if (!shuttingDown) {
      console.error(`daemon exited unexpectedly (code=${code ?? "null"} signal=${signal ?? "null"})`);
      stopAll();
      process.exit(code ?? 1);
    }
  });

  web.on("exit", (code) => {
    stopAll();
    process.exit(code ?? 0);
  });

  process.on("SIGINT", () => stopAll("SIGINT"));
  process.on("SIGTERM", () => stopAll("SIGTERM"));
};

main();
