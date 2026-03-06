#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const {
  defaultReportPath,
  renderContractReport,
} = require("./desktop_e2e_secret_contract_lib.cjs");

const resolveInputPath = (raw, fallbackPath) => {
  const value = String(raw || "").trim();
  if (!value) return fallbackPath;
  if (path.isAbsolute(value)) return value;
  return path.resolve(process.cwd(), value);
};

const parseArgs = (argv) => {
  const opts = {
    reportPath: "",
    checkReportPath: defaultReportPath,
    checkReport: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--report") {
      opts.reportPath = resolveInputPath(argv[index + 1], defaultReportPath);
      index += 1;
      continue;
    }
    if (arg === "--check-report") {
      opts.checkReport = true;
      const next = argv[index + 1];
      if (next && !next.startsWith("-")) {
        opts.checkReportPath = resolveInputPath(next, defaultReportPath);
        index += 1;
      }
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      opts.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }

  return opts;
};

const writeReport = (reportPath, content) => {
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(reportPath, content, "utf8");
};

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    process.stdout.write("usage: desktop_e2e_secret_contract.cjs [--report PATH] [--check-report [PATH]]\n");
    return;
  }

  const content = renderContractReport();
  if (opts.reportPath) {
    writeReport(opts.reportPath, content);
    process.stdout.write(`wrote contract report: ${opts.reportPath}\n`);
  }

  if (opts.checkReport) {
    const existing = fs.existsSync(opts.checkReportPath) ? fs.readFileSync(opts.checkReportPath, "utf8") : "";
    if (existing !== content) {
      process.stderr.write(`contract report out of date: ${opts.checkReportPath}\n`);
      process.exitCode = 1;
      return;
    }
    process.stdout.write(`contract report is up to date: ${opts.checkReportPath}\n`);
    return;
  }

  if (!opts.reportPath) {
    process.stdout.write(content);
  }
};

main();
