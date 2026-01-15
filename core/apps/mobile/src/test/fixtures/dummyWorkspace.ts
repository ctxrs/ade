import type {
  Message,
  Session,
  SessionHead,
  SessionHeadSnapshot,
  SessionSnapshotSummary,
  SessionTurn,
  WorkspaceActiveSnapshot,
  WorkspaceActiveTaskSummary,
} from "@ctx/types";

import { isoAt } from "../utils/deterministic";

export type DummyWorkspaceOptions = {
  workspaceId?: string;
  taskCount?: number;
  sessionsPerTask?: number;
};

export const buildDummyWorkspaceSnapshot = (opts: DummyWorkspaceOptions = {}): WorkspaceActiveSnapshot => {
  const workspaceId = opts.workspaceId ?? "ws-dummy";
  const taskCount = opts.taskCount ?? 3;
  const sessionsPerTask = opts.sessionsPerTask ?? 4;

  const tasks: WorkspaceActiveTaskSummary[] = [];
  let clock = 0;

  for (let t = 0; t < taskCount; t += 1) {
    const taskId = `task-${t + 1}`;
    const sessions: SessionSnapshotSummary[] = [];

    for (let s = 0; s < sessionsPerTask; s += 1) {
      const sessionId = `session-${t + 1}-${s + 1}`;
      const createdAt = isoAt(clock++);
      const session: Session = {
        id: sessionId,
        task_id: taskId,
        workspace_id: workspaceId,
        worktree_id: `wt-${t + 1}`,
        provider_id: "codex",
        model_id: "gpt-4",
        title: "New Task",
        agent_role: "assistant",
        status: s === 0 ? "active" : "completed",
      };
      sessions.push({
        session,
        last_message_at: createdAt,
        last_message_preview: `Preview ${sessionId}`,
        last_event_seq: s + 1,
        unread: false,
      });
    }

    const primarySession = sessions[0];
    const primaryHead: SessionHeadSnapshot = {
      session: primarySession.session,
      turns: [],
      messages: [],
      last_event_seq: primarySession.last_event_seq ?? 0,
      activity: primarySession.activity ?? { is_working: false, last_turn_status: null },
      has_more_turns: false,
      has_more_history: false,
    };

    tasks.push({
      task: {
        id: taskId,
        workspace_id: workspaceId,
        title: `Task ${t + 1}`,
        description: `Dummy task ${t + 1}`,
        status: "active",
        created_at: isoAt(clock++),
        updated_at: isoAt(clock++),
        archived_at: null,
        primary_session_id: primarySession.session.id,
        primary_worktree_id: `wt-${t + 1}`,
      },
      primary_session: primarySession,
      primary_session_head: primaryHead,
      sessions,
      sort_at: isoAt(clock++),
    });
  }

  return {
    workspace_id: workspaceId,
    snapshot_rev: 1,
    active: {
      tasks,
      total_count: tasks.length,
    },
  };
};

export type DummySessionHeadOptions = {
  sessionId: string;
  taskId: string;
  workspaceId: string;
  turnCount?: number;
};

export const buildDummySessionHead = (opts: DummySessionHeadOptions): SessionHead => {
  const turnCount = opts.turnCount ?? 20;
  const session: Session = {
    id: opts.sessionId,
    task_id: opts.taskId,
    workspace_id: opts.workspaceId,
    worktree_id: `wt-${opts.taskId}`,
    provider_id: "codex",
    model_id: "gpt-4",
    title: "New Task",
    agent_role: "assistant",
    status: "active",
  };

  const turns: SessionTurn[] = [];
  const messages: Message[] = [];
  let seq = 1;

  for (let i = 0; i < turnCount; i += 1) {
    const turnId = `turn-${opts.sessionId}-${i + 1}`;
    turns.push({
      turn_id: turnId,
      session_id: opts.sessionId,
      status: "completed",
      start_seq: seq,
      end_seq: seq + 1,
      started_at: isoAt(seq),
      updated_at: isoAt(seq + 1),
      assistant_partial: "",
      thought_partial: "",
      metrics_json: null,
      tool_total: 0,
      tool_pending: 0,
      tool_running: 0,
      tool_completed: 0,
      tool_failed: 0,
    });
    messages.push({
      id: `msg-${opts.sessionId}-${i + 1}`,
      session_id: opts.sessionId,
      turn_id: turnId,
      role: i % 2 === 0 ? "user" : "assistant",
      content: `Message ${i + 1}`,
      delivery: "immediate",
      created_at: isoAt(seq + 1),
    });
    seq += 2;
  }

  return {
    session,
    turns,
    events: [],
    messages,
    last_event_seq: seq,
    has_more_turns: false,
    has_more_history: false,
  };
};
