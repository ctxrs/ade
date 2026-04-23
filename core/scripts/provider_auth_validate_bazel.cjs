#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");

const checks = [
  ["scripts/validate_provider_auth_matrix.cjs", "--check-report"],
  ["scripts/desktop_e2e_secret_contract.cjs", "--check-report"],
];

for (const [script, ...args] of checks) {
  childProcess.execFileSync(process.execPath, [script, ...args], {
    cwd: coreRoot,
    stdio: "inherit",
  });
}
