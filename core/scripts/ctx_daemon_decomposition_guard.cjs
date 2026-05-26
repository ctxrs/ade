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
  "core/crates/ctx-workspace-services/src/repo_onboarding.rs",
  "core/crates/ctx-workspace-services/src/workspace_attachments.rs",
  "core/crates/ctx-workspace-services/src/workspace_attachments/doc_mirror.rs",
  "core/crates/ctx-workspace-services/src/workspace_attachments/materialized_install.rs",
  "core/crates/ctx-workspace-services/src/workspace_attachments/materialized_paths.rs",
  "core/crates/ctx-workspace-services/src/workspace_attachments/reference_repo.rs",
  "core/crates/ctx-workspace-services/src/workspace_attachments/tests.rs",
];

const COLLAPSED_DIRECTORIES = [
  "core/crates/ctx-workspace-services",
  "core/crates/ctx-workspace-services/src/repo_onboarding",
  "core/crates/ctx-workspace-services/src/workspace_attachments",
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
    path: "core/crates/ctx-daemon/src/daemon/task_route_handles.rs",
    limit: 1920,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/session_route_handles.rs",
    limit: 2200,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/workspace_stream_route_handles.rs",
    limit: 900,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/launch_route_handles.rs",
    limit: 420,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/maintenance_route_handles.rs",
    limit: 150,
  },
  {
    path: "core/crates/ctx-daemon/src/daemon/mobile_route_handles.rs",
    limit: 120,
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
  "ctx-session-title-service",
  "ctx-session-vcs-service",
  "ctx-workspace-active-snapshot",
  "ctx-workspace-attachments",
  "ctx-workspace-config",
  "ctx-workspace-container",
  "ctx-workspace-runtime",
  "ctx-workspace-services",
  "ctx-worktree-bootstrap-service",
  "ctx-worktree-vcs-service",
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
const TITLE_SERVICE_FORBIDDEN_DEPS = new Set([
  "ctx-session-service",
  "ctx-daemon",
  "ctx-http",
  "ctx-route-contracts",
  "axum",
]);
const SESSION_VCS_SERVICE_FORBIDDEN_DEPS = new Set([
  "ctx-session-service",
  "ctx-daemon",
  "ctx-http",
  "ctx-route-contracts",
  "ctx-workspace-services",
  "ctx-worktree-vcs-service",
  "axum",
]);
const WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS = new Set([
  "ctx-session-service",
  "ctx-session-vcs-service",
  "ctx-daemon",
  "ctx-http",
  "ctx-route-contracts",
  "ctx-workspace-services",
  "ctx-worktree-data-plane",
  "ctx-execution-runtime",
  "ctx-harness-runtime",
  "ctx-linux-sandbox-runtime",
  "ctx-sandbox-container-runtime",
  "ctx-avf-linux-runtime",
  "ctx-workspace-container",
  "ctx-workspace-runtime",
  "ctx-transport-runtime",
  "axum",
]);
const WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS = new Set([
  "ctx-session-service",
  "ctx-session-runtime",
  "ctx-daemon",
  "ctx-http",
  "ctx-route-contracts",
  "ctx-workspace-services",
  "ctx-workspace-config",
  "ctx-store",
  "ctx-workspace-active-snapshot",
  "ctx-worktree-data-plane",
  "ctx-execution-runtime",
  "ctx-harness-runtime",
  "ctx-linux-sandbox-runtime",
  "ctx-provider-runtime",
  "ctx-runtime-assets",
  "ctx-sandbox-container-runtime",
  "ctx-avf-linux-runtime",
  "ctx-workspace-container",
  "ctx-transport-runtime",
  "ctx-workspace-runtime",
  "axum",
]);
const WORKSPACE_ATTACHMENTS_FORBIDDEN_DEPS = new Set(["ctx-workspace-services"]);
const REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS = new Set([
  "ctx-workspace-services",
  "ctx-daemon",
  "ctx-http",
  "ctx-route-contracts",
  "ctx-store",
  "ctx-workspace-config",
  "ctx-workspace-container",
  "ctx-workspace-runtime",
  "ctx-worktree-data-plane",
  "ctx-execution-runtime",
  "ctx-harness-runtime",
  "ctx-linux-sandbox-runtime",
  "ctx-sandbox-container-runtime",
  "ctx-transport-runtime",
  "axum",
]);
const WORKSPACE_SERVICES_FORBIDDEN_DEPS = new Set(["ctx-repo-onboarding-service"]);
const CTX_HTTP_FORBIDDEN_DOMAIN_SERVICE_DEPS = new Set(["ctx-worktree-vcs-service"]);
const CTX_HTTP_CLI_ONLY_SERVICE_DEPS = new Set(["ctx-repo-onboarding-service"]);
const TRANSPORT_RUNTIME_FORBIDDEN_DEPS = new Set(["ctx-store"]);
const ROUTE_CONTRACTS_ALLOWED_CTX_DEPS = new Set(["ctx-core"]);
const HEAD_PROJECTION_ROOT = "core/crates/ctx-session-runtime/src/head_projection";
const CTX_HTTP_CLI_MAIN = "core/crates/ctx-http/src/main.rs";
const DAEMON_ROOT_ROUTE_FACADE_TARGETS = [
  "core/crates/ctx-daemon/src/daemon/tasks.rs",
  "core/crates/ctx-daemon/src/daemon/merge_queue.rs",
  "core/crates/ctx-daemon/src/daemon/terminals.rs",
  "core/crates/ctx-daemon/src/daemon/web_sessions.rs",
];
const DAEMON_ROOT_WEB_SESSION_TRANSPORT_FACADE_DENY = new Set([
  "WebSessionActionError",
  "WebSessionSignalBridgeError",
  "WebSessionSignalUpstream",
  "WebSessionSignalViewerGuard",
  "WebSessionViewConnectPath",
  "WebSessionViewPage",
]);
const DAEMON_HANDLE_STORE_LOOKUP_HOME = "core/crates/ctx-daemon/src/daemon/handle.rs";
const DAEMON_HANDLE_STORE_LOOKUP_STATE_HOME =
  "core/crates/ctx-daemon/src/daemon/state/store_lookup.rs";
const DAEMON_HANDLE_STORE_LOOKUP_SYMBOLS = [
  "ProtectedWorkspaceStoreLookup",
  "SessionStoreLookup",
  "TaskStoreLookup",
  "session_store_access_anyhow",
  "reject_archived_subagent_session",
  "is_transient_store_open_error",
  "scoped_mcp_session_store_error",
];

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

const isDevDependencySection = (section) =>
  section === "dev-dependencies" || /^target\..+\.dev-dependencies$/u.test(section);

const isDependencySection = (section, { includeDev = false } = {}) =>
  isProdDependencySection(section) || (includeDev && isDevDependencySection(section));

const dependencyTableNameFromSection = (section) => {
  if (section.startsWith("dependencies.")) {
    return unquoteTomlKey(section.slice("dependencies.".length));
  }
  const targetMatch = section.match(/^target\..+\.dependencies\.(.+)$/u);
  if (targetMatch) return unquoteTomlKey(targetMatch[1]);
  if (section.startsWith("dev-dependencies.")) {
    return unquoteTomlKey(section.slice("dev-dependencies.".length));
  }
  const targetDevMatch = section.match(/^target\..+\.dev-dependencies\.(.+)$/u);
  return targetDevMatch ? unquoteTomlKey(targetDevMatch[1]) : "";
};

const isProdDependencyTable = (section) =>
  section.startsWith("dependencies.")
  || /^target\..+\.dependencies\./u.test(section);

const isDevDependencyTable = (section) =>
  section.startsWith("dev-dependencies.")
  || /^target\..+\.dev-dependencies\./u.test(section);

const isDependencyTable = (section, { includeDev = false } = {}) =>
  isProdDependencyTable(section) || (includeDev && isDevDependencyTable(section));

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

const parseCargoDependencies = (raw, { includeDev = false } = {}) => {
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
      if (isDependencyTable(currentSection, { includeDev })) {
        addDependency({
          line: lineNumber,
          name: currentDependencyTable,
          section: currentSection,
        });
      }
      continue;
    }

    if (currentDependencyTable && isDependencyTable(currentSection, { includeDev })) {
      addDependency({
        line: lineNumber,
        name: packageNameFromLine(trimmed),
        section: currentSection,
      });
      continue;
    }

    if (!isDependencySection(currentSection, { includeDev })) continue;
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

const lineForOffset = (contents, offset) => contents.slice(0, offset).split(/\r?\n/u).length;

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

const checkCollapsedPaths = (rootDir) => [
  ...COLLAPSED_PATHS
    .filter((relativePath) => fs.existsSync(path.join(rootDir, relativePath)))
    .map((relativePath) => ({
      kind: "collapsed_path",
      path: relativePath,
      message: `${relativePath} must not be reintroduced; keep the decomposed directory/module layout.`,
    })),
  ...COLLAPSED_DIRECTORIES
    .filter((relativePath) => fs.existsSync(path.join(rootDir, relativePath)))
    .map((relativePath) => ({
      kind: "collapsed_path",
      path: relativePath,
      message: `${relativePath} must not be reintroduced; keep retired module directories removed.`,
    })),
];

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
    const dependencies = parseCargoDependencies(raw, {
      includeDev:
        crateName === "ctx-http"
        || crateName === "ctx-repo-onboarding-service"
        || crateName === "ctx-session-vcs-service"
        || crateName === "ctx-workspace-attachments"
        || crateName === "ctx-workspace-services"
        || crateName === "ctx-worktree-vcs-service"
        || crateName === "ctx-worktree-bootstrap-service",
    });

    for (const dependency of dependencies) {
      const dependencyIsProd = isProdDependencySection(dependency.section)
        || isProdDependencyTable(dependency.section);

      if (dependencyIsProd && isServiceOrRuntimeCrate(crateName) && SERVICE_RUNTIME_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `${crateName} must not depend on ${dependency.name}; service/runtime crates cannot depend on daemon, HTTP, or Axum.`,
        });
      }
      if (dependencyIsProd && isPackageShapeBoundaryCrate(crateName) && PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS.has(dependency.name)) {
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
      if (crateName === "ctx-session-title-service" && TITLE_SERVICE_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-session-title-service must not depend on ${dependency.name}; title generation must stay below session orchestration, daemon, HTTP, route contracts, and Axum.`,
        });
      }
      if (crateName === "ctx-session-vcs-service" && SESSION_VCS_SERVICE_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-session-vcs-service must not depend on ${dependency.name}; session VCS policy must stay below session orchestration, daemon, HTTP, route contracts, raw workspace VCS IO, and Axum.`,
        });
      }
      if (crateName === "ctx-worktree-vcs-service" && WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-worktree-vcs-service must not depend on ${dependency.name}; raw worktree VCS IO must stay below session VCS, broad workspace services, daemon, HTTP, route contracts, runtime/container/data-plane wiring, and Axum.`,
        });
      }
      if (crateName === "ctx-worktree-bootstrap-service" && WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-worktree-bootstrap-service must not depend on ${dependency.name}; bootstrap command/log policy must stay below daemon runtime wiring, route contracts, workspace config/store, container runtimes, HTTP, and Axum.`,
        });
      }
      if (crateName === "ctx-workspace-attachments" && WORKSPACE_ATTACHMENTS_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: "ctx-workspace-attachments must not depend on ctx-workspace-services; attachment policy and mount behavior must stay in the attachment crate.",
        });
      }
      if (crateName === "ctx-repo-onboarding-service" && REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-repo-onboarding-service must not depend on ${dependency.name}; repo onboarding policy must stay below daemon, HTTP, route contracts, broad workspace services, and runtime/container wiring.`,
        });
      }
      if (crateName === "ctx-workspace-services" && WORKSPACE_SERVICES_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: "ctx-workspace-services must not depend on ctx-repo-onboarding-service; do not reintroduce a repo onboarding compatibility bridge.",
        });
      }
      if (crateName === "ctx-http" && CTX_HTTP_FORBIDDEN_DOMAIN_SERVICE_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-http must not depend on ${dependency.name}; HTTP routes must go through daemon handles and route contracts.`,
        });
      }
      if (dependencyIsProd && crateName === "ctx-transport-runtime" && TRANSPORT_RUNTIME_FORBIDDEN_DEPS.has(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: "ctx-transport-runtime must not depend on ctx-store.",
        });
      }
      if (dependencyIsProd && crateName === "ctx-route-contracts" && isRouteContractsForbiddenDependency(dependency.name)) {
        violations.push({
          kind: "cargo_dependency",
          line: dependency.line,
          path: manifestRelativePath,
          message: `ctx-route-contracts must stay DTO-only; found forbidden dependency ${dependency.name}.`,
        });
      }
      if (dependencyIsProd && crateName === "ctx-workspace-active-snapshot" && isWorkspaceActiveSnapshotForbiddenDependency(dependency.name)) {
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

const checkCtxHttpCliOnlyServiceUsage = (rootDir) => {
  const srcRoot = path.join(rootDir, "core", "crates", "ctx-http", "src");
  const violations = [];
  for (const absolutePath of walkFiles(srcRoot).filter((entry) => entry.endsWith(".rs"))) {
    const relativePath = toPosix(path.relative(rootDir, absolutePath));
    if (relativePath === CTX_HTTP_CLI_MAIN) continue;
    const contents = stripRustLineComments(fs.readFileSync(absolutePath, "utf8"));
    for (const serviceCrate of CTX_HTTP_CLI_ONLY_SERVICE_DEPS) {
      const crateIdent = serviceCrate.replaceAll("-", "_");
      if (!new RegExp(`\\b${crateIdent}\\b`, "u").test(contents)) continue;
      violations.push({
        kind: "ctx_http_cli_only_service_import",
        path: relativePath,
        message: `${serviceCrate} is allowed in ctx-http only for the CLI entrypoint (${CTX_HTTP_CLI_MAIN}); HTTP library and routes must not call repo/workspace onboarding services directly.`,
      });
    }
  }
  return violations;
};

const checkDaemonRootRouteFacades = (rootDir) => {
  const violations = [];
  const routeContractReexportPattern =
    /\bpub\s+use\s+(?:(?:self|crate)\s*::\s*)?(?:daemon\s*::\s*[A-Za-z_][A-Za-z0-9_]*\s*::\s*)?route_contract\s*::/u;
  const webSessionTransportReexportPattern =
    /\bpub\s+use\s+ctx_transport_runtime\s*::\s*web_sessions\s*::[\s\S]*?;/gu;

  for (const relativePath of DAEMON_ROOT_ROUTE_FACADE_TARGETS) {
    const absolutePath = path.join(rootDir, relativePath);
    if (!fs.existsSync(absolutePath)) continue;
    const contents = stripRustLineComments(fs.readFileSync(absolutePath, "utf8"));
    if (routeContractReexportPattern.test(contents)) {
      violations.push({
        kind: "daemon_root_route_facade",
        path: relativePath,
        message: `${relativePath} must not publicly reexport route_contract symbols; callers should use the focused route-contract module or owner crate.`,
      });
    }
    if (!relativePath.endsWith("/web_sessions.rs")) continue;
    for (const match of contents.matchAll(webSessionTransportReexportPattern)) {
      const statement = match[0];
      if (/\*\s*;/u.test(statement)) {
        violations.push({
          kind: "daemon_root_route_facade",
          path: relativePath,
          message: `${relativePath} must not publicly glob-reexport ctx_transport_runtime::web_sessions; use owner-crate symbols directly.`,
        });
        continue;
      }
      for (const name of DAEMON_ROOT_WEB_SESSION_TRANSPORT_FACADE_DENY) {
        if (!new RegExp(`\\b${name}\\b`, "u").test(statement)) continue;
        violations.push({
          kind: "daemon_root_route_facade",
          path: relativePath,
          message: `${relativePath} must not publicly reexport ${name} from ctx_transport_runtime::web_sessions; use the owner crate directly.`,
        });
      }
    }
  }
  return violations;
};

const checkDaemonHandleStoreLookupOwnership = (rootDir) => {
  const violations = [];
  const handlePath = path.join(rootDir, DAEMON_HANDLE_STORE_LOOKUP_HOME);
  if (fs.existsSync(handlePath)) {
    const contents = stripRustLineComments(fs.readFileSync(handlePath, "utf8"));
    for (const symbol of DAEMON_HANDLE_STORE_LOOKUP_SYMBOLS) {
      const definitionRegex = new RegExp(
        String.raw`\b(?:(?:struct|fn)\s+${symbol}\b|impl(?:\s*<[^>]*>)?\s+${symbol}\b)`,
        "gu",
      );
      for (
        let match = definitionRegex.exec(contents);
        match;
        match = definitionRegex.exec(contents)
      ) {
        violations.push({
          kind: "daemon_handle_store_lookup_ownership",
          line: lineForOffset(contents, match.index),
          path: DAEMON_HANDLE_STORE_LOOKUP_HOME,
          message: `${symbol} must live in ${DAEMON_HANDLE_STORE_LOOKUP_STATE_HOME}; do not reintroduce store lookup ownership into daemon/handle.rs.`,
        });
      }
    }
  }

  const daemonSrcRoot = path.join(rootDir, "core", "crates", "ctx-daemon", "src");
  for (const absolutePath of walkFiles(daemonSrcRoot).filter((entry) => entry.endsWith(".rs"))) {
    const relativePath = toPosix(path.relative(rootDir, absolutePath));
    if (relativePath === DAEMON_HANDLE_STORE_LOOKUP_HOME) continue;
    const contents = stripRustLineComments(fs.readFileSync(absolutePath, "utf8"));
    for (const symbol of DAEMON_HANDLE_STORE_LOOKUP_SYMBOLS) {
      const importRegex = new RegExp(
        String.raw`\bcrate\s*::\s*daemon\s*::\s*handle\s*::\s*(?:\{[^}]*\b${symbol}\b[^}]*\}|${symbol}\b)`,
        "gu",
      );
      for (let match = importRegex.exec(contents); match; match = importRegex.exec(contents)) {
        violations.push({
          kind: "daemon_handle_store_lookup_ownership",
          line: lineForOffset(contents, match.index),
          path: relativePath,
          message: `${symbol} must be imported from crate::daemon/state-owned lookup exports, not crate::daemon::handle.`,
        });
      }
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
    ...checkCtxHttpCliOnlyServiceUsage(rootDir),
    ...checkDaemonRootRouteFacades(rootDir),
    ...checkDaemonHandleStoreLookupOwnership(rootDir),
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
  COLLAPSED_DIRECTORIES,
  CTX_HTTP_CLI_ONLY_SERVICE_DEPS,
  CTX_HTTP_FORBIDDEN_DOMAIN_SERVICE_DEPS,
  DAEMON_ROOT_ROUTE_FACADE_TARGETS,
  DAEMON_ROOT_WEB_SESSION_TRANSPORT_FACADE_DENY,
  DAEMON_HANDLE_STORE_LOOKUP_HOME,
  DAEMON_HANDLE_STORE_LOOKUP_SYMBOLS,
  HEAD_PROJECTION_FORBIDDEN_IMPORT_PATTERNS,
  MESSAGE_SERVICE_FORBIDDEN_DEPS,
  PACKAGE_SHAPE_BOUNDARY_CRATES,
  PACKAGE_SHAPE_FORBIDDEN_BACKEDGE_DEPS,
  RATCHETED_FILE_LIMITS,
  ROUTE_CONTRACTS_ALLOWED_CTX_DEPS,
  REPO_ONBOARDING_SERVICE_FORBIDDEN_DEPS,
  SERVICE_RUNTIME_FORBIDDEN_DEPS,
  SESSION_RUNTIME_FORBIDDEN_DEPS,
  SESSION_VCS_SERVICE_FORBIDDEN_DEPS,
  TITLE_SERVICE_FORBIDDEN_DEPS,
  WORKSPACE_ATTACHMENTS_FORBIDDEN_DEPS,
  WORKSPACE_SERVICES_FORBIDDEN_DEPS,
  WORKTREE_BOOTSTRAP_SERVICE_FORBIDDEN_DEPS,
  WORKTREE_VCS_SERVICE_FORBIDDEN_DEPS,
  checkCargoDependencyDirection,
  checkCollapsedPaths,
  checkCtxHttpCliOnlyServiceUsage,
  checkDaemonRootRouteFacades,
  checkDaemonHandleStoreLookupOwnership,
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
