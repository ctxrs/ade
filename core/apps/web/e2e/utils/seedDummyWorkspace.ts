import { execSync } from "child_process";
import { mkdtempSync, writeFileSync } from "fs";
import { tmpdir } from "os";
import path from "path";
import type { APIRequestContext } from "playwright/test";

type NumberRange = { min: number; max: number };

type SeedOptions = {
  tasks: number;
  sessionsPerTask: number | NumberRange;
  turnsPerSession: number;
  workspaceName?: string;
  repoRoot?: string;
  throttleMs?: number;
  createDefaultSession?: boolean;
  messageBytes?: number | NumberRange;
  messagePrefix?: string;
  includeToolSummaries?: boolean;
  toolSummariesPerTurn?: number;
  toolSummaryFixtures?: Array<{
    kind: string;
    title?: string;
    input?: any;
    output_text?: string;
  }>;
  awaitTurnCompletion?: boolean;
  completionTimeoutMs?: number;
};

type SeedResult = {
  workspaceId: string;
  taskIds: string[];
  sessionIdsByTask: Record<string, string[]>;
};

type StreamOptions = {
  sessionIds: string[];
  intervalMs?: number;
  durationMs?: number;
  messageBytes?: number | NumberRange;
  messagePrefix?: string;
  includeToolSummaries?: boolean;
  toolSummariesPerTurn?: number;
  toolSummaryFixtures?: Array<{
    kind: string;
    title?: string;
    input?: any;
    output_text?: string;
  }>;
};

const parseCount = (value: number | NumberRange, index: number): number => {
  if (typeof value === "number") return value;
  const span = Math.max(0, value.max - value.min);
  return value.min + (index % (span + 1));
};

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const DEFAULT_TOOL_FIXTURES = [
  { kind: "execute", title: "Run pwd", input: { command: "pwd" } },
  { kind: "search", title: "Searched context", input: { query: "context" } },
  { kind: "execute", title: "Explored .ctx", input: { command: "ls .ctx" } },
  { kind: "read", title: "Read .ctx", input: { path: ".ctx" } },
  {
    kind: "execute",
    title: "Explored specs",
    input: { command: "ls .ctx/ctx-pack/specs" },
  },
  {
    kind: "execute",
    title: "Run ./scripts/supercat.sh .ctx/ctx-pack/specs",
    input: { command: "./scripts/supercat.sh .ctx/ctx-pack/specs" },
  },
  { kind: "search", title: "Searched workbench", input: { query: "workbench" } },
  { kind: "read", title: "Read ctx-pack", input: { path: ".ctx/ctx-pack" } },
];

const chunkFixtures = (fixtures: SeedOptions["toolSummaryFixtures"], count: number, offset: number) => {
  const list = (fixtures && fixtures.length > 0 ? fixtures : DEFAULT_TOOL_FIXTURES).slice();
  if (list.length === 0 || count <= 0) return [];
  const out = [];
  for (let i = 0; i < count; i++) {
    out.push(list[(offset + i) % list.length]);
  }
  return out;
};

const buildToolMarker = (fixtures: SeedOptions["toolSummaryFixtures"]) => {
  if (!fixtures || fixtures.length === 0) return "";
  return `\n[[tool_calls]]\n${JSON.stringify(fixtures)}\n[[/tool_calls]]`;
};

const buildPaddedMessage = (base: string, targetBytes?: number): string => {
  if (!targetBytes || targetBytes <= base.length) return base;
  const padding = targetBytes - base.length;
  if (padding === 1) return `${base} `;
  return `${base} ${"x".repeat(padding - 1)}`;
};

