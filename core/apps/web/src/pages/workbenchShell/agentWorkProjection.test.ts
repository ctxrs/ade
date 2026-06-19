import { describe, expect, it } from "vitest";
import type { ChangeSet, Contribution, PullRequestRef, Task } from "@ctx/types";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import { normalizeWorkspaceAgentWork } from "../../state/workspaceAgentWorkStore";
import { projectAgentWorkForTask, projectWorkbenchTaskBoard, summarizeAgentWorkForTask } from "./agentWorkProjection";

const pr = (number: number): PullRequestRef => ({
  provider: "github",
  owner: "ctxrs",
  repo: "ctx",
  number,
});

const changeSet = (overrides: Partial<ChangeSet> & Pick<ChangeSet, "id">): ChangeSet => ({
  workspace_id: "workspace-1",
  ...overrides,
});

const contribution = (overrides: Partial<Contribution> & Pick<Contribution, "id">): Contribution => ({
  workspace_id: "workspace-1",
  subject: { kind: "system", label: "test" },
  target: { kind: "system", label: "test" },
  ...overrides,
});

const taskItem = (
  id: string,
  overrides: {
    task?: Partial<Task>;
    sortAtMs?: number;
  } = {},
): WorkspaceActiveSnapshotItem => ({
  id,
  task: {
    id,
    workspace_id: "workspace-1",
    title: id,
    status: "ready",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    ...overrides.task,
  },
  sessions: [],
  sort_at: null,
  sortAtMs: overrides.sortAtMs ?? 0,
});

