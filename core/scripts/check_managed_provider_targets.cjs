#!/usr/bin/env node
const fs = require("fs");
const path = require("path");

const usage = () => {
  console.error(
    "usage: node core/scripts/check_managed_provider_targets.cjs --provider <id> --target <os-arch> [--target <os-arch>]",
  );
};

const parseArgs = (argv) => {
  const options = {
    providerId: "",
    targets: [],
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    switch (flag) {
      case "--provider":
        if (!value) throw new Error("missing value for --provider");
        options.providerId = value;
        i += 1;
        break;
      case "--target":
        if (!value) throw new Error("missing value for --target");
        options.targets.push(value);
        i += 1;
        break;
      default:
        throw new Error(`unknown argument: ${flag}`);
    }
  }
  if (!options.providerId) throw new Error("missing required --provider");
  if (options.targets.length === 0) throw new Error("missing required --target");
  return options;
};

const main = () => {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage();
    throw err;
  }

  const matrixPath = path.resolve(__dirname, "..", "crates", "ctx-provider-accounts", "src", "provider_matrix.json");
  const raw = JSON.parse(fs.readFileSync(matrixPath, "utf8"));
  const providers = Array.isArray(raw?.providers) ? raw.providers : [];
  const provider = providers.find((entry) => entry?.id === options.providerId);
  if (!provider) {
    throw new Error(`provider not found: ${options.providerId}`);
  }
  const targets = provider?.managed_install?.targets;
  if (!targets || typeof targets !== "object") {
    throw new Error(`provider has no managed targets: ${options.providerId}`);
  }

  const missing = options.targets.filter((target) => !targets[target]);
  if (missing.length > 0) {
    throw new Error(
      `provider ${options.providerId} is missing managed target(s): ${missing.join(", ")} (provider_matrix.json)`,
    );
  }
  console.log(`ok: ${options.providerId} managed targets present: ${options.targets.join(", ")}`);
};

main();
