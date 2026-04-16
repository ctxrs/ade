#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const METRICS = [
  { key: "session_switch_ms", label: "Session switch p95", target: 100, kind: "latency" },
  { key: "replay_final_to_state_ms", label: "Replay final marker->state p95", target: 150, kind: "latency" },
  { key: "replay_final_to_dom_ms", label: "Replay final marker->DOM p95", target: 200, kind: "latency" },
  { key: "final_ingress_to_dom_ms", label: "Final ingress->DOM p95", target: 150, kind: "latency" },
  { key: "final_ws_to_dom_ms", label: "Final WS->DOM p95", target: 50, kind: "latency" },
  { key: "interrupt_click_to_pending_ms", label: "Interrupt click->pending p95", target: 50, kind: "latency" },
  { key: "switch_to_first_paint_ms", label: "Switch->first paint p95", target: 100, kind: "latency" },
  { key: "switch_to_authoritative_ms", label: "Switch->authoritative p95", target: 200, kind: "latency" },
  { path: ["long_tasks_ms", "over_budget"], label: "Long tasks over budget", target: 0, kind: "count" },
  { key: "foreground_queue_age_ms", label: "Foreground queue age p95", target: 75, kind: "latency" },
  { key: "workspace_backlog_age_ms", label: "Workspace backlog age p95", target: 250, kind: "latency" },
  { key: "foreground_gap_recovery_ms", label: "Foreground gap recovery p95", target: 1000, kind: "latency" },
  { key: "stale_pending_after_terminal_assistant_message", label: "Stale pending after terminal", target: 0, kind: "count" },
  { key: "stale_pending_duplicate_assistant_message", label: "Duplicate stale pending", target: 0, kind: "count" },
  { key: "late_chunk_after_terminal_count", label: "Late chunk after terminal", target: 0, kind: "count" },
  { key: "projection_or_seq_regression_count", label: "Projection/seq regression", target: 0, kind: "count" },
  { key: "gap_repair_mismatch_count", label: "Gap repair mismatch", target: 0, kind: "count" },
  { key: "switch_stale_visible_count", label: "Switch stale visible", target: 0, kind: "count" },
  { key: "nav_thread_activity_mismatch_count", label: "Nav/thread activity mismatch", target: 0, kind: "count" },
  { key: "workspace_stream_reset_count", label: "Workspace stream reset", target: 0, kind: "count" },
  { key: "foreground_rehydrate_count", label: "Foreground rehydrate", target: 0, kind: "count" },
];

function usage() {
  console.error(
    "Usage: node ./scripts/replay-loadtest-compare.mjs " +
      "--baseline <file-or-dir> --candidate <file-or-dir> " +
      "[--out-json path] [--out-md path]",
  );
  process.exit(1);
}

function parseArgs(argv) {
  const args = {
    baseline: "",
    candidate: "",
    outJson: "",
    outMd: "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--baseline") {
      args.baseline = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--candidate") {
      args.candidate = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--out-json") {
      args.outJson = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    if (arg === "--out-md") {
      args.outMd = String(argv[i + 1] ?? "");
      i += 1;
      continue;
    }
    usage();
  }
  if (!args.baseline || !args.candidate) {
    usage();
  }
  return args;
}

function isObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function normalizeProfileName(filePath) {
  const base = path.basename(filePath);
  return base.replace(/\.summary\.json$/i, "").replace(/\.json$/i, "");
}

