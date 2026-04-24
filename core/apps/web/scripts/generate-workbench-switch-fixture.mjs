#!/usr/bin/env node

import { mkdir, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const DEFAULT_OUT = path.join(os.tmpdir(), "ctx-workbench-switch-fixture.json");
const WORKSPACE_ID = "ws-switch-fixture";
const BASE_TIME_MS = Date.parse("2025-01-01T00:00:00Z");

function usage() {
  console.error(
    "Usage: node ./scripts/generate-workbench-switch-fixture.mjs " +
      "[--out path] [--task-count n] [--turns-per-session n] " +
      "[--content-profile plain|markdown|code|mixed] [--message-bytes n] [--running-every n]",
  );
  process.exit(1);
}

function parseArgs(argv) {
  const parsed = {
    out: DEFAULT_OUT,
    taskCount: 20,
    turnsPerSession: 24,
    contentProfile: "mixed",
    messageBytes: 768,
    runningEvery: 4,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--out") {
      parsed.out = String(argv[i + 1] ?? parsed.out);
      i += 1;
      continue;
    }
    if (arg === "--task-count") {
      parsed.taskCount = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--turns-per-session") {
      parsed.turnsPerSession = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--content-profile") {
      parsed.contentProfile = String(argv[i + 1] ?? parsed.contentProfile);
      i += 1;
      continue;
    }
    if (arg === "--message-bytes") {
      parsed.messageBytes = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--running-every") {
      parsed.runningEvery = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      usage();
    }
    usage();
  }

  if (!Number.isInteger(parsed.taskCount) || parsed.taskCount < 1 || parsed.taskCount > 100) {
    throw new Error("--task-count must be an integer from 1 to 100.");
  }
  if (!Number.isInteger(parsed.turnsPerSession) || parsed.turnsPerSession < 1 || parsed.turnsPerSession > 500) {
    throw new Error("--turns-per-session must be an integer from 1 to 500.");
  }
  if (!["plain", "markdown", "code", "mixed"].includes(parsed.contentProfile)) {
    throw new Error("--content-profile must be one of plain, markdown, code, or mixed.");
  }
  if (!Number.isInteger(parsed.messageBytes) || parsed.messageBytes < 0 || parsed.messageBytes > 20000) {
    throw new Error("--message-bytes must be an integer from 0 to 20000.");
  }
  if (!Number.isInteger(parsed.runningEvery) || parsed.runningEvery < 0 || parsed.runningEvery > 100) {
    throw new Error("--running-every must be an integer from 0 to 100.");
  }

  return parsed;
}

function iso(offsetMs) {
  return new Date(BASE_TIME_MS + offsetMs).toISOString();
}

function paddedNumber(value, width = 2) {
  return String(value).padStart(width, "0");
}

function contentKind(profile, taskIndex, turnIndex) {
  if (profile !== "mixed") return profile;
  const kinds = ["plain", "markdown", "code"];
  return kinds[(taskIndex + turnIndex) % kinds.length];
}

function repeatToMinimum(content, minBytes) {
  if (!Number.isFinite(minBytes) || minBytes <= 0 || content.length >= minBytes) return content;
  const filler = "\n\nAdditional deterministic load paragraph for switch replay measurement.";
  let next = content;
  while (next.length < minBytes) {
    next += filler;
  }
  return next;
}

function assistantContent({ profile, taskIndex, turnIndex, minBytes }) {
  const taskLabel = paddedNumber(taskIndex + 1);
  const turnLabel = paddedNumber(turnIndex + 1, 3);
  const kind = contentKind(profile, taskIndex, turnIndex);
  if (kind === "markdown") {
    return repeatToMinimum(
      [
        `### Task ${taskLabel} Turn ${turnLabel}`,
        "",
        `This assistant response exercises markdown parsing for task ${taskLabel}.`,
        "",
        "- deterministic bullet one",
        "- deterministic bullet two with **bold** and `inline code`",
        "- deterministic bullet three with a [local label](https://example.invalid)",
        "",
        "| Column | Value |",
        "| --- | ---: |",
        `| task | ${taskLabel} |`,
        `| turn | ${turnLabel} |`,
      ].join("\n"),
      minBytes,
    );
  }
  if (kind === "code") {
    return repeatToMinimum(
      [
        `Code-heavy response for task ${taskLabel}, turn ${turnLabel}.`,
        "",
        "```ts",
        "type ReplayMetric = {",
        "  sessionId: string;",
        "  visibleMs: number;",
        "  stableMs: number;",
        "};",
        "",
        "export function summarizeReplayMetric(input: ReplayMetric[]): number {",
        "  return input.reduce((sum, item) => sum + item.visibleMs + item.stableMs, 0);",
        "}",
        "```",
      ].join("\n"),
      minBytes,
    );
  }
  return repeatToMinimum(
    `Plain deterministic assistant reply for task ${taskLabel}, turn ${turnLabel}. ` +
      "This text is intentionally stable so replay diffs are predictable.",
    minBytes,
  );
}

