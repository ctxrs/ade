#!/usr/bin/env node
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, resolve, join } from "node:path";
import { execFileSync } from "node:child_process";
import { DatabaseSync } from "node:sqlite";
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

export function resolveWorkspaceTemplateDir(scenarioPath, scenario) {
  const raw = String(scenario.workspace_template_dir || "").trim();
  if (!raw) return null;
  return resolve(dirname(scenarioPath), raw);
}

export function materializeWorkspaceTemplate(root, templateDir) {
  rmSync(root, { recursive: true, force: true });
  mkdirSync(root, { recursive: true });
  cpSync(templateDir, root, {
    recursive: true,
    force: true,
  });
}

export function ensureWorkspaceRoot(root, scenario, scenarioPath) {
  const templateDir = resolveWorkspaceTemplateDir(scenarioPath, scenario);
  if (templateDir) {
    materializeWorkspaceTemplate(root, templateDir);
  } else {
    mkdirSync(root, { recursive: true });
  }
  const readmePath = join(root, "README.md");
  if (!existsSync(readmePath)) {
    writeFileSync(readmePath, `# ${scenario.workspace_name}\n`, "utf8");
  }
  if (!existsSync(join(root, ".git"))) {
    execFileSync("git", ["init"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["config", "user.email", "demo@example.com"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["config", "user.name", "Demo"], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["add", "."], { cwd: root, stdio: "ignore" });
    execFileSync("git", ["commit", "-m", scenario.workspace_commit_message || "init"], { cwd: root, stdio: "ignore" });
  }
}

export function resolveWorkspaceDbPath(dataDir, workspaceId) {
  return resolve(dataDir, "db", "workspaces", workspaceId, "db.sqlite");
}

export function normalizeDemoTaskStatus(value) {
  const raw = String(value || "idle").trim().toLowerCase();
  if (raw === "working" || raw === "new_message" || raw === "unread" || raw === "idle") {
    return raw === "unread" ? "new_message" : raw;
  }
  throw new Error(`unsupported demo task status: ${value}`);
}

export function isoMinutesAgo(minutesAgo, baseNow = new Date()) {
  const numericMinutes = Number(minutesAgo);
  if (!Number.isFinite(numericMinutes) || numericMinutes < 0) {
    throw new Error(`minutes_ago must be a non-negative number; received ${minutesAgo}`);
  }
  return new Date(baseNow.getTime() - numericMinutes * 60_000).toISOString();
}

function buildSeedTurns(seed, index) {
  if (Array.isArray(seed.turns) && seed.turns.length > 0) {
    return seed.turns;
  }
  const title = String(seed.title || `Demo task ${index + 1}`).trim();
  return [
    {
      user: `Please take care of: ${title}.`,
      assistant: `I’m on it. I’ll update you once ${title.toLowerCase()} is in better shape.`,
    },
  ];
}

function normalizeActiveTaskSeed(seed, index) {
  if (!seed || typeof seed !== "object") {
    throw new Error(`active_tasks[${index}] must be an object`);
  }
  const title = String(seed.title || "").trim();
  if (!title) {
    throw new Error(`active_tasks[${index}].title is required`);
  }
  const providerId = String(seed.provider_id || "").trim();
  if (!providerId) {
    throw new Error(`active_tasks[${index}].provider_id is required`);
  }
  const modelId = String(seed.model_id || "").trim();
  if (!modelId) {
    throw new Error(`active_tasks[${index}].model_id is required`);
  }
  return {
    title,
    session_title: String(seed.session_title || title).trim(),
    provider_id: providerId,
    model_id: modelId,
    minutes_ago: Number(seed.minutes_ago ?? 0),
    status: normalizeDemoTaskStatus(seed.status),
    turns: buildSeedTurns(seed, index),
  };
}

function applySeededTaskPresentationState({ dataDir, workspaceId, taskId, sessionId, minutesAgo, status }) {
  if (!dataDir) {
    throw new Error("active_tasks requires a daemon data dir for state patching");
  }
  const iso = isoMinutesAgo(minutesAgo);
  const dbPath = resolveWorkspaceDbPath(dataDir, workspaceId);
  const db = new DatabaseSync(dbPath);
  try {
    db.exec("PRAGMA busy_timeout = 5000");
    db.prepare(
      `UPDATE tasks
         SET updated_at = ?,
             last_activity_at = ?,
             last_assistant_message_at = ?
       WHERE id = ?`,
    ).run(iso, iso, iso, taskId);
    db.prepare(
      `UPDATE sessions
         SET updated_at = ?
       WHERE id = ?`,
    ).run(iso, sessionId);
    db.prepare(
      `UPDATE session_snapshot_summaries
         SET last_message_at = ?,
             updated_at = ?,
             last_turn_status = ?,
             running_turn_count = ?
       WHERE session_id = ?`,
    ).run(iso, iso, status === "working" ? "running" : "completed", status === "working" ? 1 : 0, sessionId);
  } finally {
    db.close();
  }
}

async function seedActiveTasks(baseUrl, token, workspace, scenario, options) {
  const rawSeeds = Array.isArray(scenario.active_tasks) ? scenario.active_tasks : [];
  if (rawSeeds.length === 0) {
    return [];
  }

  const seeded = [];
  for (const [index, rawSeed] of rawSeeds.entries()) {
    const seed = normalizeActiveTaskSeed(rawSeed, index);
    const task = await api(
      baseUrl,
      token,
      "POST",
      `/api/workspaces/${workspace.id}/tasks`,
      {
        title: seed.title,
        create_default_session: false,
      },
    );
    const session = await api(
      baseUrl,
      token,
      "POST",
      `/api/tasks/${task.id}/sessions`,
      {
        provider_id: seed.provider_id,
        model_id: seed.model_id,
      },
    );
    const transcript = await api(
      baseUrl,
      token,
      "POST",
      `/api/dev/sessions/${session.id}/seed_transcript`,
      {
        session_title: seed.session_title,
        task_title: seed.title,
        turns: seed.turns,
      },
    );
    applySeededTaskPresentationState({
      dataDir: options.dataDir,
      workspaceId: workspace.id,
      taskId: task.id,
      sessionId: session.id,
      minutesAgo: seed.minutes_ago,
      status: seed.status,
    });
    const refreshedTask = await api(
      baseUrl,
      token,
      "POST",
      `/api/tasks/${task.id}/${seed.status === "new_message" ? "mark_unread" : "mark_read"}`,
    );
    seeded.push({
      index,
      status: seed.status,
      title: seed.title,
      task_id: task.id,
      session_id: session.id,
      transcript,
      refreshed_task: refreshedTask,
    });
  }
  return seeded;
}

async function configureWorkspaceExecution(baseUrl, token, workspaceId, scenario) {
  const environment = String(scenario.execution_environment || "").trim();
  if (!environment) return null;
  return api(baseUrl, token, "POST", `/api/workspaces/${workspaceId}/execution_config`, {
    environment,
    network_mode: scenario.network_mode ?? null,
    allowlist: scenario.allowlist ?? null,
  });
}

async function findWorkspace(baseUrl, token, rootPath) {
  const workspaces = await api(baseUrl, token, "GET", "/api/workspaces");
  return workspaces.find((workspace) => workspace.root_path === rootPath) || null;
}

export async function bootstrapDemoFixture(options) {
  const scenario = JSON.parse(readFileSync(options.scenarioPath, "utf8"));
  ensureWorkspaceRoot(options.workspaceRoot, scenario, options.scenarioPath);
  const { daemonUrl, authToken } = readDaemonAuth(options);

  let workspace = await findWorkspace(daemonUrl, authToken, options.workspaceRoot);
  if (!workspace) {
    workspace = await api(daemonUrl, authToken, "POST", "/api/workspaces", {
      root_path: options.workspaceRoot,
      name: scenario.workspace_name,
    });
  }

  const execution_config = await configureWorkspaceExecution(daemonUrl, authToken, workspace.id, scenario);
  const seeded_active_tasks = await seedActiveTasks(daemonUrl, authToken, workspace, scenario, options);
  const bootstrapMode = String(scenario.bootstrap_mode || "seeded_session").trim();
  if (bootstrapMode === "workspace_only") {
    return {
      scenario_id: scenario.id,
      workspace_id: workspace.id,
      task_id: null,
      session_id: null,
      next_prompt: scenario.next_prompt,
      seeded: null,
      seeded_active_tasks,
      execution_config,
    };
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
    seeded_active_tasks,
    execution_config,
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
