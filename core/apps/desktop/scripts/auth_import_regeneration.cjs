#!/usr/bin/env node

const {
  parseArgs,
  usage,
  createDaemonClient,
  runRegeneration,
  writeReport,
} = require("./auth_import_regeneration_lib.cjs");

const main = async () => {
  const options = parseArgs(process.argv.slice(2), process.env, process.cwd());
  if (options.help) {
    process.stdout.write(usage());
    return;
  }

  const client = createDaemonClient({
    baseUrl: options.baseUrl,
    token: options.token,
  });
  const report = await runRegeneration(client, options);
  writeReport(options.reportPath, report);

  process.stdout.write(`report: ${options.reportPath}\n`);
  process.stdout.write(`result: ${report.result}\n`);
  process.stdout.write(`selected_candidates: ${report.selected_candidates.length}\n`);
  process.stdout.write(`providers_checked: ${report.providers.length}\n`);
  if (report.warnings.length > 0) {
    process.stdout.write(`warnings: ${report.warnings.join(" | ")}\n`);
  }

  if (report.result !== "pass") {
    process.exitCode = 1;
  }
};

main().catch((error) => {
  process.stderr.write(`${String(error && error.stack ? error.stack : error)}\n`);
  process.exitCode = 1;
});
