#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");

const COLLAPSED_PATHS = [
  "core/crates/ctx-daemon/src/daemon/sessions/route_contract.rs",
  "core/crates/ctx-daemon/src/daemon/tasks/route_contract.rs",
  "core/crates/ctx-daemon/src/daemon/workspaces/route_contract.rs",
  "core/crates/ctx-daemon/src/daemon/workspaces/stream/handle.rs",
  "core/crates/ctx-daemon/src/daemon/workspaces/stream/subscriptions.rs",
  "core/crates/ctx-daemon/src/daemon/workspaces/run_archive.rs",
  "core/crates/ctx-session-service/src/head_projection.rs",
];

const RATCHETED_FILE_LIMITS = [
  {
    path: "core/crates/ctx-daemon/src/daemon/org_policy_route.rs",
    limit: 510,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/sessions/subagents_route.rs",
    limit: 620,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/sessions/message_route.rs",
    limit: 610,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/repo_onboarding.rs",
    limit: 590,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/terminals/route_contract.rs",
    limit: 590,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/tasks/lifecycle.rs",
    limit: 320,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/workspaces/management.rs",
    limit: 570,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/org_policy.rs",
    limit: 240,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/sessions/message_commands.rs",
    limit: 220,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/providers/codex_app_login.rs",
    limit: 520,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/providers/bootstrap.rs",
    limit: 510,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/sessions/handle.rs",
    limit: 600,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/workspaces.rs",
    limit: 600,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/tasks.rs",
    limit: 120,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/mobile_access.rs",
    limit: 120,
  },
];

const SERVICE_RUNTIME_FORBIDDEN_DEPS = new Set(["ctx-daemon", "ctx-http", "axum"]);
const PACKAGE_SHAPE_BOUNDARY_CRATES = new Set([
  "ctx-merge-queue",
  "ctx-org-policy",
  "ctx-provider-runtime",
  "ctx-resource-utilization",
  "ctx-route-contracts",
  "ctx-settings-service",
  "ctx-transport-runtime",
  "ctx-update-service",
  "ctx-mobile-access-service",
  "ctx-run-archive-service",
  "ctx-run-scheduler",
  "ctx-session-artifacts",
  "ctx-session-message-service",
  "ctx-session-runtime",
  "ctx-session-runner",
  "ctx-workspace-active-snapshot",
  "ctx-workspace-attachments",
  "ctx-workspace-config",
  "ctx-workspace-container",
  "ctx-workspace-runtime",
  "ctx-workspace-services",
]);
const PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS = new Set(["ctx-daemon", "ctx-http", "axum"]);
const MESSAGE_SERVICE_FORBIDDEN_DEPS = new Set([
  "ctx-session-service",
  "ctx-daemon",
  "ctx-http",
  "axum",
]);
const SESSION_RUNTIME_FORBIDDEN_DEPS = new Set([
  "ctx-session-service",
  "ctx-daemon",
  "ctx-http",
  "ctx-workspace-active-snapshot",
  "ctx-route-contracts",
  "axum",
]);
const TRANSPORT_RUNTIME_FORBIDDEN_DEPS = new Set(["ctx-store"]);
const ROUTE_CONTRACTS_ALLOWED_CTX_DEPS = new Set(["ctx-core"]);
const HEAD_PROJECTION_ROOT = "core/crates/ctx-session-runtime/src/head_projection";

const toPosix = (value) => value.split(path.sep).join("/");

const countLines = (raw) => {
  if (raw.length === 0) return 0;
  const parts = raw.split(/\r?\n/u);
  return raw.endsWith("\n") ? parts.length - 1 : parts.length;
};

