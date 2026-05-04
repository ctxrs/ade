#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(__dirname, "..");
const defaultOutDir = path.join(os.tmpdir(), "ctx-foreground-large-head-sla-proof");
const generatorScript = path.join(__dirname, "generate-foreground-large-head-fixture.mjs");
const replayScript = path.join(__dirname, "replay-loadtest.mjs");

function usage() {
  console.error(
    "Usage: node ./scripts/run-foreground-large-head-sla-proof.mjs --base-url url [--out-dir path] " +
      "[--session-head-base-delay-ms ms] [--session-head-bandwidth-kibps kibps] " +
      "[--gap-count n] [--first-gap-delay-ms ms] [--gap-interval-ms ms] [--max-gap-recovery-p95-ms ms] [--max-final-ws-to-dom-p95-ms ms] " +
      "[--max-final-ingress-to-dom-p95-ms ms] [--max-recovered-head-to-state-p95-ms ms] " +
      "[--max-recovered-head-to-dom-p95-ms ms] [--max-switch-overlap-long-task-ms ms] [--max-head-response-bytes bytes]",
  );
  process.exit(1);
}

function parseArgs(argv) {
  const parsed = {
    baseUrl: process.env.CTX_LOADTEST_BASE_URL || "",
    outDir: defaultOutDir,
    sessionHeadBaseDelayMs: 80,
    sessionHeadBandwidthKibps: 384,
    gapCount: 10,
    firstGapDelayMs: 4000,
    gapIntervalMs: 1000,
    maxGapRecoveryP95Ms: 1000,
    maxFinalWsToDomP95Ms: 50,
    maxFinalIngressToDomP95Ms: 150,
    maxRecoveredHeadToStateP95Ms: 250,
    maxRecoveredHeadToDomP95Ms: 250,
    maxSwitchOverlapLongTaskMs: 250,
    maxHeadResponseBytes: 256000,
    messageCount: 164,
    messageBytes: 260,
    toolSummaries: 335,
    boundedToolSummaries: 96,
    toolPreviewBytes: 500,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--base-url") {
      parsed.baseUrl = String(argv[i + 1] ?? parsed.baseUrl);
      i += 1;
      continue;
    }
    if (arg === "--out-dir") {
      parsed.outDir = String(argv[i + 1] ?? parsed.outDir);
      i += 1;
      continue;
    }
    if (arg === "--session-head-base-delay-ms") {
      parsed.sessionHeadBaseDelayMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--session-head-bandwidth-kibps") {
      parsed.sessionHeadBandwidthKibps = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--gap-count") {
      parsed.gapCount = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--first-gap-delay-ms") {
      parsed.firstGapDelayMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--gap-interval-ms") {
      parsed.gapIntervalMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-gap-recovery-p95-ms") {
      parsed.maxGapRecoveryP95Ms = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-final-ws-to-dom-p95-ms") {
      parsed.maxFinalWsToDomP95Ms = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-final-ingress-to-dom-p95-ms") {
      parsed.maxFinalIngressToDomP95Ms = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-recovered-head-to-state-p95-ms") {
      parsed.maxRecoveredHeadToStateP95Ms = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-recovered-head-to-dom-p95-ms") {
      parsed.maxRecoveredHeadToDomP95Ms = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-switch-overlap-long-task-ms") {
      parsed.maxSwitchOverlapLongTaskMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--max-head-response-bytes") {
      parsed.maxHeadResponseBytes = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--message-count") {
      parsed.messageCount = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--message-bytes") {
      parsed.messageBytes = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--tool-summaries") {
      parsed.toolSummaries = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--bounded-tool-summaries") {
      parsed.boundedToolSummaries = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--tool-preview-bytes") {
      parsed.toolPreviewBytes = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      usage();
    }
    usage();
  }

  if (!parsed.baseUrl) {
    throw new Error("--base-url is required.");
  }
  for (const [name, value] of Object.entries(parsed)) {
    if (name === "baseUrl" || name === "outDir") continue;
    if (!Number.isFinite(value) || value < 0) {
      throw new Error(`--${name} must be a non-negative number.`);
    }
  }
  if (parsed.gapCount < 1) {
    throw new Error("--gap-count must be at least 1.");
  }
  return parsed;
}

