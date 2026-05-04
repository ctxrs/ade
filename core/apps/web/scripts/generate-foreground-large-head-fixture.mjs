#!/usr/bin/env node

import { mkdir, readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(__dirname, "..");
const defaultBaseFixture = path.resolve(appRoot, "e2e/fixtures/workbench-replay-fixture.json");
const defaultOut = path.join(os.tmpdir(), "ctx-foreground-large-head-fixture.json");
const baseTimeMs = Date.parse("2026-05-03T00:00:00Z");
const workspaceId = "ws-foreground-large-head";
const foregroundSessionId = "session-a";
const foregroundTaskId = "task-a";
const foregroundTurnId = "turn-foreground-large-head";
const finalContent = "Foreground large-head recovered latest message";

function usage() {
  console.error(
    "Usage: node ./scripts/generate-foreground-large-head-fixture.mjs " +
      "[--out path] [--variant legacy|bounded] [--message-count n] [--message-bytes n] " +
      "[--tool-summaries n] [--bounded-tool-summaries n] [--tool-preview-bytes n] " +
      "[--gap-count n] [--first-gap-delay-ms ms] [--gap-interval-ms ms]",
  );
  process.exit(1);
}

function parseArgs(argv) {
  const parsed = {
    out: defaultOut,
    variant: "bounded",
    messageCount: 164,
    messageBytes: 260,
    toolSummaries: 335,
    boundedToolSummaries: 96,
    toolPreviewBytes: 850,
    gapCount: 10,
    firstGapDelayMs: 4000,
    gapIntervalMs: 1000,
    baseFixture: defaultBaseFixture,
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--out") {
      parsed.out = String(argv[i + 1] ?? parsed.out);
      i += 1;
      continue;
    }
    if (arg === "--variant") {
      parsed.variant = String(argv[i + 1] ?? parsed.variant);
      i += 1;
      continue;
    }
    if (arg === "--message-count") {
      parsed.messageCount = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--message-bytes") {
      parsed.messageBytes = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--tool-summaries") {
      parsed.toolSummaries = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--bounded-tool-summaries") {
      parsed.boundedToolSummaries = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--tool-preview-bytes") {
      parsed.toolPreviewBytes = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--gap-count") {
      parsed.gapCount = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--first-gap-delay-ms") {
      parsed.firstGapDelayMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--gap-interval-ms") {
      parsed.gapIntervalMs = Number(argv[i + 1]);
      i += 1;
      continue;
    }
    if (arg === "--base-fixture") {
      parsed.baseFixture = String(argv[i + 1] ?? parsed.baseFixture);
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      usage();
    }
    usage();
  }

  if (!["legacy", "bounded"].includes(parsed.variant)) {
    throw new Error("--variant must be legacy or bounded.");
  }
  for (const [name, value] of Object.entries({
    messageCount: parsed.messageCount,
    messageBytes: parsed.messageBytes,
    toolSummaries: parsed.toolSummaries,
    boundedToolSummaries: parsed.boundedToolSummaries,
    toolPreviewBytes: parsed.toolPreviewBytes,
    gapCount: parsed.gapCount,
    firstGapDelayMs: parsed.firstGapDelayMs,
    gapIntervalMs: parsed.gapIntervalMs,
  })) {
    if (!Number.isInteger(value) || value < 0) {
      throw new Error(`--${name} must be a non-negative integer.`);
    }
  }
  if (parsed.messageCount < 2) {
    throw new Error("--message-count must be at least 2.");
  }
  if (parsed.toolSummaries < 1) {
    throw new Error("--tool-summaries must be at least 1.");
  }
  if (parsed.boundedToolSummaries < 1 || parsed.boundedToolSummaries > parsed.toolSummaries) {
    throw new Error("--bounded-tool-summaries must be from 1 to --tool-summaries.");
  }
  if (parsed.gapCount < 1) {
    throw new Error("--gap-count must be at least 1.");
  }

  return parsed;
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function iso(offsetMs) {
  return new Date(baseTimeMs + offsetMs).toISOString();
}

function repeatToMinimum(prefix, bytes) {
  if (prefix.length >= bytes) return prefix;
  const filler = " deterministic foreground large-head payload";
  let next = prefix;
  while (next.length < bytes) {
    next += filler;
  }
  return next.slice(0, bytes);
}

function byteLength(value) {
  return Buffer.byteLength(JSON.stringify(value));
}

function buildSession(baseFixture) {
  const existing =
    baseFixture.session_snapshots?.[foregroundSessionId]?.head?.session ??
    baseFixture.active_snapshot?.active?.tasks?.[0]?.primary_session?.session;
  return {
    ...existing,
    id: foregroundSessionId,
    task_id: foregroundTaskId,
    workspace_id: workspaceId,
    worktree_id: "wt-foreground-large-head",
    provider_id: "fake",
    model_id: "fake-model",
    title: "Foreground large-head proof",
    agent_role: "assistant",
    status: "active",
    created_at: iso(0),
    updated_at: iso(20_000),
  };
}

function buildTurn(status, lastSeq, toolCount) {
  return {
    turn_id: foregroundTurnId,
    session_id: foregroundSessionId,
    user_message_id: "msg-foreground-user",
    status,
    start_seq: 1,
    end_seq: status === "completed" ? lastSeq : null,
    started_at: iso(0),
    updated_at: iso(20_000),
    assistant_partial: null,
    thought_partial: null,
    tool_total: toolCount,
    tool_pending: status === "completed" ? 0 : 1,
    tool_running: status === "completed" ? 0 : 1,
    tool_completed: status === "completed" ? toolCount : Math.max(0, toolCount - 1),
    tool_failed: 0,
  };
}

function buildMessages(messageCount, messageBytes, assistantOrderOffset = 1) {
  const messages = [
    {
      id: "msg-foreground-user",
      session_id: foregroundSessionId,
      task_id: foregroundTaskId,
      turn_id: foregroundTurnId,
      order_seq: 1,
      turn_sequence: 1,
      role: "user",
      content: "Please keep working until the foreground session detail pane is fresh.",
      delivery: "immediate",
      created_at: iso(0),
    },
  ];
  const assistantCount = messageCount - 1;
  for (let index = 0; index < assistantCount; index += 1) {
    const isFinal = index === assistantCount - 1;
    const label = String(index + 1).padStart(3, "0");
    const content = isFinal
      ? finalContent
      : repeatToMinimum(`Foreground large-head assistant progress ${label}.`, messageBytes);
    messages.push({
      id: `msg-foreground-assistant-${label}`,
      session_id: foregroundSessionId,
      task_id: foregroundTaskId,
      turn_id: foregroundTurnId,
      order_seq: assistantOrderOffset + index + 1,
      turn_sequence: assistantOrderOffset + index + 1,
      role: "assistant",
      content,
      delivery: "immediate",
      created_at: iso(1000 + index * 50),
    });
  }
  return messages;
}

function buildToolSummaries(count, previewBytes) {
  return Array.from({ length: count }, (_, index) => {
    const label = String(index).padStart(3, "0");
    return {
      session_id: foregroundSessionId,
      tool_call_id: `tool-${label}`,
      turn_id: foregroundTurnId,
      tool_kind: "shell",
      provider_tool_name: "exec_command",
      title: `tool ${label}`,
      subtitle: "large-head deterministic summary",
      status: "completed",
      input_preview: {
        cmd: `printf proof-${label}`,
        payload: repeatToMinimum(`input-preview-${label}`, Math.max(0, Math.floor(previewBytes / 3))),
      },
      output_preview: repeatToMinimum(`output-preview-${label}`, previewBytes),
      order_seq: index + 2,
      input_truncated: true,
      input_original_bytes: previewBytes * 2,
      output_truncated: true,
      output_original_bytes: previewBytes * 4,
      first_event_seq: index + 2,
      created_at: iso(2000 + index * 10),
      updated_at: iso(2500 + index * 10),
    };
  });
}

function attachHeadWindow(head, limits) {
  head.head_window = {
    turn_limit: limits.turnLimit,
    message_limit: limits.messageLimit,
    event_limit: 0,
    byte_limit: limits.byteLimit,
    turn_count: head.turns.length,
    message_count: head.messages.length,
    event_count: Array.isArray(head.events) ? head.events.length : 0,
    bytes: byteLength({
      turns: head.turns,
      messages: head.messages,
      events: head.events ?? [],
      tool_summaries: head.tool_summaries ?? [],
    }),
    truncated: Boolean(limits.truncated),
  };
  return head;
}

function buildHead({ session, status, messages, toolSummaries, lastSeq, variant }) {
  const head = {
    session,
    turns: [buildTurn(status, lastSeq, toolSummaries.length)],
    tool_summaries: toolSummaries,
    events: [],
    messages,
    last_event_seq: lastSeq,
    projection_rev: lastSeq,
    state_rev: lastSeq,
    activity: {
      is_working: status !== "completed",
      last_turn_status: status,
    },
    has_more_turns: false,
    has_more_history: false,
    history_cursor: null,
  };
  return attachHeadWindow(head, {
    turnLimit: 5,
    messageLimit: 200,
    byteLimit: variant === "bounded" ? 256_000 : 1_500_000,
    truncated: variant === "bounded",
  });
}

function buildSummary(session, head, preview) {
  return {
    session,
    last_message_at: iso(20_000),
    last_message_preview: preview,
    last_event_seq: head.last_event_seq,
    projection_rev: head.projection_rev,
    state_rev: head.state_rev,
    activity: head.activity,
    unread: false,
  };
}

function updateWorkspace(baseFixture, runningHead, recoveredHead) {
  const fixture = clone(baseFixture);
  fixture.workspace = {
    id: workspaceId,
    name: "Foreground Large Head SLA",
    root_path: "/tmp/ctx-foreground-large-head",
    created_at: iso(0),
  };
  fixture.providers = [
    {
      provider_id: "fake",
      name: "Fake",
      harness: "fake",
      is_enabled: true,
      is_default: true,
    },
  ];

  fixture.active_snapshot.workspace_id = workspaceId;
  fixture.active_snapshot.snapshot_rev = 1;
  fixture.active_snapshot.archived_rev = 0;

  const foregroundEntry = fixture.active_snapshot.active.tasks[0];
  foregroundEntry.task = {
    ...foregroundEntry.task,
    id: foregroundTaskId,
    workspace_id: workspaceId,
    title: "Foreground large-head proof",
    status: "completed",
    primary_session_id: foregroundSessionId,
    primary_worktree_id: "wt-foreground-large-head",
    updated_at: iso(15_000),
    last_activity_at: iso(15_000),
    has_active_session: false,
  };
  foregroundEntry.primary_session = buildSummary(runningHead.session, runningHead, "Foreground recovery pending");
  foregroundEntry.primary_session_head = runningHead;
  foregroundEntry.sessions = [foregroundEntry.primary_session];
  foregroundEntry.sort_at = iso(15_000);

  const backgroundEntry = fixture.active_snapshot.active.tasks[1];
  if (backgroundEntry) {
    const backgroundSessionId = backgroundEntry.primary_session?.session?.id ?? "session-b";
    const backgroundTaskId = backgroundEntry.task?.id ?? "task-b";
    backgroundEntry.task = {
      ...backgroundEntry.task,
      id: backgroundTaskId,
      workspace_id: workspaceId,
      title: "Background nav churn control",
      updated_at: iso(16_000),
      last_activity_at: iso(16_000),
    };
    for (const key of ["primary_session", "primary_session_head"]) {
      const target = backgroundEntry[key];
      if (target?.session) {
        target.session = {
          ...target.session,
          id: backgroundSessionId,
          task_id: backgroundTaskId,
          workspace_id: workspaceId,
        };
      }
      if (key === "primary_session_head") {
        target.messages = (target.messages ?? []).map((message) => ({
          ...message,
          session_id: backgroundSessionId,
          task_id: backgroundTaskId,
        }));
      }
    }
  }

  fixture.session_snapshots = {
    ...fixture.session_snapshots,
    [foregroundSessionId]: {
      summary: buildSummary(recoveredHead.session, recoveredHead, finalContent),
      head: recoveredHead,
      state: {
        artifacts: [],
        git_status: null,
      },
    },
  };

  for (const [sessionId, snapshot] of Object.entries(fixture.session_snapshots)) {
    if (sessionId === foregroundSessionId) continue;
    if (snapshot?.summary?.session) {
      snapshot.summary.session.workspace_id = workspaceId;
    }
    if (snapshot?.head?.session) {
      snapshot.head.session.workspace_id = workspaceId;
    }
  }

  fixture.active_heads = {
    workspace_id: workspaceId,
    snapshot_rev: 1,
    heads: fixture.active_snapshot.active.tasks
      .map((entry) => entry.primary_session_head)
      .filter(Boolean),
  };
  return fixture;
}

function buildStream(args, lastSeq) {
  let rev = 1;
  const streamEvent = (delayMs, event) => ({
    delay_ms: delayMs,
    event: {
      type: "event",
      rev: rev += 1,
      event,
    },
  });
  const stream = [
    streamEvent(
      0,
      {
        type: "ready",
        workspace_id: workspaceId,
        snapshot_rev: 1,
        archived_rev: 0,
      },
    ),
  ];
  stream.push(streamEvent(
    Math.max(500, args.firstGapDelayMs - 750),
    {
      type: "session_head_delta",
      workspace_id: workspaceId,
      snapshot_rev: 1,
      delta: {
        session_id: foregroundSessionId,
        last_event_seq: 2,
        projection_rev: 2,
        state_rev: 2,
        turn: buildTurn("completed", 2, 0),
        message: {
          id: "msg-foreground-pregap-progress",
          session_id: foregroundSessionId,
          task_id: foregroundTaskId,
          turn_id: foregroundTurnId,
          order_seq: 2,
          turn_sequence: 2,
          role: "assistant",
          content: "Foreground pre-gap live progress",
          delivery: "immediate",
          created_at: iso(3000),
        },
      },
    },
  ));
  for (let index = 0; index < args.gapCount; index += 1) {
    stream.push(streamEvent(
      args.firstGapDelayMs + index * args.gapIntervalMs,
      {
        type: "task_delta",
        workspace_id: workspaceId,
        snapshot_rev: 1,
        delta: {
          kind: "updated",
          task: {
            id: foregroundTaskId,
            workspace_id: workspaceId,
            title: "Foreground large-head proof",
            description: null,
            status: "completed",
            primary_session_id: foregroundSessionId,
            primary_worktree_id: "wt-foreground-large-head",
            created_at: iso(0),
            updated_at: iso(15_000 + index * 500),
            archived_at: null,
            assistant_seen_at: null,
            last_activity_at: iso(15_000 + index * 500),
            last_assistant_message_at: iso(15_000 + index * 500),
            has_active_session: false,
          },
        },
      },
    ));
    stream.push(streamEvent(
      args.firstGapDelayMs + 25 + index * args.gapIntervalMs,
      {
        type: "session_gap",
        workspace_id: workspaceId,
        snapshot_rev: 1,
        session_id: foregroundSessionId,
        after_seq: index === 0 ? 1 : lastSeq,
        reason: "foreground_large_head_sla_fixture",
      },
    ));
  }
  return stream;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const baseFixture = JSON.parse(await readFile(args.baseFixture, "utf8"));
  const session = buildSession(baseFixture);
  const allToolSummaries = buildToolSummaries(args.toolSummaries, args.toolPreviewBytes);
  const selectedToolSummaries =
    args.variant === "bounded"
      ? allToolSummaries.slice(-args.boundedToolSummaries)
      : allToolSummaries;
  const runningMessages = buildMessages(2, args.messageBytes).slice(0, 1);
  const recoveredMessages = buildMessages(
    args.messageCount,
    args.messageBytes,
    args.toolSummaries + 1,
  );
  const lastSeq = args.messageCount + args.toolSummaries;
  const runningHead = buildHead({
    session,
    status: "completed",
    messages: runningMessages,
    toolSummaries: selectedToolSummaries,
    lastSeq: 1,
    variant: args.variant,
  });
  const recoveredHead = buildHead({
    session,
    status: "completed",
    messages: recoveredMessages,
    toolSummaries: selectedToolSummaries,
    lastSeq,
    variant: args.variant,
  });
  const fixture = updateWorkspace(baseFixture, runningHead, recoveredHead);
  fixture.stream = buildStream(args, lastSeq);
  fixture.foreground_large_head = {
    variant: args.variant,
    final_content: finalContent,
    message_count: recoveredHead.messages.length,
    tool_summary_count: recoveredHead.tool_summaries.length,
    source_tool_summary_count: args.toolSummaries,
    head_response_bytes: byteLength(recoveredHead),
    active_head_response_bytes: byteLength(runningHead),
    gap_count: args.gapCount,
  };

  await mkdir(path.dirname(args.out), { recursive: true });
  await writeFile(args.out, `${JSON.stringify(fixture, null, 2)}\n`);
  console.log(
    `wrote ${args.variant} foreground large-head fixture to ${args.out} ` +
      `(head_bytes=${fixture.foreground_large_head.head_response_bytes}, ` +
      `tool_summaries=${fixture.foreground_large_head.tool_summary_count})`,
  );
}

await main();
