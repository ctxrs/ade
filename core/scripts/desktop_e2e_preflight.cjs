#!/usr/bin/env node

const {
  resolveRequirement,
  resolveSuiteContract,
  suiteIds,
} = require("./desktop_e2e_secret_contract_lib.cjs");

const parseListArg = (value) =>
  String(value || "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);

const parseArgs = (argv) => {
  const opts = {
    suiteId: "",
    allowMissing: false,
    cellIds: [],
    caseIds: [],
    platform: "",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--suite") {
      opts.suiteId = String(argv[index + 1] || "").trim();
      index += 1;
      continue;
    }
    if (arg === "--cell") {
      opts.cellIds.push(...parseListArg(argv[index + 1]));
      index += 1;
      continue;
    }
    if (arg === "--case") {
      opts.caseIds.push(...parseListArg(argv[index + 1]));
      index += 1;
      continue;
    }
    if (arg === "--platform") {
      opts.platform = String(argv[index + 1] || "").trim();
      index += 1;
      continue;
    }
    if (arg === "--allow-missing") {
      opts.allowMissing = true;
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

const describeResolution = (resolution) => {
  switch (resolution.status) {
    case "present":
      return `${resolution.envName}: present via ${resolution.valueSource}`;
    case "default":
      return `${resolution.envName}: using default (${resolution.value})`;
    case "available":
      return `${resolution.envName}: available via ${resolution.valueSource}`;
    case "skipped":
      return `${resolution.envName}: not applicable`;
    case "invalid":
      return `${resolution.envName}: invalid (${resolution.errors.join("; ")})`;
    default:
      return `${resolution.envName}: missing`;
  }
};

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help || !opts.suiteId) {
    process.stdout.write(
      `usage: desktop_e2e_preflight.cjs --suite <${suiteIds.join("|")}> [--cell ID[,ID...]] [--case ID[,ID...]] [--platform darwin|linux|win32] [--allow-missing]\n`,
    );
    return;
  }

  const suite = resolveSuiteContract(opts.suiteId, {
    cellIds: opts.cellIds,
    caseIds: opts.caseIds,
    env: process.env,
    platform: opts.platform || process.platform,
  });
  const requiredResolutions = suite.requirements.map((requirement) =>
    resolveRequirement(requirement, { env: process.env, platform: suite.platform }),
  );
  const optionalResolutions = suite.optionalRequirements.map((requirement) =>
    resolveRequirement(requirement, { env: process.env, platform: suite.platform }),
  );

  process.stdout.write(`preflight suite: ${suite.id}\n`);
  process.stdout.write(`${suite.title}\n`);
  for (const resolution of requiredResolutions) {
    process.stdout.write(`- ${describeResolution(resolution)}\n`);
  }
  for (const resolution of optionalResolutions) {
    process.stdout.write(`- ${describeResolution(resolution)}\n`);
  }
  for (const note of suite.notes) {
    process.stdout.write(`note: ${note}\n`);
  }

  const blocking = requiredResolutions.filter((resolution) =>
    resolution.applies && resolution.required && (resolution.status === "missing" || resolution.status === "invalid"),
  );
  if (blocking.length === 0) {
    process.stdout.write("preflight passed\n");
    return;
  }

  process.stderr.write(`preflight failed: ${blocking.map((resolution) => resolution.envName).join(", ")}\n`);
  for (const resolution of blocking) {
    if (resolution.status === "missing") {
      process.stderr.write(`- ${resolution.envName}: missing\n`);
    } else {
      process.stderr.write(`- ${resolution.envName}: ${resolution.errors.join("; ")}\n`);
    }
  }
  if (opts.allowMissing) {
    process.stderr.write("allow-missing enabled; continuing despite preflight failure\n");
    return;
  }
  process.stderr.write("rerun with --allow-missing only when intentionally opting into local non-strict mode\n");
  process.exitCode = 1;
};

main();