const stripTomlComment = (line) => line.replace(/\s+#.*$/u, "");

const unquoteTomlKey = (value) => value.trim().replace(/^["']|["']$/gu, "");

const dependencyNameFromKey = (key) => unquoteTomlKey(key.split(".")[0] || "");

const isProdDependencySection = (section) =>
  section === "dependencies" || /^target\..+\.dependencies$/u.test(section);

const dependencyTableNameFromSection = (section) => {
  if (section.startsWith("dependencies.")) {
    return unquoteTomlKey(section.slice("dependencies.".length));
  }
  const targetMatch = section.match(/^target\..+\.dependencies\.(.+)$/u);
  return targetMatch ? unquoteTomlKey(targetMatch[1]) : "";
};

const isProdDependencyTable = (section) =>
  section.startsWith("dependencies.")
  || /^target\..+\.dependencies\./u.test(section);

const packageNameFromInlineValue = (value) => {
  const match = value.match(/\bpackage\s*=\s*["']([^"']+)["']/u);
  return match ? match[1] : "";
};

const packageNameFromLine = (trimmed) => {
  const match = trimmed.match(/^package\s*=\s*["']([^"']+)["']/u);
  return match ? match[1] : "";
};

const startsMultilineInlineTable = (value) =>
  value.trim().startsWith("{") && !value.includes("}");

const parseCargoDependencies = (raw) => {
  const dependencies = [];
  let currentSection = "";
  let currentDependencyTable = "";
  let currentInlineDependencyTable = null;

  const addDependency = ({ line, name, section }) => {
    if (!name) return;
    dependencies.push({
      line,
      name,
      section,
    });
  };

  const lines = raw.split(/\r?\n/u);
  for (let index = 0; index < lines.length; index += 1) {
    const lineNumber = index + 1;
    const trimmed = stripTomlComment(lines[index]).trim();
    if (trimmed.length === 0) continue;

    if (currentInlineDependencyTable) {
      addDependency({
        line: lineNumber,
        name: packageNameFromLine(trimmed),
        section: currentInlineDependencyTable.section,
      });
      if (trimmed.includes("}")) {
        currentInlineDependencyTable = null;
      }
      continue;
    }

    const sectionMatch = trimmed.match(/^\[([^\]]+)\]$/u);
    if (sectionMatch) {
      currentSection = sectionMatch[1].trim();
      currentDependencyTable = dependencyTableNameFromSection(currentSection);
      if (isProdDependencyTable(currentSection)) {
        addDependency({
          line: lineNumber,
          name: currentDependencyTable,
          section: currentSection,
        });
      }
      continue;
    }

    if (currentDependencyTable && isProdDependencyTable(currentSection)) {
      addDependency({
        line: lineNumber,
        name: packageNameFromLine(trimmed),
        section: currentSection,
      });
      continue;
    }

    if (!isProdDependencySection(currentSection)) continue;
    const keyValueMatch = trimmed.match(/^([^=]+?)\s*=\s*(.+)$/u);
    if (!keyValueMatch) continue;

    addDependency({
      line: lineNumber,
      name: dependencyNameFromKey(keyValueMatch[1]),
      section: currentSection,
    });
    addDependency({
      line: lineNumber,
      name: packageNameFromInlineValue(keyValueMatch[2]),
      section: currentSection,
    });
    if (startsMultilineInlineTable(keyValueMatch[2])) {
      currentInlineDependencyTable = {
        section: currentSection,
      };
    }
  }

  return dependencies;
};

const packageNameFromCargoToml = (raw) => {
  let inPackageSection = false;
  for (const line of raw.split(/\r?\n/u)) {
    const trimmed = stripTomlComment(line).trim();
    const sectionMatch = trimmed.match(/^\[([^\]]+)\]$/u);
    if (sectionMatch) {
      inPackageSection = sectionMatch[1].trim() === "package";
      continue;
    }
    if (!inPackageSection) continue;
    const nameMatch = trimmed.match(/^name\s*=\s*["']([^"']+)["']/u);
    if (nameMatch) return nameMatch[1];
  }
  return "";
};

const isServiceOrRuntimeCrate = (crateName) =>
  crateName.endsWith("-service")
  || crateName.endsWith("-services")
  || crateName.endsWith("-runtime")
  || crateName.includes("-runtime-");

const isPackageShapeBoundaryCrate = (crateName) =>
  PACKAGE_SHAPE_BOUNDARY_CRATES.has(crateName)
  || /^ctx-(?:task|subagent|run-scheduler|session-runner|mobile-access|artifacts|session-artifacts|run-archive|web-session|workspace-stream)-service$/u.test(crateName);

const isWorkspaceActiveSnapshotForbiddenDependency = (dependencyName) => {
  if (dependencyName === "ctx-daemon" || dependencyName === "ctx-store") return true;
  if (dependencyName.startsWith("ctx-http")) return true;
  if (dependencyName === "ctx-transport-runtime") return true;
  return dependencyName.endsWith("-runtime") || dependencyName.includes("-runtime-");
};

const isRouteContractsForbiddenDependency = (dependencyName) => {
  if (dependencyName === "axum") return true;
  if (!dependencyName.startsWith("ctx-")) return false;
  return !ROUTE_CONTRACTS_ALLOWED_CTX_DEPS.has(dependencyName);
};

const listCargoManifests = (rootDir) => {
  const cratesRoot = path.join(rootDir, "core", "crates");
  if (!fs.existsSync(cratesRoot)) return [];
  return fs.readdirSync(cratesRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(cratesRoot, entry.name, "Cargo.toml"))
    .filter((manifestPath) => fs.existsSync(manifestPath))
    .sort();
};

const walkFiles = (dirPath, output = []) => {
  if (!fs.existsSync(dirPath)) return output;
  for (const entry of fs.readdirSync(dirPath, { withFileTypes: true })) {
    const absolutePath = path.join(dirPath, entry.name);
    if (entry.isDirectory()) {
      walkFiles(absolutePath, output);
      continue;
    }
    output.push(absolutePath);
  }
  return output;
};

const stripRustLineComments = (contents) =>
  contents
    .split(/\r?\n/u)
    .map((line) => line.replace(/\/\/.*$/u, ""))
    .join("\n");

const HEAD_PROJECTION_FORBIDDEN_IMPORT_PATTERNS = [
  {
    name: "daemon import",
    pattern: /\bctx_daemon\b/u,
  },
  {
    name: "HTTP import",
    pattern: /\bctx_http(?:_[A-Za-z0-9_]+)?\b/u,
  },
  {
    name: "store import",
    pattern: /\bctx_store\b/u,
  },
  {
    name: "provider import",
    pattern: /\bctx_(?:providers|provider_(?!runtime\b)[A-Za-z0-9_]+)\b/u,
  },
  {
    name: "transport import",
    pattern: /\bctx_transport_(?!runtime\b)[A-Za-z0-9_]+\b/u,
  },
  {
    name: "runtime import",
    pattern: /\bctx_[A-Za-z0-9_]*runtime[A-Za-z0-9_]*\b/u,
  },
];

const checkCollapsedPaths = (rootDir) =>
  COLLAPSED_PATHS
    .filter((relativePath) => fs.existsSync(path.join(rootDir, relativePath)))
    .map((relativePath) => ({
      kind: "collapsed_path",
      path: relativePath,
      message: `${relativePath} must not be reintroduced; keep the decomposed directory/module layout.`,
    }));

const checkRatchetedFileCaps = (rootDir) => {
  const violations = [];
  for (const entry of RATCHETED_FILE_LIMITS) {
    const absolutePath = path.join(rootDir, entry.path);
    if (!fs.existsSync(absolutePath)) continue;
    const lineCount = countLines(fs.readFileSync(absolutePath, "utf8"));
    if (lineCount <= entry.limit) continue;
    violations.push({
      kind: "file_cap",
      lineCount,
      limit: entry.limit,
      path: entry.path,
      message: `${entry.path} is ${lineCount} lines (limit ${entry.limit}).`,
    });
  }
  return violations;
};

const checkCargoDependencyDirection = (rootDir) => {
  const violations = [];
  for (const manifestPath of listCargoManifests(rootDir)) {
    const raw = fs.readFileSync(manifestPath, "utf8");
    const crateName = packageNameFromCargoToml(raw);
    const manifestRelativePath = toPosix(path.relative(rootDir, manifestPath));
    const dependencies = parseCargoDependencies(raw);

    for (const dependency of dependencies) {
      if (isServiceOrRuntimeCrate(crateName) && SERVICE_RUNTIME_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `${crateName} must not depend on ${dependency.name}; service/runtime crates cannot depend on daemon, HTTP, or Axum.`,
        });
      }
      if (isPackageShapeBoundaryCrate(crateName) && PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `${crateName} must not depend on ${dependency.name}; package-shape boundary crates cannot depend on daemon, HTTP, or Axum.`,
        });
      }
      if (crateName === "ctx-session-message-service" && MESSAGE_SERVICE_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-session-message-service must not depend on ${dependency.name}; message persistence must not couple back to session orchestration, daemon, HTTP, or Axum.`,
        });
      }
      if (crateName === "ctx-session-runtime" && SESSION_RUNTIME_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-session-runtime must not depend on ${dependency.name}; session runtime must stay below session orchestration, daemon, HTTP, active-snapshot runtime, route contracts, and Axum.`,
        });
      }
      if (crateName === "ctx-transport-runtime" && TRANSPORT_RUNTIME_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: "ctx-transport-runtime must not depend on ctx-store.",
        });
      }
      if (crateName === "ctx-route-contracts" && isRouteContractsForbiddenDependency(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-route-contracts must stay DTO-only; found forbidden dependency ${dependency.name}.`,
        });
      }
      if (crateName === "ctx-workspace-active-snapshot" && isWorkspaceActiveSnapshotForbiddenDependency(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-workspace-active-snapshot must stay daemon/http/store/runtime-free; found ${dependency.name}.`,
        });
      }
    }
  }
  return violations;
};