describe("agentWorkProjection", () => {
  it("projects an empty task detail for an empty graph", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [],
      contributions: [],
    });

    expect(projectAgentWorkForTask(graph, "task-missing")).toEqual({
      taskId: "task-missing",
      changeSetCount: 0,
      contributionCount: 0,
      linkedPullRequestCount: 0,
      latestUpdateTimestamp: null,
      counts: {
        changeSets: 0,
        contributions: 0,
        linkedPullRequests: 0,
      },
      changeSetIds: [],
      contributionIds: [],
      linkedPullRequestKeys: [],
      changeSets: [],
      contributions: [],
      linkedPullRequests: [],
    });
  });

  it("summarizes task-linked change sets, contributions, pull requests, and latest timestamp", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "change-set-1",
          created_at: "2026-01-01T10:00:00Z",
          updated_at: "2026-01-02T10:00:00Z",
          pull_requests: [{ pull_request: pr(41), kind: "result" }],
        }),
        changeSet({
          id: "change-set-2",
          created_at: "2026-01-03T10:00:00Z",
          updated_at: "2026-01-03T11:00:00Z",
        }),
      ],
      contributions: [
        contribution({
          id: "contribution-1",
          created_at: "2026-01-01T12:00:00Z",
          subject: { kind: "task", task_id: "task-1" },
          target: { kind: "change_set", change_set_id: "change-set-1" },
        }),
        contribution({
          id: "contribution-2",
          change_set_id: "change-set-2",
          updated_at: "2026-01-03T12:00:00Z",
          subject: { kind: "task", task_id: "task-1" },
          target: { kind: "session", session_id: "session-1" },
        }),
        contribution({
          id: "contribution-3",
          updated_at: "2026-01-04T09:00:00Z",
          subject: { kind: "task", task_id: "task-1" },
          target: { kind: "pull_request", pull_request: pr(42) },
        }),
        contribution({
          id: "contribution-4",
          subject: { kind: "task", task_id: "task-2" },
          target: { kind: "change_set", change_set_id: "change-set-1" },
        }),
      ],
    });

    expect(summarizeAgentWorkForTask(graph, "task-1")).toEqual({
      taskId: "task-1",
      changeSetCount: 2,
      contributionCount: 3,
      linkedPullRequestCount: 2,
      latestUpdateTimestamp: "2026-01-04T09:00:00Z",
    });
  });

  it("returns an empty summary for tasks without indexed agent work", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [],
      contributions: [],
    });

    expect(summarizeAgentWorkForTask(graph, "task-missing")).toEqual({
      taskId: "task-missing",
      changeSetCount: 0,
      contributionCount: 0,
      linkedPullRequestCount: 0,
      latestUpdateTimestamp: null,
    });
  });

  it("summarizes work reachable through parent and child agent sessions", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "change-set-from-child",
          updated_at: "2026-01-05T10:00:00Z",
        }),
      ],
      contributions: [
        contribution({
          id: "task-parent-session",
          subject: { kind: "task", task_id: "task-with-subagents" },
          target: { kind: "session", session_id: "session-parent" },
        }),
        contribution({
          id: "parent-child-session",
          subject: { kind: "session", session_id: "session-parent" },
          target: { kind: "session", session_id: "session-child" },
        }),
        contribution({
          id: "child-produced-change-set",
          updated_at: "2026-01-05T11:00:00Z",
          subject: { kind: "session", session_id: "session-child" },
          target: { kind: "change_set", change_set_id: "change-set-from-child" },
        }),
      ],
    });

    expect(summarizeAgentWorkForTask(graph, "task-with-subagents")).toEqual({
      taskId: "task-with-subagents",
      changeSetCount: 1,
      contributionCount: 3,
      linkedPullRequestCount: 0,
      latestUpdateTimestamp: "2026-01-05T11:00:00Z",
    });
  });

  it("summarizes change sets captured from a session worktree", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "change-set-from-worktree",
          source_worktree_id: "worktree-1",
          updated_at: "2026-01-06T10:00:00Z",
          pull_requests: [{ pull_request: pr(43), kind: "result" }],
        }),
      ],
      contributions: [
        contribution({
          id: "task-session",
          subject: { kind: "task", task_id: "task-with-worktree" },
          target: { kind: "session", session_id: "session-1" },
        }),
        contribution({
          id: "session-worktree",
          updated_at: "2026-01-06T11:00:00Z",
          subject: { kind: "session", session_id: "session-1" },
          target: { kind: "worktree", worktree_id: "worktree-1" },
        }),
      ],
    });

    expect(summarizeAgentWorkForTask(graph, "task-with-worktree")).toEqual({
      taskId: "task-with-worktree",
      changeSetCount: 1,
      contributionCount: 2,
      linkedPullRequestCount: 1,
      latestUpdateTimestamp: "2026-01-06T11:00:00Z",
    });
  });

  it("does not summarize another task through a shared worktree", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "change-set-task-1",
          source_worktree_id: "shared-worktree",
          updated_at: "2026-01-07T10:00:00Z",
          pull_requests: [{ pull_request: pr(44), kind: "result" }],
        }),
        changeSet({
          id: "change-set-task-2",
          source_worktree_id: "shared-worktree",
          updated_at: "2026-01-08T10:00:00Z",
          pull_requests: [{ pull_request: pr(45), kind: "result" }],
        }),
      ],
      contributions: [
        contribution({
          id: "task-1-session",
          subject: { kind: "task", task_id: "task-1" },
          target: { kind: "session", session_id: "session-1" },
        }),
        contribution({
          id: "session-1-worktree",
          updated_at: "2026-01-07T11:00:00Z",
          subject: { kind: "session", session_id: "session-1" },
          target: { kind: "worktree", worktree_id: "shared-worktree" },
        }),
        contribution({
          id: "session-1-change-set",
          subject: { kind: "session", session_id: "session-1" },
          target: { kind: "change_set", change_set_id: "change-set-task-1" },
        }),
        contribution({
          id: "task-2-session",
          subject: { kind: "task", task_id: "task-2" },
          target: { kind: "session", session_id: "session-2" },
        }),
        contribution({
          id: "session-2-worktree",
          updated_at: "2026-01-08T11:00:00Z",
          subject: { kind: "session", session_id: "session-2" },
          target: { kind: "worktree", worktree_id: "shared-worktree" },
        }),
        contribution({
          id: "session-2-change-set",
          subject: { kind: "session", session_id: "session-2" },
          target: { kind: "change_set", change_set_id: "change-set-task-2" },
        }),
      ],
    });

    expect(summarizeAgentWorkForTask(graph, "task-1")).toEqual({
      taskId: "task-1",
      changeSetCount: 1,
      contributionCount: 3,
      linkedPullRequestCount: 1,
      latestUpdateTimestamp: "2026-01-07T11:00:00Z",
    });
  });

  it("projects direct change_set_id links with records, counts, and timestamps", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "change-set-direct",
          created_at: "2026-02-01T10:00:00Z",
          updated_at: "2026-02-02T10:00:00Z",
        }),
      ],
      contributions: [
        contribution({
          id: "direct-link",
          change_set_id: "change-set-direct",
          updated_at: "2026-02-03T10:00:00Z",
          subject: { kind: "task", task_id: "task-direct" },
          target: { kind: "system", label: "ctx" },
        }),
      ],
    });

    const detail = projectAgentWorkForTask(graph, "task-direct");

    expect(detail.counts).toEqual({
      changeSets: 1,
      contributions: 1,
      linkedPullRequests: 0,
    });
    expect(detail.changeSetIds).toEqual(["change-set-direct"]);
    expect(detail.contributionIds).toEqual(["direct-link"]);
    expect(detail.changeSets.map((record) => record.id)).toEqual(["change-set-direct"]);
    expect(detail.contributions.map((record) => record.id)).toEqual(["direct-link"]);
    expect(detail.latestUpdateTimestamp).toBe("2026-02-03T10:00:00Z");
  });

  it("projects change sets linked through subject and target endpoints", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({ id: "change-set-subject" }),
        changeSet({ id: "change-set-target", updated_at: "2026-02-04T10:00:00Z" }),
      ],
      contributions: [
        contribution({
          id: "target-endpoint-link",
          subject: { kind: "task", task_id: "task-endpoints" },
          target: { kind: "change-set", id: "change-set-target" },
        }),
        contribution({
          id: "subject-endpoint-link",
          subject: { kind: "change_set", change_set_id: "change-set-subject" },
          target: { kind: "task", task_id: "task-endpoints" },
        }),
      ],
    });

    const detail = projectAgentWorkForTask(graph, "task-endpoints");

    expect(detail.changeSetIds).toEqual(["change-set-subject", "change-set-target"]);
    expect(detail.contributionIds).toEqual(["subject-endpoint-link", "target-endpoint-link"]);
    expect(projectAgentWorkForTask(graph, "task-missing").changeSetIds).toEqual([]);
    expect(detail.linkedPullRequestKeys).toEqual([]);
  });

  it("aggregates pull requests from change sets and contributions without duplicates", () => {
    const pullRequest = pr(50);
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "change-set-pr",
          updated_at: "2026-03-01T10:00:00Z",
          pull_requests: [
            { pull_request: pullRequest, kind: "result", state: "open" },
            { pull_request: pullRequest, kind: "result", state: "open" },
          ],
        }),
      ],
      contributions: [
        contribution({
          id: "task-change-set",
          subject: { kind: "task", task_id: "task-pr" },
          target: { kind: "change_set", change_set_id: "change-set-pr" },
        }),
        contribution({
          id: "task-pr-link",
          updated_at: "2026-03-02T10:00:00Z",
          subject: { kind: "task", task_id: "task-pr" },
          target: { kind: "pull_request", pull_request: pullRequest },
        }),
      ],
    });

    const detail = projectAgentWorkForTask(graph, "task-pr");

    expect(detail.linkedPullRequestCount).toBe(1);
    expect(detail.linkedPullRequests).toHaveLength(1);
    expect(detail.linkedPullRequests[0]).toMatchObject({
      pullRequest,
      changeSetIds: ["change-set-pr"],
      contributionIds: ["task-pr-link"],
      latestUpdateTimestamp: "2026-03-02T10:00:00Z",
    });
    expect(detail.linkedPullRequests[0].links).toEqual([{ pull_request: pullRequest, kind: "result", state: "open" }]);
  });

  it("groups task list items into deterministic board lanes", () => {
    const graph = normalizeWorkspaceAgentWork({
      change_sets: [
        changeSet({
          id: "review-change-set",
          pull_requests: [{ pull_request: pr(60), kind: "result" }],
        }),
      ],
      contributions: [
        contribution({
          id: "review-link",
          subject: { kind: "task", task_id: "review-task" },
          target: { kind: "change_set", change_set_id: "review-change-set" },
        }),
      ],
    });
    const board = projectWorkbenchTaskBoard(graph, [
      taskItem("other-task", { sortAtMs: 4 }),
      taskItem("active-task-low", { task: { status: "running" }, sortAtMs: 1 }),
      taskItem("active-task-high", { task: { has_active_session: true }, sortAtMs: 3 }),
      taskItem("archived-task", { task: { archived_at: "2026-03-01T00:00:00Z" }, sortAtMs: 5 }),
      taskItem("review-task", { sortAtMs: 2 }),
    ]);

    expect(board.lanes.map((lane) => lane.id)).toEqual(["active", "needs-review", "archived", "other"]);
    expect(board.lanes.find((lane) => lane.id === "active")?.cards.map((card) => card.taskId)).toEqual([
      "active-task-high",
      "active-task-low",
    ]);
    // PR-linked agent work is a temporary review signal until task summaries expose an explicit review state.
    expect(board.lanes.find((lane) => lane.id === "needs-review")?.cards.map((card) => card.taskId)).toEqual([
      "review-task",
    ]);
    expect(board.lanes.find((lane) => lane.id === "archived")?.cards.map((card) => card.taskId)).toEqual([
      "archived-task",
    ]);
    expect(board.lanes.find((lane) => lane.id === "other")?.cards.map((card) => card.taskId)).toEqual(["other-task"]);
    expect(board.cardsByTaskId["review-task"].agentWorkSummary.linkedPullRequestCount).toBe(1);
  });
});
