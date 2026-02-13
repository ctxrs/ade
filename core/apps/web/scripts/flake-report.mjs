#!/usr/bin/env node

import fs from "node:fs";

const args = process.argv.slice(2);
const files = [];
let format = "table";

for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === "--jsonl") {
    const file = args[i + 1];
    if (!file) {
      console.error("error: --jsonl requires a path");
      process.exit(2);
    }
    files.push(file);
    i += 1;
    continue;
  }
  if (arg === "--format") {
    const next = args[i + 1];
    if (!next || !["table", "json"].includes(next)) {
      console.error("error: --format must be table|json");
      process.exit(2);
    }
    format = next;
    i += 1;
    continue;
  }
  if (arg === "-h" || arg === "--help") {
    console.log("usage: flake-report.mjs [--jsonl <path>]... [--format table|json]");
    process.exit(0);
  }
  console.error(`error: unknown option: ${arg}`);
  process.exit(2);
}

if (files.length === 0) {
  files.push("ci-flake-stats.jsonl");
}

const laneForJob = (job) => {
  const j = String(job || "").toLowerCase();
  if (j.includes("premerge")) return "premerge_required";
  if (j.includes("cross-platform")) return "cross_platform";
  if (j.includes("soak")) return "soak";
  if (j.includes("load")) return "load";
  if (j.includes("t3") || j.includes("fault") || j.includes("fuzz") || j.includes("replay"))
    return "t3";
  return "other";
};

const laneStats = new Map();
const ensure = (lane) => {
  if (!laneStats.has(lane)) {
    laneStats.set(lane, { total: 0, failed: 0, flake: 0, flake_failed: 0 });
  }
  return laneStats.get(lane);
};

for (const file of files) {
  if (!fs.existsSync(file)) {
    console.error(`error: missing input file: ${file}`);
    process.exit(1);
  }
  const raw = fs.readFileSync(file, "utf8").split(/\r?\n/);
  for (const line of raw) {
    if (!line.trim()) continue;
    const event = JSON.parse(line);
    const lane = laneForJob(event.job);
    const s = ensure(lane);
    s.total += 1;
    if (event.status === "failed") s.failed += 1;
    if (event.status === "flake") s.flake += 1;
    if (event.status === "flake_failed") s.flake_failed += 1;
  }
}

const rows = [...laneStats.entries()]
  .map(([lane, s]) => {
    const flakeCount = s.flake + s.flake_failed;
    return {
      lane,
      total: s.total,
      failed: s.failed,
      flake: s.flake,
      flake_failed: s.flake_failed,
      flake_rate: s.total > 0 ? Number((flakeCount / s.total).toFixed(4)) : 0,
      failure_rate: s.total > 0 ? Number((s.failed / s.total).toFixed(4)) : 0,
    };
  })
  .sort((a, b) => a.lane.localeCompare(b.lane));

if (format === "json") {
  console.log(JSON.stringify({ files, lanes: rows }, null, 2));
  process.exit(0);
}

console.log(`files=${files.length}`);
console.log("lane\ttotal\tfailed\tflake\tflake_failed\tflake_rate\tfailure_rate");
for (const r of rows) {
  console.log(
    `${r.lane}\t${r.total}\t${r.failed}\t${r.flake}\t${r.flake_failed}\t${r.flake_rate}\t${r.failure_rate}`,
  );
}
