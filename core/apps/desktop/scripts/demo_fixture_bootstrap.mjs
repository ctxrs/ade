#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { execFileSync } from "node:child_process";
import { api, readDaemonAuth } from "./demo_lib.mjs";

function parseArgs(argv) {
  const out = {
    scenarioPath: null,
    workspaceRoot: null,
    daemonUrl: process.env.CTX_DAEMON_URL || "",
    authToken: process.env.CTX_AUTH_TOKEN || "",
    dataDir: process.env.CTX_DATA_ROOT || "",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = argv[i + 1];
    if (arg === "--scenario") {
      out.scenarioPath = resolve(next);
      i += 1;
    } else if (arg === "--workspace-root") {
      out.workspaceRoot = resolve(next);
      i += 1;
    } else if (arg === "--daemon-url") {
      out.daemonUrl = next;
      i += 1;
    } else if (arg === "--auth-token") {
      out.authToken = next;
      i += 1;
    } else if (arg === "--data-dir") {
      out.dataDir = resolve(next);
      i += 1;
    } else if (arg === "--help") {
      printHelp();
      process.exit(0);
    }
  }
  if (!out.scenarioPath) {
    throw new Error("--scenario is required");
  }
  if (!out.workspaceRoot) {
    throw new Error("--workspace-root is required");
  }
  return out;
}

function printHelp() {
  process.stdout.write(`demo_fixture_bootstrap

Usage:
  node core/apps/desktop/scripts/demo_fixture_bootstrap.mjs \\
    --scenario core/apps/desktop/automation/fixtures/demo-ping-pong-fixture.json \\
    --workspace-root /tmp/ctx-demo-ping-pong

Options:
  --scenario <file>
  --workspace-root <dir>
  --daemon-url <url>
  --auth-token <token>
  --data-dir <dir>
`);
}

function ensureWorkspaceRoot(root, scenario) {
  mkdirSync(root, { recursive: true });
  const readmePath = join(root, "README.md");
  if (!existsSync(readmePath)) {
    writeFileSync(readmePath, `# ${scenario.workspace_name}\n`, "utf8");
  }
  if (!existsSync(join(root, ".git"))) {
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["config", "user.email", "demo@example.com"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["config", "user.name", "Demo"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["add", "README.md"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["commit", "-m", "init"], { cwd: root, stdio: "ignore" });
  }
}

async function findWorkspace(baseUrl, token, rootPath) {
  const workspaces = await api(baseUrl, token, "GET", "/api/workspaces");
  return workspaces.find((workspace) => workspace.root_path === rootPath) || null;
}

export async function bootstrapDemoFixture(options) {
  const scenario = JSON.parse(readFileSync(options.scenarioPath, "utf8"));
  ensureWorkspaceRoot(options.workspaceRoot, scenario);
  const { daemonUrl, authToken } = readDaemonAuth(options);

  let workspace = await findWorkspace(daemonUrl, authToken, options.workspaceRoot);
  if (!workspace) {
    workspace = await api(daemonUrl, authToken, "POST", "/api/workspaces", {
      root_path: options.workspaceRoot,
      name: scenario.workspace_name,
    });
  }

  const task = await api(
    daemonUrl,
    authToken,
    "POST",
    `/api/workspaces/${workspace.id}/tasks`,
    {
      title: scenario.task_title,
      create_default_session: false,
    },
  );

  const session = await api(
    daemonUrl,
    authToken,
    "POST",
    `/api/tasks/${task.id}/sessions`,
    {
      provider_id: scenario.provider_id,
      model_id: scenario.model_id,
    },
  );

  const seeded = await api(
    daemonUrl,
    authToken,
    "POST",
    `/api/dev/sessions/${session.id}/seed_transcript`,
    {
      session_title: scenario.session_title,
      task_title: scenario.task_title,
      turns: scenario.prior_turns,
    },
  );

  return {
    scenario_id: scenario.id,
    workspace_id: workspace.id,
    task_id: task.id,
    session_id: session.id,
    next_prompt: scenario.next_prompt,
    seeded,
  };
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const result = await bootstrapDemoFixture(options);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (import.meta.url === `file://${process.argv[1]}`) {
  main().catch((error) => {
    process.stderr.write(`${String(error?.stack || error)}\n`);
    process.exit(1);
  });
}