function readProfileMap(targetPath) {
  const abs = path.resolve(targetPath);
  const stat = fs.statSync(abs);
  if (stat.isFile()) {
    return new Map([[normalizeProfileName(abs), readJson(abs)]]);
  }
  const entries = fs
    .readdirSync(abs, { withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith(".json"))
    .map((entry) => path.join(abs, entry.name))
    .sort();
  const out = new Map();
  for (const filePath of entries) {
    out.set(normalizeProfileName(filePath), readJson(filePath));
  }
  return out;
}

function metricKey(metric) {
  return metric.key ?? metric.path.join(".");
}

function readMetricValue(summary, metric) {
  const value =
    Array.isArray(metric.path) && metric.path.length > 0
      ? metric.path.reduce((current, key) => (current == null ? undefined : current[key]), summary)
      : summary?.[metric.key];
  if (Number.isFinite(value)) {
    return value;
  }
  return Number.isFinite(value?.p95) ? value.p95 : null;
}

function pct(value) {
  return Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "n/a";
}

function compareMetric(metric, baselineSummary, candidateSummary) {
  const baseline = readMetricValue(baselineSummary, metric);
  const candidate = readMetricValue(candidateSummary, metric);
  if (!Number.isFinite(baseline) || !Number.isFinite(candidate)) {
    return {
      key: metricKey(metric),
      label: metric.label,
      target: metric.target,
      baseline,
      candidate,
      absolutePass: false,
      relativePass: false,
      improvementRatio: null,
      regressionRatio: null,
      pass: false,
      reason: "missing_p95",
    };
  }

  const baselineMeetsTarget = baseline <= metric.target;
  const candidateMeetsTarget = candidate <= metric.target;
  const improvementRatio = baseline > 0 ? (baseline - candidate) / baseline : baseline === candidate ? 0 : null;
  const regressionRatio = baseline > 0 ? (candidate - baseline) / baseline : candidate === baseline ? 0 : null;

  let relativePass;
  let reason;
  if (metric.kind === "count") {
    relativePass = candidate <= baseline;
    reason = relativePass ? "count_not_increased" : "count_increased";
  } else if (!baselineMeetsTarget) {
    relativePass = candidateMeetsTarget || improvementRatio >= 0.5;
    reason = relativePass ? "improved_or_meets_target" : "insufficient_improvement";
  } else {
    relativePass = candidate <= baseline * 1.1;
    reason = relativePass ? "within_regression_budget" : "regressed_over_budget";
  }

  return {
    key: metricKey(metric),
    label: metric.label,
    target: metric.target,
    baseline,
    candidate,
    absolutePass: candidateMeetsTarget,
    relativePass,
    improvementRatio,
    regressionRatio,
    pass: candidateMeetsTarget && relativePass,
    reason,
  };
}

function compareProfile(name, baselinePayload, candidatePayload) {
  const baselineSummary = isObject(baselinePayload?.summary) ? baselinePayload.summary : baselinePayload;
  const candidateSummary = isObject(candidatePayload?.summary) ? candidatePayload.summary : candidatePayload;
  const metrics = METRICS.map((metric) => compareMetric(metric, baselineSummary, candidateSummary));
  return {
    profile: name,
    pass: metrics.every((metric) => metric.pass),
    metrics,
  };
}

function buildMarkdown(report) {
  const lines = ["# Replay Loadtest Comparison", ""];
  for (const profile of report.profiles) {
    lines.push(`## ${profile.profile}`);
    lines.push("");
    lines.push(`status: ${profile.pass ? "PASS" : "FAIL"}`);
    lines.push("");
    lines.push("| Metric | Baseline p95 | Candidate p95 | Target | Delta | Result |");
    lines.push("| --- | ---: | ---: | ---: | ---: | --- |");
    for (const metric of profile.metrics) {
      const delta =
        Number.isFinite(metric.baseline) && Number.isFinite(metric.candidate)
          ? `${(metric.candidate - metric.baseline).toFixed(1)}`
          : "n/a";
      lines.push(
        `| ${metric.label} | ${metric.baseline ?? "n/a"} | ${metric.candidate ?? "n/a"} | ${metric.target} | ${delta} | ${metric.pass ? "PASS" : `FAIL (${metric.reason})`} |`,
      );
    }
    lines.push("");
  }

  lines.push("## Summary");
  lines.push("");
  lines.push(`overall: ${report.pass ? "PASS" : "FAIL"}`);
  lines.push("");
  lines.push("Relative rules:");
  lines.push(`- baseline missing target: candidate must meet the target or improve by at least ${pct(0.5)}`);
  lines.push(`- baseline already within target: candidate may regress by at most ${pct(0.1)}`);
  lines.push("");
  return `${lines.join("\n")}\n`;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const baselinePath = path.resolve(args.baseline);
  const candidatePath = path.resolve(args.candidate);
  const baselineStat = fs.statSync(baselinePath);
  const candidateStat = fs.statSync(candidatePath);
  let profiles;
  if (baselineStat.isFile() && candidateStat.isFile()) {
    profiles = [
      compareProfile(
        normalizeProfileName(candidatePath),
        readJson(baselinePath),
        readJson(candidatePath),
      ),
    ];
  } else {
    const baseline = readProfileMap(baselinePath);
    const candidate = readProfileMap(candidatePath);
    const profileNames = Array.from(new Set([...baseline.keys(), ...candidate.keys()])).sort();
    if (profileNames.length === 0) {
      throw new Error("No loadtest JSON files found.");
    }
    profiles = profileNames.map((name) => {
      const baselinePayload = baseline.get(name);
      const candidatePayload = candidate.get(name);
      if (!baselinePayload || !candidatePayload) {
        return {
          profile: name,
          pass: false,
          metrics: [],
          reason: !baselinePayload ? "missing_baseline" : "missing_candidate",
        };
      }
      return compareProfile(name, baselinePayload, candidatePayload);
    });
  }

  const report = {
    generated_at: new Date().toISOString(),
    baseline: baselinePath,
    candidate: candidatePath,
    pass: profiles.every((profile) => profile.pass),
    profiles,
  };

  const outJson =
    args.outJson ||
    path.join(path.dirname(path.resolve(args.candidate)), "comparison.json");
  const outMd =
    args.outMd ||
    path.join(path.dirname(path.resolve(args.candidate)), "comparison.md");
  fs.mkdirSync(path.dirname(outJson), { recursive: true });
  fs.mkdirSync(path.dirname(outMd), { recursive: true });
  fs.writeFileSync(outJson, `${JSON.stringify(report, null, 2)}\n`);
  fs.writeFileSync(outMd, buildMarkdown(report));
  console.log(`wrote ${outJson}`);
  console.log(`wrote ${outMd}`);

  if (!report.pass) {
    process.exitCode = 1;
  }
}

main();