const checkHeadProjectionPurity = (rootDir) => {
  const headProjectionRoot = path.join(rootDir, HEAD_PROJECTION_ROOT);
  const violations = [];
  for (const absolutePath of walkFiles(headProjectionRoot).filter((entry) => entry.endsWith(".rs"))) {
    const relativePath = toPosix(path.relative(rootDir, absolutePath));
    const contents = stripRustLineComments(fs.readFileSync(absolutePath, "utf8"));
    for (const pattern of HEAD_PROJECTION_FORBIDDEN_IMPORT_PATTERNS) {
      if (!pattern.pattern.test(contents)) continue;
      violations.push({
        kind: "head_projection_import",
        path: relativePath,
        message: `${relativePath} contains a forbidden ${pattern.name}; head_projection must stay projection-only.`,
      });
    }
  }
  return violations;
};

const evaluateDecompositionBoundaries = (rootDir = repoRoot) => {
  const violations = [
    ...checkCollapsedPaths(rootDir),
    ...checkRatchetedFileCaps(rootDir),
    ...checkCargoDependencyDirection(rootDir),
    ...checkHeadProjectionPurity(rootDir),
  ];
  return { violations };
};

const printViolations = ({ stream = process.stderr, violations }) => {
  if (violations.length === 0) return;
  stream.write("ctx daemon decomposition boundary violations:\n");
  for (const violation of violations) {
    const location = violation.line ? `${violation.path}:${violation.line}` : violation.path;
    stream.write(`  - ${location}: ${violation.message}\n`);
  }
  stream.write("\n");
  stream.write("Fix the boundary regression directly. Do not add compatibility shims, fallback imports, or broader source-size exceptions.\n");
};

