import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import type { APIRequestContext } from "playwright/test";

type SeedOptions = {
  tasks: number;
  sessionsPerTask: number | { min: number; max: number };
  turnsPerSession: number;
  workspaceName?: string;
  repoRoot?: string;
  throttleMs?: number;
};

type SeedResult = {
  workspaceId: string;
  taskIds: string[];
  sessionIdsByTask: Record<string, string[]>;
};

const parseCount = (value: number | { min: number; max: number }, index: number): number => {
  if (typeof value === "number") return value;
  const span = Math.max(0, value.max - value.min);
  return value.min + (index % (span + 1));
};

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function initRepo(): string {
  const repo = mkdtempSync(path.join(tmpdir(), "context-e2e-fixture-"));
  execSync("git init", { cwd: repo });
  execSync("git config user.email test@example.com", { cwd: repo });
  execSync("git config user.name Test", { cwd: repo });
  writeFileSync(path.join(repo, "README.md"), "fixture\n");
  execSync("git add .", { cwd: repo });
  execSync("git commit -m init", { cwd: repo });
  return repo;
}

async function apiPost<T>(request: APIRequestContext, url: string, data: any): Promise<T> {
  const resp = await request.post(url, { data });
  if (!resp.ok()) {
    throw new Error(`seed request failed: ${url} (${resp.status()})`);
  }
  return (await resp.json()) as T;
}

export async function seedDummyWorkspace(
  request: APIRequestContext,
  opts: SeedOptions,
): Promise<SeedResult> {
  const repoRoot = opts.repoRoot ?? initRepo();
  const workspaceName = opts.workspaceName ?? `ws-fixture-${Date.now()}`;
  const workspace = await apiPost<{ id: string }>(request, "/api/workspaces", {
    root_path: repoRoot,
    name: workspaceName,
  });

  const taskIds: string[] = [];
  const sessionIdsByTask: Record<string, string[]> = {};
  const throttle = opts.throttleMs ?? 15;

  for (let i = 0; i < opts.tasks; i++) {
    const task = await apiPost<{ id: string }>(request, `/api/workspaces/${workspace.id}/tasks`, {
      title: `fixture task ${i + 1}`,
    });
    taskIds.push(task.id);
    sessionIdsByTask[task.id] = [];

    const sessionCount = parseCount(opts.sessionsPerTask, i);
    for (let s = 0; s < sessionCount; s++) {
      const track = await apiPost<{ id: string }>(request, `/api/tasks/${task.id}/tracks`, {
        label: `fixture track ${s + 1}`,
      });
      const session = await apiPost<{ id: string }>(request, `/api/tracks/${track.id}/sessions`, {
        provider_id: "fake",
        model_id: "fake-model",
      });
      sessionIdsByTask[task.id].push(session.id);

      for (let t = 0; t < opts.turnsPerSession; t++) {
        await apiPost(request, `/api/sessions/${session.id}/messages`, {
          content: `fixture msg ${i + 1}.${s + 1}.${t + 1}`,
          delivery: "immediate",
        });
        if (throttle > 0) {
          await sleep(throttle);
        }
      }
    }
  }

  return { workspaceId: workspace.id, taskIds, sessionIdsByTask };
}
