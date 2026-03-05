#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const readJson = (filePath, label) => {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    throw new Error(`failed to parse ${label} at ${filePath}: ${error?.message || String(error)}`);
  }
};

const normalizeText = (value) => String(value || "").trim();
const asArray = (value) => (Array.isArray(value) ? value : []);

const parseArgs = (argv) => {
  const opts = {
    reportPath: "",
    outJson: "",
    outMarkdown: "",
    printJson: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--report") {
      opts.reportPath = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--out-json") {
      opts.outJson = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--out-markdown") {
      opts.outMarkdown = normalizeText(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--json") {
      opts.printJson = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }
  if (!opts.reportPath && !opts.help) {
    throw new Error("--report is required");
  }
  return opts;
};

const printHelp = () => {
  console.log([
    "Usage: node core/scripts/linux_arm_provider_report_summary.cjs --report <path> [options]",
    "",
    "Options:",
    "  --report <path>         Runtime install smoke report JSON path",
    "  --out-json <path>       Write summary JSON",
    "  --out-markdown <path>   Write summary markdown table",
    "  --json                  Print summary JSON to stdout",
    "  --help                  Show help",
  ].join("\n"));
};

const summarizeCounts = (rows, keySelector) => {
  const counts = {};
  for (const row of rows) {
    const key = normalizeText(keySelector(row));
    if (!key) continue;
    counts[key] = (counts[key] || 0) + 1;
  }
  return counts;
};

const computeSummary = (report) => {
  const rows = asArray(report.results).map((entry) => (entry && typeof entry === "object" ? entry : {}));
  const passes = rows.filter((row) => normalizeText(row.result) === "pass");
  const failures = rows.filter((row) => normalizeText(row.result) === "fail");

  const summary = {
    generated_at: new Date().toISOString(),
    source_generated_at: normalizeText(report.generated_at),
    lane: normalizeText(report.lane),
    provider_total: rows.length,
    provider_passed: passes.length,
    provider_failed: failures.length,
    pass_rate: rows.length > 0 ? Number((passes.length / rows.length).toFixed(4)) : 0,
    failure_categories: summarizeCounts(failures, (row) => row.category || "unknown"),
    failure_stages: summarizeCounts(failures, (row) => row.stage || "unknown"),
    failure_error_codes: summarizeCounts(failures, (row) => row.error_code || "none"),
    reason_distribution: summarizeCounts(failures, (row) => row.reason || "unknown"),
    failing_providers: failures.map((row) => ({
      provider_id: normalizeText(row.provider_id),
      stage: normalizeText(row.stage),
      error_code: normalizeText(row.error_code),
      category: normalizeText(row.category),
      reason: normalizeText(row.reason),
    })),
  };

  return summary;
};

const markdownFromSummary = (summary) => {
  const lines = [
    "# Linux ARM Provider Reliability Summary",
    "",
    `- Source generated at: ${summary.source_generated_at || "unknown"}`,
    `- Providers: ${summary.provider_passed}/${summary.provider_total} passed`,
    `- Failures: ${summary.provider_failed}`,
    `- Pass rate: ${summary.pass_rate}`,
    "",
    "## Failure Categories",
    "",
    "| Category | Count |",
    "| --- | ---: |",
  ];
  const categoryEntries = Object.entries(summary.failure_categories);
  if (categoryEntries.length === 0) {
    lines.push("| none | 0 |");
  } else {
    for (const [category, count] of categoryEntries) {
      lines.push(`| ${category} | ${count} |`);
    }
  }
  lines.push("", "## Failing Providers", "", "| Provider | Stage | Error Code | Category | Reason |", "| --- | --- | --- | --- | --- |");
  if (summary.failing_providers.length === 0) {
    lines.push("| none | - | - | - | - |");
  } else {
    for (const row of summary.failing_providers) {
      lines.push(`| ${row.provider_id || "unknown"} | ${row.stage || "unknown"} | ${row.error_code || "-"} | ${row.category || "unknown"} | ${row.reason || "-"} |`);
    }
  }
  lines.push("");
  return `${lines.join("\n")}\n`;
};

const writeIfRequested = (filePath, contents) => {
  if (!filePath) return;
  const resolved = path.isAbsolute(filePath) ? filePath : path.resolve(process.cwd(), filePath);
  fs.mkdirSync(path.dirname(resolved), { recursive: true });
  fs.writeFileSync(resolved, contents, "utf8");
  console.log(`wrote summary artifact: ${resolved}`);
};

if (require.main === module) {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    printHelp();
    process.exit(0);
  }

  const report = readJson(opts.reportPath, "runtime install smoke report");
  const summary = computeSummary(report);

  writeIfRequested(opts.outJson, `${JSON.stringify(summary, null, 2)}\n`);
  writeIfRequested(opts.outMarkdown, markdownFromSummary(summary));

  if (opts.printJson) {
    process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
  } else {
    console.log(
      `linux-arm report summary: providers=${summary.provider_total} passed=${summary.provider_passed} failed=${summary.provider_failed} pass_rate=${summary.pass_rate}`,
    );
  }
}

module.exports = {
  computeSummary,
  markdownFromSummary,
};