function initRepo(): string {
  const repo = mkdtempSync(path.join(tmpdir(), "ctx-e2e-fixture-"));
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

async function apiGet<T>(request: APIRequestContext, url: string): Promise<T> {
  const resp = await request.get(url);
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
  const createDefaultSession = opts.createDefaultSession ?? true;
  const includeToolSummaries = Boolean(opts.includeToolSummaries);
  const toolSummariesPerTurn = opts.toolSummariesPerTurn ?? 6;
  const toolSummaryFixtures = opts.toolSummaryFixtures ?? DEFAULT_TOOL_FIXTURES;
  const messagePrefix = opts.messagePrefix ?? "fixture msg";
  const messageBytes = opts.messageBytes;
  const awaitTurnCompletion = Boolean(opts.awaitTurnCompletion);
  const completionTimeoutMs = opts.completionTimeoutMs ?? 15_000;

  for (let i = 0; i < opts.tasks; i++) {
    const task = await apiPost<{ id: string }>(request, `/api/workspaces/${workspace.id}/tasks`, {
      title: `fixture task ${i + 1}`,
      create_default_session: createDefaultSession,
    });
    taskIds.push(task.id);
    sessionIdsByTask[task.id] = [];

    const sessionCount = parseCount(opts.sessionsPerTask, i);
    for (let s = 0; s < sessionCount; s++) {
      const session = await apiPost<{ id: string }>(request, `/api/tasks/${task.id}/sessions`, {
        provider_id: "fake",
        model_id: "fake-model",
      });
      sessionIdsByTask[task.id].push(session.id);

      for (let t = 0; t < opts.turnsPerSession; t++) {
        const toolFixtures = includeToolSummaries
          ? chunkFixtures(toolSummaryFixtures, toolSummariesPerTurn, t * toolSummariesPerTurn)
          : [];
        const toolMarker = includeToolSummaries ? buildToolMarker(toolFixtures) : "";
        const baseMessage = `${messagePrefix} ${i + 1}.${s + 1}.${t + 1}`;
        const paddedMessage = buildPaddedMessage(
          baseMessage,
          messageBytes ? parseCount(messageBytes, t) : undefined,
        );
        await apiPost(request, `/api/sessions/${session.id}/messages`, {
          content: `${paddedMessage}${toolMarker}`,
          delivery: "immediate",
        });
        if (awaitTurnCompletion) {
          const start = Date.now();
          while (true) {
            const snapshot = await apiGet<{
              head: {
                turns: Array<{ status: string; tool_total?: number | null }>;
                tool_summaries?: any[];
              };
            }>(request, `/api/sessions/${session.id}/snapshot?limit=1`);
            const head = snapshot?.head;
            const turns = Array.isArray(head?.turns) ? head.turns : [];
            if (turns.length === 0) {
              if (Date.now() - start > completionTimeoutMs) {
                throw new Error(`turn completion timeout for session ${session.id}`);
              }
              await sleep(50);
              continue;
            }
            const last = turns[turns.length - 1];
            const done = last?.status === "completed" || last?.status === "done";
            if (done) break;
            if (Date.now() - start > completionTimeoutMs) {
              throw new Error(`turn completion timeout for session ${session.id}`);
            }
            await sleep(50);
          }
        }
        if (throttle > 0) {
          await sleep(throttle);
        }
      }
    }
  }

  return { workspaceId: workspace.id, taskIds, sessionIdsByTask };
}

export function startStreamingMessages(
  request: APIRequestContext,
  opts: StreamOptions,
): { stop: () => Promise<void> } {
  const intervalMs = opts.intervalMs ?? 250;
  const messagePrefix = opts.messagePrefix ?? "stream msg";
  const includeToolSummaries = Boolean(opts.includeToolSummaries);
  const toolSummariesPerTurn = opts.toolSummariesPerTurn ?? 3;
  const toolSummaryFixtures = opts.toolSummaryFixtures ?? DEFAULT_TOOL_FIXTURES;
  const messageBytes = opts.messageBytes;
  const sessionIds = opts.sessionIds;

  if (!sessionIds || sessionIds.length === 0) {
    throw new Error("startStreamingMessages requires at least one session id");
  }

  let stopped = false;
  let tick = 0;
  let inflight: Promise<void> = Promise.resolve();

  const sendOnce = async () => {
    if (stopped) return;
    const sessionId = sessionIds[tick % sessionIds.length];
    const toolFixtures = includeToolSummaries
      ? chunkFixtures(toolSummaryFixtures, toolSummariesPerTurn, tick * toolSummariesPerTurn)
      : [];
    const toolMarker = includeToolSummaries ? buildToolMarker(toolFixtures) : "";
    const baseMessage = `${messagePrefix} ${tick + 1}`;
    const paddedMessage = buildPaddedMessage(
      baseMessage,
      messageBytes ? parseCount(messageBytes, tick) : undefined,
    );
    tick += 1;
    await apiPost(request, `/api/sessions/${sessionId}/messages`, {
      content: `${paddedMessage}${toolMarker}`,
      delivery: "immediate",
    });
  };

  const timer = setInterval(() => {
    inflight = inflight.then(sendOnce).catch(() => {
      // Swallow to keep the interval alive for the test harness.
    });
  }, intervalMs);

  const stop = async () => {
    if (stopped) return;
    stopped = true;
    clearInterval(timer);
    await inflight;
  };

  if (opts.durationMs && opts.durationMs > 0) {
    setTimeout(() => {
      stop().catch(() => {
        // ignore
      });
    }, opts.durationMs);
  }

  return { stop };
}