function createSession({ taskIndex, turnsPerSession, contentProfile, messageBytes, running }) {
  const taskNumber = taskIndex + 1;
  const label = paddedNumber(taskNumber);
  const taskId = `task-${label}`;
  const sessionId = `session-${label}`;
  const worktreeId = `wt-${label}`;
  const createdAt = iso(taskIndex * 60_000);
  const updatedAt = iso(taskIndex * 60_000 + turnsPerSession * 5_000);
  const sessionStatus = running ? "active" : "completed";
  const taskStatus = running ? "running" : "completed";
  const session = {
    id: sessionId,
    task_id: taskId,
    workspace_id: WORKSPACE_ID,
    worktree_id: worktreeId,
    provider_id: "fake",
    model_id: "fake-model",
    title: `Switch fixture session ${label}`,
    agent_role: "assistant",
    status: sessionStatus,
    created_at: createdAt,
    updated_at: updatedAt,
  };
  const turns = [];
  const messages = [];
  let seq = 0;

  for (let turnIndex = 0; turnIndex < turnsPerSession; turnIndex += 1) {
    const turnNumber = turnIndex + 1;
    const turnLabel = paddedNumber(turnNumber, 3);
    const turnId = `turn-${label}-${turnLabel}`;
    const userSeq = seq + 1;
    const assistantSeq = seq + 2;
    const startedAt = iso(taskIndex * 60_000 + turnIndex * 5_000);
    const completedAt = iso(taskIndex * 60_000 + turnIndex * 5_000 + 2_000);
    turns.push({
      turn_id: turnId,
      session_id: sessionId,
      user_message_id: `msg-${label}-${turnLabel}-user`,
      status: "completed",
      start_seq: userSeq,
      end_seq: assistantSeq,
      started_at: startedAt,
      updated_at: completedAt,
      assistant_partial: null,
      thought_partial: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    });
    messages.push({
      id: `msg-${label}-${turnLabel}-user`,
      session_id: sessionId,
      task_id: taskId,
      turn_id: turnId,
      role: "user",
      content: `User prompt for task ${label}, turn ${turnLabel}.`,
      delivery: "immediate",
      created_at: startedAt,
      order_seq: userSeq,
      turn_sequence: turnNumber,
    });
    messages.push({
      id: `msg-${label}-${turnLabel}-assistant`,
      session_id: sessionId,
      task_id: taskId,
      turn_id: turnId,
      role: "assistant",
      content: assistantContent({
        profile: contentProfile,
        taskIndex,
        turnIndex,
        minBytes: messageBytes,
      }),
      delivery: "immediate",
      created_at: completedAt,
      order_seq: assistantSeq,
      turn_sequence: turnNumber,
    });
    seq = assistantSeq;
  }

  const head = {
    session,
    turns,
    messages,
    events: [],
    last_event_seq: seq,
    state_rev: seq,
    projection_rev: seq,
    has_more_turns: false,
  };
  const summary = {
    session,
    last_message_at: updatedAt,
    last_message_preview: String(messages[messages.length - 1]?.content ?? "").slice(0, 240),
    last_event_seq: seq,
    state_rev: seq,
    projection_rev: seq,
    activity: {
      is_working: running,
      last_turn_status: running ? "running" : "completed",
    },
    unread: false,
  };
  const task = {
    id: taskId,
    workspace_id: WORKSPACE_ID,
    title: `switch fixture task ${label}`,
    description: null,
    status: taskStatus,
    primary_session_id: sessionId,
    primary_worktree_id: worktreeId,
    created_at: createdAt,
    updated_at: updatedAt,
    archived_at: null,
    assistant_seen_at: updatedAt,
    last_activity_at: updatedAt,
    last_assistant_message_at: updatedAt,
    has_active_session: running,
  };

  return {
    taskEntry: {
      task,
      primary_session: summary,
      primary_session_head: head,
      sessions: [summary],
      sort_at: iso((1000 - taskIndex) * 60_000),
    },
    snapshot: {
      summary,
      head,
      state: {
        artifacts: [],
        git_status: null,
      },
    },
  };
}

function buildFixture(args) {
  const taskEntries = [];
  const sessionSnapshots = {};
  for (let taskIndex = 0; taskIndex < args.taskCount; taskIndex += 1) {
    const running = args.runningEvery > 0 && taskIndex % args.runningEvery === 0;
    const { taskEntry, snapshot } = createSession({
      taskIndex,
      turnsPerSession: args.turnsPerSession,
      contentProfile: args.contentProfile,
      messageBytes: args.messageBytes,
      running,
    });
    taskEntries.push(taskEntry);
    sessionSnapshots[snapshot.summary.session.id] = snapshot;
  }

  return {
    workspace: {
      id: WORKSPACE_ID,
      name: "Switch Fixture Workspace",
      root_path: "/tmp/ctx-switch-fixture",
      created_at: iso(0),
    },
    health: {
      version: "0.0.0-fixture",
      pid: 0,
      data_root: "/tmp/ctx-switch-fixture-data",
      daemon_url: "http://127.0.0.1:5173",
      auth_required: false,
    },
    providers: [
      {
        provider_id: "fake",
        installed: true,
        health: "ok",
        diagnostics: [],
      },
    ],
    active_snapshot: {
      workspace_id: WORKSPACE_ID,
      snapshot_rev: 1,
      archived_rev: 0,
      active: {
        total_count: taskEntries.length,
        tasks: taskEntries,
      },
    },
    session_snapshots: sessionSnapshots,
    stream: [
      {
        delay_ms: 0,
        event: {
          type: "ready",
          workspace_id: WORKSPACE_ID,
          snapshot_rev: 1,
          archived_rev: 0,
        },
      },
    ],
  };
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const outPath = path.resolve(args.out);
  await mkdir(path.dirname(outPath), { recursive: true });
  await writeFile(outPath, `${JSON.stringify(buildFixture(args), null, 2)}\n`);
  console.log(`wrote ${outPath}`);
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
});