function runCommand(command, args, options = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? appRoot,
      env: {
        ...process.env,
        ...(options.env ?? {}),
      },
      stdio: "inherit",
    });
    child.on("error", reject);
    child.on("exit", (code, signal) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`${command} ${args.join(" ")} exited with ${signal ?? code}`));
    });
  });
}

function readMetric(summary, key, field = "p95") {
  const value = summary?.[key]?.[field];
  return Number.isFinite(value) ? value : null;
}

function readCount(summary, key) {
  const value = summary?.[key];
  if (Number.isFinite(value)) return value;
  const count = summary?.[key]?.count;
  return Number.isFinite(count) ? count : null;
}

function fmt(value, suffix = "") {
  return Number.isFinite(value) ? `${Math.round(value * 10) / 10}${suffix}` : "n/a";
}

function fmtRatioAsPercent(value) {
  return Number.isFinite(value) ? fmt(value * 100, "%") : "n/a";
}

function improvementRatio(baseline, candidate) {
  if (!Number.isFinite(baseline) || baseline <= 0 || !Number.isFinite(candidate)) return null;
  return (baseline - candidate) / baseline;
}

function buildReport({ args, legacy, bounded }) {
  const legacySummary = legacy.summary;
  const boundedSummary = bounded.summary;
  const metrics = {
    headBytes: {
      legacy: readMetric(legacySummary, "session_head_response_bytes"),
      bounded: readMetric(boundedSummary, "session_head_response_bytes"),
      target: args.maxHeadResponseBytes,
    },
    headRouteMs: {
      legacy: readMetric(legacySummary, "session_head_route_ms"),
      bounded: readMetric(boundedSummary, "session_head_route_ms"),
      target: args.maxGapRecoveryP95Ms,
    },
    gapRecoveryMs: {
      legacy: readMetric(legacySummary, "foreground_gap_recovery_ms"),
      bounded: readMetric(boundedSummary, "foreground_gap_recovery_ms"),
      target: args.maxGapRecoveryP95Ms,
      count: readCount(boundedSummary, "foreground_gap_recovery_ms"),
    },
    finalWsToDomMs: {
      legacy: readMetric(legacySummary, "final_ws_to_dom_ms"),
      bounded: readMetric(boundedSummary, "final_ws_to_dom_ms"),
      target: args.maxFinalWsToDomP95Ms,
      count: readCount(boundedSummary, "final_ws_to_dom_ms"),
    },
    finalIngressToDomMs: {
      legacy: readMetric(legacySummary, "final_ingress_to_dom_ms"),
      bounded: readMetric(boundedSummary, "final_ingress_to_dom_ms"),
      target: args.maxFinalIngressToDomP95Ms,
      count: readCount(boundedSummary, "final_ingress_to_dom_ms"),
    },
    recoveredHeadToStateMs: {
      legacy: readMetric(legacySummary, "replay_final_to_state_ms"),
      bounded: readMetric(boundedSummary, "replay_final_to_state_ms"),
      target: args.maxRecoveredHeadToStateP95Ms,
      count: readCount(boundedSummary, "replay_final_to_state_ms"),
    },
    recoveredHeadToDomMs: {
      legacy: readMetric(legacySummary, "replay_final_to_dom_ms"),
      bounded: readMetric(boundedSummary, "replay_final_to_dom_ms"),
      target: args.maxRecoveredHeadToDomP95Ms,
      count: readCount(boundedSummary, "replay_final_to_dom_ms"),
    },
    switchOverlapLongTaskMs: {
      legacy: readMetric(legacySummary, "switch_overlap_long_task_ms", "max"),
      bounded: readMetric(boundedSummary, "switch_overlap_long_task_ms", "max"),
      target: args.maxSwitchOverlapLongTaskMs,
    },
    gapTimeoutCount: {
      legacy: readCount(legacySummary, "foreground_gap_recovery_timeout_count"),
      bounded: readCount(boundedSummary, "foreground_gap_recovery_timeout_count"),
      target: 0,
    },
    foregroundRehydrateCount: {
      legacy: readCount(legacySummary, "foreground_rehydrate_count"),
      bounded: readCount(boundedSummary, "foreground_rehydrate_count"),
      target: args.gapCount,
    },
    gapRepairMismatchCount: {
      legacy: readCount(legacySummary, "gap_repair_mismatch_count"),
      bounded: readCount(boundedSummary, "gap_repair_mismatch_count"),
      target: 0,
    },
    navThreadMismatchCount: {
      legacy: readCount(legacySummary, "nav_thread_activity_mismatch_count"),
      bounded: readCount(boundedSummary, "nav_thread_activity_mismatch_count"),
      target: 0,
    },
    switchStaleVisibleCount: {
      legacy: readCount(legacySummary, "switch_stale_visible_count"),
      bounded: readCount(boundedSummary, "switch_stale_visible_count"),
      target: 0,
    },
    longTasksOverBudget: {
      legacy: legacySummary.long_tasks_ms?.over_budget ?? null,
      bounded: boundedSummary.long_tasks_ms?.over_budget ?? null,
      target: null,
    },
  };

  const failures = [];
  const minRecoveredIntervals = Math.max(1, args.gapCount - 1);
  if ((metrics.foregroundRehydrateCount.bounded ?? 0) < args.gapCount) {
    failures.push(
      `candidate captured ${metrics.foregroundRehydrateCount.bounded ?? 0} foreground rehydrates, expected ${args.gapCount}`,
    );
  }
  if ((metrics.gapRecoveryMs.count ?? 0) < minRecoveredIntervals) {
    failures.push(
      `candidate captured ${metrics.gapRecoveryMs.count ?? 0} completed gap recovery intervals, expected at least ${minRecoveredIntervals}`,
    );
  }
  if ((metrics.recoveredHeadToStateMs.count ?? 0) < 1) {
    failures.push("candidate did not capture recovered head->state visibility");
  }
  if ((metrics.recoveredHeadToDomMs.count ?? 0) < 1) {
    failures.push("candidate did not capture recovered head->DOM visibility");
  }
  if (!Number.isFinite(metrics.headBytes.bounded) || metrics.headBytes.bounded > metrics.headBytes.target) {
    failures.push(`candidate head bytes ${fmt(metrics.headBytes.bounded)} > ${metrics.headBytes.target}`);
  }
  if (!Number.isFinite(metrics.gapRecoveryMs.bounded) || metrics.gapRecoveryMs.bounded > metrics.gapRecoveryMs.target) {
    failures.push(`candidate foreground gap recovery p95 ${fmt(metrics.gapRecoveryMs.bounded, "ms")} > ${metrics.gapRecoveryMs.target}ms`);
  }
  if (
    (metrics.finalWsToDomMs.count ?? 0) > 0 &&
    (!Number.isFinite(metrics.finalWsToDomMs.bounded) || metrics.finalWsToDomMs.bounded > metrics.finalWsToDomMs.target)
  ) {
    failures.push(`candidate final ws->DOM p95 ${fmt(metrics.finalWsToDomMs.bounded, "ms")} > ${metrics.finalWsToDomMs.target}ms`);
  }
  if (
    (metrics.finalIngressToDomMs.count ?? 0) > 0 &&
    (!Number.isFinite(metrics.finalIngressToDomMs.bounded) ||
      metrics.finalIngressToDomMs.bounded > metrics.finalIngressToDomMs.target)
  ) {
    failures.push(
      `candidate final ingress->DOM p95 ${fmt(metrics.finalIngressToDomMs.bounded, "ms")} > ${metrics.finalIngressToDomMs.target}ms`,
    );
  }
  if (
    !Number.isFinite(metrics.recoveredHeadToStateMs.bounded) ||
    metrics.recoveredHeadToStateMs.bounded > metrics.recoveredHeadToStateMs.target
  ) {
    failures.push(
      `candidate recovered head->state p95 ${fmt(metrics.recoveredHeadToStateMs.bounded, "ms")} > ${metrics.recoveredHeadToStateMs.target}ms`,
    );
  }
  if (
    !Number.isFinite(metrics.recoveredHeadToDomMs.bounded) ||
    metrics.recoveredHeadToDomMs.bounded > metrics.recoveredHeadToDomMs.target
  ) {
    failures.push(
      `candidate recovered head->DOM p95 ${fmt(metrics.recoveredHeadToDomMs.bounded, "ms")} > ${metrics.recoveredHeadToDomMs.target}ms`,
    );
  }
  if (
    Number.isFinite(metrics.switchOverlapLongTaskMs.bounded) &&
    metrics.switchOverlapLongTaskMs.bounded > metrics.switchOverlapLongTaskMs.target
  ) {
    failures.push(
      `candidate switch-overlap long task max ${fmt(metrics.switchOverlapLongTaskMs.bounded, "ms")} > ${metrics.switchOverlapLongTaskMs.target}ms`,
    );
  }
  for (const key of [
    "gapTimeoutCount",
    "gapRepairMismatchCount",
    "navThreadMismatchCount",
    "switchStaleVisibleCount",
  ]) {
    const metric = metrics[key];
    if (!Number.isFinite(metric.bounded) || metric.bounded > metric.target) {
      failures.push(`candidate ${key} ${fmt(metric.bounded)} > ${metric.target}`);
    }
  }
  if (
    Number.isFinite(metrics.headBytes.legacy) &&
    Number.isFinite(metrics.headBytes.bounded) &&
    metrics.headBytes.bounded >= metrics.headBytes.legacy
  ) {
    failures.push("candidate head response is not smaller than legacy control");
  }
  if (
    Number.isFinite(metrics.gapRecoveryMs.legacy) &&
    Number.isFinite(metrics.gapRecoveryMs.bounded) &&
    metrics.gapRecoveryMs.bounded > metrics.gapRecoveryMs.legacy
  ) {
    failures.push("candidate gap recovery p95 regressed against legacy control");
  }

  const comparisons = {
    headBytesReduction: improvementRatio(metrics.headBytes.legacy, metrics.headBytes.bounded),
    headRouteMsReduction: improvementRatio(metrics.headRouteMs.legacy, metrics.headRouteMs.bounded),
    gapRecoveryMsReduction: improvementRatio(metrics.gapRecoveryMs.legacy, metrics.gapRecoveryMs.bounded),
    recoveredHeadToDomMsReduction: improvementRatio(
      metrics.recoveredHeadToDomMs.legacy,
      metrics.recoveredHeadToDomMs.bounded,
    ),
  };

  return {
    pass: failures.length === 0,
    failures,
    config: {
      baseUrl: args.baseUrl,
      sessionHeadBaseDelayMs: args.sessionHeadBaseDelayMs,
      sessionHeadBandwidthKibps: args.sessionHeadBandwidthKibps,
      gapCount: args.gapCount,
      firstGapDelayMs: args.firstGapDelayMs,
      gapIntervalMs: args.gapIntervalMs,
      messageCount: args.messageCount,
      toolSummaries: args.toolSummaries,
      boundedToolSummaries: args.boundedToolSummaries,
      toolPreviewBytes: args.toolPreviewBytes,
      maxRecoveredHeadToStateP95Ms: args.maxRecoveredHeadToStateP95Ms,
      maxRecoveredHeadToDomP95Ms: args.maxRecoveredHeadToDomP95Ms,
      maxSwitchOverlapLongTaskMs: args.maxSwitchOverlapLongTaskMs,
    },
    metrics,
    comparisons,
  };
}

