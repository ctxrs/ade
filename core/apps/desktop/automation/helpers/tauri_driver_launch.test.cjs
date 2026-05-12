const test = require("node:test");
const assert = require("node:assert/strict");

const { buildNonDarwinTauriDriverLaunch } = require("./tauri_driver_launch.cjs");

test("linux headless tauri-driver launch passes explicit port under xvfb", () => {
  const launch = buildNonDarwinTauriDriverLaunch({
    platform: "linux",
    hasDisplay: false,
    port: 33553,
    nativePort: 33554,
  });
  assert.equal(launch.command, "xvfb-run");
  assert.deepEqual(launch.args, [
    "-a",
    "pnpm",
    "exec",
    "tauri-driver",
    "--port",
    "33553",
    "--native-port",
    "33554",
  ]);
});

test("linux display tauri-driver launch passes explicit port", () => {
  const launch = buildNonDarwinTauriDriverLaunch({
    platform: "linux",
    hasDisplay: true,
    port: 4444,
    nativePort: 4446,
  });
  assert.equal(launch.command, "pnpm");
  assert.deepEqual(launch.args, [
    "exec",
    "tauri-driver",
    "--port",
    "4444",
    "--native-port",
    "4446",
  ]);
});
