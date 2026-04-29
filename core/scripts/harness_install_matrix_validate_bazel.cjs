#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");

const checks = [
  ["scripts/validate_harness_install_matrix.cjs", "--check-report"],
  [
    "--test",
    "scripts/validate_harness_install_matrix.test.cjs",
    "scripts/release_runtime_install_smoke_contract.test.cjs",
    "apps/desktop/scripts/run_harness_install_matrix.test.cjs",
  ],
];

for (const args of checks) {
  childProcess.execFileSync(process.execPath, args, {
    cwd: coreRoot,
    stdio: "inherit",
  });
}
