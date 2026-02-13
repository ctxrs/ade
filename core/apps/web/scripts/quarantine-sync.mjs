#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const args = process.argv.slice(2);
let reportPath = null;
let promoteThreshold = 0;
let demoteThreshold = 0.03;

for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === "--report") {
    reportPath = args[i + 1];
    i += 1;
    continue;
  }
  if (arg === "--promote-threshold") {
    promoteThreshold = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--demote-threshold") {
    demoteThreshold = Number(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "-h" || arg === "--help") {
    console.log(
      "usage: quarantine-sync.mjs --report <flake-report.json> [--promote-threshold <n>] [--demote-threshold <n>]",
    );
    process.exit(0);
  }
  console.error(`error: unknown option: ${arg}`);
  process.exit(2);
}

if (!reportPath) {
  console.error("error: --report is required");
  process.exit(2);
}
if (!fs.existsSync(reportPath)) {
  console.error(`error: report not found: ${reportPath}`);
  process.exit(1);
}

const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
const lanes = Array.isArray(report.lanes) ? report.lanes : [];

const manifestPath = path.resolve("e2e/suites/quarantine.txt");
const existing = fs.existsSync(manifestPath)
  ? fs
      .readFileSync(manifestPath, "utf8")
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#"))
  : [];

const suggestions = [];
for (const lane of lanes) {
  if (lane.lane === "premerge_required") {
    if (lane.flake_rate > demoteThreshold || lane.flake_failed > 0) {
      suggestions.push(
        `demote-from-premerge: lane=${lane.lane} flake_rate=${lane.flake_rate} flake_failed=${lane.flake_failed}`,
      );
    }
  } else if (lane.flake_rate <= promoteThreshold && lane.total >= 5) {
    suggestions.push(`promotion-candidate: lane=${lane.lane} flake_rate=${lane.flake_rate}`);
  }
}

console.log(`quarantine manifest: ${manifestPath}`);
console.log(`current entries: ${existing.length}`);
if (suggestions.length === 0) {
  console.log("no quarantine suggestions");
  process.exit(0);
}
console.log("suggestions:");
for (const item of suggestions) {
  console.log(`- ${item}`);
}