function buildMarkdown(report, paths) {
  const lines = [
    "# Foreground Large-Head SLA Proof",
    "",
    `status: ${report.pass ? "PASS" : "FAIL"}`,
    "",
    "## Config",
    "",
    `- base URL: ${report.config.baseUrl}`,
    `- gap count: ${report.config.gapCount}`,
    `- first gap delay: ${report.config.firstGapDelayMs}ms`,
    `- gap interval: ${report.config.gapIntervalMs}ms`,
    `- head shaping: ${report.config.sessionHeadBaseDelayMs}ms base + ${report.config.sessionHeadBandwidthKibps} KiB/s`,
    `- fixture shape: ${report.config.messageCount} messages, ${report.config.toolSummaries} legacy tool summaries, ${report.config.boundedToolSummaries} bounded tool summaries`,
    "",
    "## Metrics",
    "",
    "| Metric | Legacy p95 | Bounded p95 | Target |",
    "| --- | ---: | ---: | ---: |",
    `| head response bytes | ${fmt(report.metrics.headBytes.legacy)} | ${fmt(report.metrics.headBytes.bounded)} | ${report.metrics.headBytes.target} |`,
    `| head route ms | ${fmt(report.metrics.headRouteMs.legacy, "ms")} | ${fmt(report.metrics.headRouteMs.bounded, "ms")} | ${report.metrics.headRouteMs.target}ms |`,
    `| foreground gap recovery ms | ${fmt(report.metrics.gapRecoveryMs.legacy, "ms")} | ${fmt(report.metrics.gapRecoveryMs.bounded, "ms")} | ${report.metrics.gapRecoveryMs.target}ms |`,
    `| recovered head to state ms | ${fmt(report.metrics.recoveredHeadToStateMs.legacy, "ms")} | ${fmt(report.metrics.recoveredHeadToStateMs.bounded, "ms")} | ${report.metrics.recoveredHeadToStateMs.target}ms |`,
    `| recovered head to DOM ms | ${fmt(report.metrics.recoveredHeadToDomMs.legacy, "ms")} | ${fmt(report.metrics.recoveredHeadToDomMs.bounded, "ms")} | ${report.metrics.recoveredHeadToDomMs.target}ms |`,
    `| switch-overlap long task max | ${fmt(report.metrics.switchOverlapLongTaskMs.legacy, "ms")} | ${fmt(report.metrics.switchOverlapLongTaskMs.bounded, "ms")} | ${report.metrics.switchOverlapLongTaskMs.target}ms |`,
    "",
    "## Stream Diagnostics",
    "",
    "| Metric | Legacy p95 | Bounded p95 | Notes |",
    "| --- | ---: | ---: | --- |",
    `| final WS to DOM ms | ${fmt(report.metrics.finalWsToDomMs.legacy, "ms")} | ${fmt(report.metrics.finalWsToDomMs.bounded, "ms")} | only present when fixture streams a terminal delta |`,
    `| final ingress to DOM ms | ${fmt(report.metrics.finalIngressToDomMs.legacy, "ms")} | ${fmt(report.metrics.finalIngressToDomMs.bounded, "ms")} | only present when fixture streams a terminal delta |`,
    "",
    "## Counters",
    "",
    "| Counter | Legacy | Bounded | Target |",
    "| --- | ---: | ---: | ---: |",
    `| gap recovery timeouts | ${fmt(report.metrics.gapTimeoutCount.legacy)} | ${fmt(report.metrics.gapTimeoutCount.bounded)} | 0 |`,
    `| foreground rehydrates | ${fmt(report.metrics.foregroundRehydrateCount.legacy)} | ${fmt(report.metrics.foregroundRehydrateCount.bounded)} | ${report.metrics.foregroundRehydrateCount.target} |`,
    `| gap repair mismatches | ${fmt(report.metrics.gapRepairMismatchCount.legacy)} | ${fmt(report.metrics.gapRepairMismatchCount.bounded)} | 0 |`,
    `| nav/thread mismatches | ${fmt(report.metrics.navThreadMismatchCount.legacy)} | ${fmt(report.metrics.navThreadMismatchCount.bounded)} | 0 |`,
    `| stale visible switches | ${fmt(report.metrics.switchStaleVisibleCount.legacy)} | ${fmt(report.metrics.switchStaleVisibleCount.bounded)} | 0 |`,
    `| long tasks over budget | ${fmt(report.metrics.longTasksOverBudget.legacy)} | ${fmt(report.metrics.longTasksOverBudget.bounded)} | diagnostic |`,
    "",
    "## Improvement",
    "",
    `- head bytes reduction: ${fmtRatioAsPercent(report.comparisons.headBytesReduction)}`,
    `- head route reduction: ${fmtRatioAsPercent(report.comparisons.headRouteMsReduction)}`,
    `- gap recovery reduction: ${fmtRatioAsPercent(report.comparisons.gapRecoveryMsReduction)}`,
    `- recovered head DOM reduction: ${fmtRatioAsPercent(report.comparisons.recoveredHeadToDomMsReduction)}`,
    "",
    "## Artifacts",
    "",
    `- legacy summary: ${paths.legacySummary}`,
    `- bounded summary: ${paths.boundedSummary}`,
    `- report JSON: ${paths.reportJson}`,
  ];
  if (report.failures.length > 0) {
    lines.push("", "## Failures", "");
    for (const failure of report.failures) {
      lines.push(`- ${failure}`);
    }
  }
  return `${lines.join("\n")}\n`;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const outDir = path.resolve(args.outDir);
  const paths = {
    legacyFixture: path.join(outDir, "legacy.fixture.json"),
    boundedFixture: path.join(outDir, "bounded.fixture.json"),
    legacySummary: path.join(outDir, "legacy.summary.json"),
    boundedSummary: path.join(outDir, "bounded.summary.json"),
    reportJson: path.join(outDir, "foreground-large-head-sla.report.json"),
    reportMd: path.join(outDir, "foreground-large-head-sla.report.md"),
  };
  await mkdir(outDir, { recursive: true });

  const commonFixtureArgs = [
    "--message-count",
    String(args.messageCount),
    "--message-bytes",
    String(args.messageBytes),
    "--tool-summaries",
    String(args.toolSummaries),
    "--bounded-tool-summaries",
    String(args.boundedToolSummaries),
    "--tool-preview-bytes",
    String(args.toolPreviewBytes),
    "--gap-count",
    String(args.gapCount),
    "--first-gap-delay-ms",
    String(args.firstGapDelayMs),
    "--gap-interval-ms",
    String(args.gapIntervalMs),
  ];

  await runCommand(process.execPath, [
    generatorScript,
    "--variant",
    "legacy",
    "--out",
    paths.legacyFixture,
    ...commonFixtureArgs,
  ]);
  await runCommand(process.execPath, [
    generatorScript,
    "--variant",
    "bounded",
    "--out",
    paths.boundedFixture,
    ...commonFixtureArgs,
  ]);

  const replayEnv = {
    CTX_REPLAY_FINAL_WAIT_MS: "500",
  };
  const replayArgs = [
    "--base-url",
    args.baseUrl,
    "--session-head-base-delay-ms",
    String(args.sessionHeadBaseDelayMs),
    "--session-head-bandwidth-kibps",
    String(args.sessionHeadBandwidthKibps),
    "--wait-timeout-ms",
    "30000",
    "--max-long-task-ms",
    "100",
  ];

  await runCommand(process.execPath, [
    replayScript,
    "--fixture",
    paths.legacyFixture,
    "--out",
    paths.legacySummary,
    ...replayArgs,
  ], { env: replayEnv });
  await runCommand(process.execPath, [
    replayScript,
    "--fixture",
    paths.boundedFixture,
    "--out",
    paths.boundedSummary,
    ...replayArgs,
  ], { env: replayEnv });

  const legacy = JSON.parse(await readFile(paths.legacySummary, "utf8"));
  const bounded = JSON.parse(await readFile(paths.boundedSummary, "utf8"));
  const report = buildReport({ args, legacy, bounded });
  await writeFile(paths.reportJson, `${JSON.stringify(report, null, 2)}\n`);
  await writeFile(paths.reportMd, buildMarkdown(report, paths));
  console.log(`wrote SLA report to ${paths.reportMd}`);

  if (!report.pass) {
    console.error("foreground large-head SLA proof failed:");
    for (const failure of report.failures) {
      console.error(`- ${failure}`);
    }
    process.exit(1);
  }
}

await main();
