const test = require("node:test");
const assert = require("node:assert/strict");
const net = require("node:net");
const path = require("node:path");
const { spawn } = require("node:child_process");

const CLI_PATH = require.resolve("@crabnebula/test-runner-backend/cli.js", {
  paths: [path.resolve(__dirname, "..")],
});

const listen = (port) =>
  new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });

const canConnect = (port) =>
  new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    socket.once("connect", () => {
      socket.destroy();
      resolve(true);
    });
    socket.once("error", () => {
      resolve(false);
    });
  });

const waitForExit = (child) =>
  new Promise((resolve) => {
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });

test("darwin CrabNebula backend still binds fixed port 3000", { skip: process.platform !== "darwin" }, async () => {
  let blocker = null;
  if (!(await canConnect(3000))) {
    blocker = await listen(3000);
  }
  const requestedPort = 62329;
  const child = spawn(
    process.execPath,
    [CLI_PATH, "--host", "127.0.0.1", "--port", String(requestedPort)],
    {
      stdio: ["ignore", "pipe", "pipe"],
      env: {
        ...process.env,
        TAURI_TEST_BACKEND_PORT: String(requestedPort),
        TEST_RUNNER_BACKEND_PORT: String(requestedPort),
        CTX_AUTOMATION_CN_BACKEND_PORT: String(requestedPort),
      },
    },
  );

  let output = "";
  child.stdout.on("data", (chunk) => {
    output += String(chunk);
  });
  child.stderr.on("data", (chunk) => {
    output += String(chunk);
  });

  const result = await waitForExit(child);
  blocker?.close();

  assert.notEqual(result.code, 0, output);
  assert.match(output, /failed to bind to port 3000/i);
  assert.doesNotMatch(output, new RegExp(`failed to bind to port ${requestedPort}`, "i"));
});