const run = ({
  rootDir = repoRoot,
  stdout = process.stdout,
  stderr = process.stderr,
} = {}) => {
  const result = evaluateDecompositionBoundaries(rootDir);
  printViolations({
    ...result,
    stream: result.violations.length === 0 ? stdout : stderr,
  });
  return result.violations.length === 0 ? 0 : 1;
};

if (require.main === module) {
  process.exitCode = run();
}

module.exports = {
  COLLAPSED_PATHS,
  HEAD_PROJECTION_FORBIDDEN_IMPORT_PATTERNS,
  MESSAGE_SERVICE_FORBIDDEN_DEPS,
  PACKAGE_SHAPE_BOUNDARY_CRATES,
  PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS,
  RATCHETED_FILE_LIMITS,
  ROUTE_CONTRACTS_ALLOWED_CTX_DEPS,
  SERVICE_RUNTIME_FORBIDDEN_DEPS,
  SESSION_RUNTIME_FORBIDDEN_DEPS,
  checkCargoDependencyDirection,
  checkCollapsedPaths,
  checkHeadProjectionPurity,
  checkRatchetedFileCaps,
  countLines,
  evaluateDecompositionBoundaries,
  isPackageShapeBoundaryCrate,
  isRouteContractsForbiddenDependency,
  isServiceOrRuntimeCrate,
  isWorkspaceActiveSnapshotForbiddenDependency,
  packageNameFromCargoToml,
  parseCargoDependencies,
  run,
};
