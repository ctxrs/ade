#!/usr/bin/env node

const childProcess = require("node:child_process");

const {
  buildExecutionPlan,
  normalizeRepoRelativePath,
} = require("./lib/test_taxonomy/execution.cjs");
const { getFamiliesById } = require("./lib/test_taxonomy/families.cjs");
const { getProfiles } = require("./lib/test_taxonomy/profiles.cjs");
const {
  resolveMergeBaseFiles,
  resolveWorkingTreeFiles,
} = require("./lib/verification_git_changes.cjs");

function parseArgs(argv) {
  const args = {
    base: "",
    changedFiles: [],
    checkNonEmpty: false,
    json: false,
    keepGoing: false,
    list: false,
    listProfiles: false,
    profile: "",
    run: false,
    selectionMode: "",
    touchedOnly: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--profile") {
      args.profile = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--changed-file") {
      args.changedFiles.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--base") {
      args.base = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--touched-only") {
      args.touchedOnly = true;
    } else if (arg === "--selection-mode") {
      args.selectionMode = argv[index + 1] || "";
      index += 1;
    } else if (arg === "--check-nonempty") {
      args.checkNonEmpty = true;
    } else if (arg === "--json") {
      args.json = true;
    } else if (arg === "--run") {
      args.run = true;
    } else if (arg === "--keep-going") {
      args.keepGoing = true;
    } else if (arg === "--list") {
      args.list = true;
    } else if (arg === "--list-profiles") {
      args.listProfiles = true;
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }

  if (!args.profile && !args.listProfiles) {
    throw new Error("--profile is required");
  }
  return args;
}

function resolveChangedFiles(args) {
  if (args.changedFiles.length > 0) {
    return args.changedFiles.map(normalizeRepoRelativePath).filter(Boolean);
  }
  if (args.base) {
    return resolveMergeBaseFiles(args.base).changedFiles;
  }
  if (args.touchedOnly) {
    return resolveWorkingTreeFiles();
  }
  return [];
}

function commandFailureExitStatus(failure) {
  if (typeof failure.status === "number") {
    return failure.status;
  }
  return 1;
}

function runCommands(commands, options = {}) {
  const {
    keepGoing = false,
    spawnSyncImpl = childProcess.spawnSync,
    stderr = process.stderr,
  } = options;
  const failures = [];

  for (const [index, command] of commands.entries()) {
    const result = spawnSyncImpl("bash", ["-lc", command], {
      stdio: "inherit",
    });
    if (result.error) {
      if (!keepGoing) {
        throw result.error;
      }
      failures.push({
        command,
        error: result.error,
        index,
        status: 1,
      });
      stderr.write(
        `taxonomy profile command ${index + 1}/${commands.length} failed to start: ${result.error.message}\n`,
      );
      continue;
    }
    if (result.status !== 0) {
      const status = result.status ?? 1;
      if (!keepGoing) {
        return status;
      }
      failures.push({
        command,
        index,
        signal: result.signal,
        status,
      });
      stderr.write(
        `taxonomy profile command ${index + 1}/${commands.length} failed with status ${status}: ${command}\n`,
      );
    }
  }

  if (failures.length > 0) {
    stderr.write("\nTaxonomy profile failures:\n");
    for (const failure of failures) {
      const status = commandFailureExitStatus(failure);
      const signalSuffix = failure.signal ? ` signal=${failure.signal}` : "";
      const errorSuffix = failure.error ? ` error=${failure.error.message}` : "";
      stderr.write(
        `- ${failure.index + 1}/${commands.length} status=${status}${signalSuffix}${errorSuffix}: ${failure.command}\n`,
      );
    }
    return commandFailureExitStatus(failures[0]);
  }

  return 0;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.listProfiles) {
    const profiles = getProfiles(getFamiliesById())
      .map((profile) => profile.id)
      .sort();
    for (const profileId of profiles) {
      process.stdout.write(`${profileId}\n`);
    }
    return;
  }
  const changedFiles = resolveChangedFiles(args);
  const plan = buildExecutionPlan({
    profileId: args.profile,
    changedFiles,
    selectionMode: args.selectionMode,
    touchedOnly: args.touchedOnly,
  });

  if (args.checkNonEmpty) {
    process.exit(plan.commands.length > 0 ? 0 : 1);
  }
  if (args.json) {
    process.stdout.write(`${JSON.stringify({
      changedFiles,
      commands: plan.commands,
      entries: plan.selectedEntries.map((entry) => entry.id),
      profile: plan.profile.id,
    }, null, 2)}\n`);
    return;
  }
  if (args.run) {
    process.exit(runCommands(plan.commands, { keepGoing: args.keepGoing }));
  }
  if (args.list || true) {
    for (const command of plan.commands) {
      process.stdout.write(`${command}\n`);
    }
  }
}

module.exports = {
  parseArgs,
  resolveChangedFiles,
  runCommands,
};

if (require.main === module) {
  main();
}
