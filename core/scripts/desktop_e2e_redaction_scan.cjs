#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const {
  defaultAllowlistPath,
  envCarriesSecretPayload,
  envSpecs,
  resolveRequirement,
  resolveSuiteContract,
  suiteIds,
} = require("./desktop_e2e_secret_contract_lib.cjs");

const parseArgs = (argv) => {
  const opts = {
    suiteId: "",
    paths: [],
    allowlistPath: defaultAllowlistPath,
    includeDeferred: false,
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
    if (arg === "--path") {
      opts.paths.push(String(argv[index + 1] || "").trim());
      index += 1;
      continue;
    }
    if (arg === "--allowlist") {
      opts.allowlistPath = String(argv[index + 1] || "").trim();
      index += 1;
      continue;
    }
    if (arg === "--cell") {
      opts.cellIds.push(...String(argv[index + 1] || "").split(",").map((value) => value.trim()).filter(Boolean));
      index += 1;
      continue;
    }
    if (arg === "--case") {
      opts.caseIds.push(...String(argv[index + 1] || "").split(",").map((value) => value.trim()).filter(Boolean));
      index += 1;
      continue;
    }
    if (arg === "--platform") {
      opts.platform = String(argv[index + 1] || "").trim();
      index += 1;
      continue;
    }
    if (arg === "--include-deferred") {
      opts.includeDeferred = true;
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

const loadAllowlist = (allowlistPath) => {
  if (!allowlistPath || !fs.existsSync(allowlistPath)) return [];
  return fs.readFileSync(allowlistPath, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .map((line) => {
      const parts = line.split("\t");
      if (parts.length < 3) {
        throw new Error(`invalid allowlist row (expected path_regex<TAB>match_regex<TAB>justification): ${line}`);
      }
      return {
        pathRegex: new RegExp(parts[0]),
        matchRegex: new RegExp(parts[1]),
        justification: parts.slice(2).join("\t").trim(),
      };
    });
};

const collectFiles = (rootPath, results = []) => {
  if (!rootPath) return results;
  if (!fs.existsSync(rootPath)) return results;
  const stat = fs.statSync(rootPath);
  if (stat.isFile()) {
    results.push(path.resolve(rootPath));
    return results;
  }
  if (!stat.isDirectory()) return results;
  for (const entry of fs.readdirSync(rootPath, { withFileTypes: true })) {
    const next = path.join(rootPath, entry.name);
    if (entry.isDirectory()) {
      collectFiles(next, results);
    } else if (entry.isFile()) {
      results.push(path.resolve(next));
    }
  }
  return results;
};

const isProbablyText = (buffer) => {
  const limit = Math.min(buffer.length, 4096);
  for (let index = 0; index < limit; index += 1) {
    if (buffer[index] === 0) return false;
  }
  return true;
};

const escapeRegex = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const resolveRedactionSecrets = (suite) => {
  const seenEnvNames = new Set();
  const resolved = [];
  for (const requirement of [...suite.requirements, ...suite.optionalRequirements]) {
    const envName = String(requirement?.envName || "").trim();
    if (!envName || seenEnvNames.has(envName) || !envCarriesSecretPayload(envName)) continue;
    seenEnvNames.add(envName);
    resolved.push({
      envName,
      requirement,
      resolution: resolveRequirement(requirement, {
        env: process.env,
        platform: suite.platform,
      }),
    });
  }
  return resolved;
};

const buildPatterns = (resolvedSecrets) => {
  const patterns = [];
  const seenSecretValues = new Set();
  for (const { envName, resolution } of resolvedSecrets) {
    if (envSpecs[envName]?.kind === "secret") {
      patterns.push({
        kind: "named-assignment",
        envName,
        regex: new RegExp(`\\b${escapeRegex(envName)}\\b\\s*[:=]\\s*[^\\s"']+`, "g"),
      });
    }
    const secretValues = Array.isArray(resolution.secretValues) && resolution.secretValues.length > 0
      ? resolution.secretValues
      : (resolution.value ? [resolution.value] : []);
    for (const secretValue of secretValues) {
      const secretKey = `${envName}\u0000${secretValue}`;
      if (seenSecretValues.has(secretKey)) continue;
      seenSecretValues.add(secretKey);
      patterns.push({
        kind: "literal-secret",
        envName,
        regex: new RegExp(escapeRegex(secretValue), "g"),
      });
    }
  }
  patterns.push({
    kind: "authorization-bearer",
    envName: "authorization",
    regex: /\bAuthorization\b[^\n\r]*\bBearer\s+[A-Za-z0-9._-]{10,}/g,
  });
  return patterns;
};

const allowlisted = (entries, filePath, matchText) =>
  entries.some((entry) => entry.pathRegex.test(filePath) && entry.matchRegex.test(matchText));

const main = () => {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help || !opts.suiteId || opts.paths.length === 0) {
    process.stdout.write(
      `usage: desktop_e2e_redaction_scan.cjs --suite <${suiteIds.join("|")}> --path <file-or-dir> [--path ...] [--allowlist PATH] [--cell ID[,ID...]] [--case ID[,ID...]] [--platform darwin|linux|win32] [--include-deferred]\n`,
    );
    return;
  }

  const suite = resolveSuiteContract(opts.suiteId, {
    env: process.env,
    platform: opts.platform || process.platform,
    includeDeferred: opts.includeDeferred,
    cellIds: opts.cellIds,
    caseIds: opts.caseIds,
  });
  const allowlist = loadAllowlist(opts.allowlistPath);
  const resolvedSecrets = resolveRedactionSecrets(suite);
  const invalidResolutions = resolvedSecrets.filter(({ resolution }) => resolution.status === "invalid");
  if (invalidResolutions.length > 0) {
    process.stderr.write(`redaction scan invalid for ${suite.id}\n`);
    for (const { envName, resolution } of invalidResolutions) {
      process.stderr.write(`- ${envName}: ${resolution.errors.join("; ")}\n`);
    }
    process.exitCode = 1;
    return;
  }
  const patterns = buildPatterns(resolvedSecrets);
  const files = [];
  for (const candidate of opts.paths) {
    collectFiles(candidate, files);
  }

  const findings = [];
  for (const filePath of files) {
    const buffer = fs.readFileSync(filePath);
    if (!isProbablyText(buffer)) continue;
    const content = buffer.toString("utf8");
    for (const pattern of patterns) {
      pattern.regex.lastIndex = 0;
      let match = pattern.regex.exec(content);
      while (match) {
        const matchedText = String(match[0] || "");
        if (!allowlisted(allowlist, filePath, matchedText)) {
          findings.push({
            filePath,
            envName: pattern.envName,
            kind: pattern.kind,
            match: matchedText,
          });
        }
        match = pattern.regex.exec(content);
      }
    }
  }

  if (findings.length === 0) {
    process.stdout.write(`redaction scan passed for ${suite.id}\n`);
    return;
  }

  process.stderr.write(`redaction scan failed for ${suite.id}\n`);
  for (const finding of findings) {
    process.stderr.write(`- ${finding.filePath}: ${finding.kind} ${finding.envName} -> ${finding.match}\n`);
  }
  process.exitCode = 1;
};

main();
