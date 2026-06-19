import type { ChangeSet, Contribution, ContributionEndpoint, PullRequestLink, PullRequestRef } from "@ctx/types";
import type { WorkspaceActiveSnapshotItem } from "../../state/workspaceActiveSnapshotStore";
import type {
  AgentWorkEndpointBucket,
  AgentWorkIndexedEndpoint,
  WorkspaceAgentWorkGraph,
} from "../../state/workspaceAgentWorkStore";
import { indexedContributionEndpoint, pullRequestEndpointKey } from "../../state/workspaceAgentWorkStore";

export type AgentWorkTaskSummary = {
  taskId: string;
  changeSetCount: number;
  contributionCount: number;
  linkedPullRequestCount: number;
  latestUpdateTimestamp: string | null;
};

export type AgentWorkLinkedPullRequest = {
  key: string;
  pullRequest: PullRequestRef;
  links: PullRequestLink[];
  changeSetIds: string[];
  contributionIds: string[];
  latestUpdateTimestamp: string | null;
};

export type AgentWorkTaskDetailCounts = {
  changeSets: number;
  contributions: number;
  linkedPullRequests: number;
};

export type AgentWorkTaskDetail = AgentWorkTaskSummary & {
  counts: AgentWorkTaskDetailCounts;
  changeSetIds: string[];
  contributionIds: string[];
  linkedPullRequestKeys: string[];
  changeSets: ChangeSet[];
  contributions: Contribution[];
  linkedPullRequests: AgentWorkLinkedPullRequest[];
};

export type WorkbenchTaskBoardLaneId = "active" | "needs-review" | "archived" | "other";

export type WorkbenchTaskBoardCard = {
  taskId: string;
  item: WorkspaceActiveSnapshotItem;
  agentWorkSummary: AgentWorkTaskSummary;
  laneId: WorkbenchTaskBoardLaneId;
  sortAtMs: number;
};

export type WorkbenchTaskBoardLane = {
  id: WorkbenchTaskBoardLaneId;
  title: string;
  cards: WorkbenchTaskBoardCard[];
};

export type WorkbenchTaskBoardProjection = {
  lanes: WorkbenchTaskBoardLane[];
  cardsByTaskId: Record<string, WorkbenchTaskBoardCard>;
};

const emptyTaskSummary = (taskId: string): AgentWorkTaskSummary => ({
  taskId,
  changeSetCount: 0,
  contributionCount: 0,
  linkedPullRequestCount: 0,
  latestUpdateTimestamp: null,
});

const emptyTaskDetail = (taskId: string): AgentWorkTaskDetail => ({
  ...emptyTaskSummary(taskId),
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

export const formatAgentWorkSummaryChips = (
  summary: AgentWorkTaskSummary | null | undefined,
): string[] => {
  const chips: string[] = [];
  if (summary?.changeSetCount) {
    chips.push(`${summary.changeSetCount} change${summary.changeSetCount === 1 ? "" : "s"}`);
  }
  if (summary?.linkedPullRequestCount) {
    chips.push(`${summary.linkedPullRequestCount} PR${summary.linkedPullRequestCount === 1 ? "" : "s"}`);
  }
  if (chips.length === 0 && summary?.contributionCount) {
    chips.push(`${summary.contributionCount} link${summary.contributionCount === 1 ? "" : "s"}`);
  }
  return chips;
};

const addLatestTimestamp = (current: string | null, candidate: string | null | undefined): string | null => {
  if (!candidate) return current;
  if (!current) return candidate;
  const currentMs = Date.parse(current);
  const candidateMs = Date.parse(candidate);
  if (Number.isFinite(currentMs) && Number.isFinite(candidateMs)) {
    return candidateMs > currentMs ? candidate : current;
  }
  return candidate > current ? candidate : current;
};

const addRecordTimestamps = (
  latest: string | null,
  record: Pick<ChangeSet | Contribution, "created_at" | "updated_at">,
): string | null => addLatestTimestamp(addLatestTimestamp(latest, record.created_at), record.updated_at);

const endpointPullRequestKey = (endpoint: ContributionEndpoint): string | null => {
  const indexed = indexedContributionEndpoint(endpoint);
  return indexed?.kind === "pull_request" ? indexed.id : null;
};

const addContributionPullRequestKeys = (keys: Set<string>, contribution: Contribution): void => {
  const subjectKey = endpointPullRequestKey(contribution.subject);
  const targetKey = endpointPullRequestKey(contribution.target);
  if (subjectKey) keys.add(subjectKey);
  if (targetKey) keys.add(targetKey);
};

const addChangeSetPullRequestKeys = (keys: Set<string>, changeSet: ChangeSet): void => {
  for (const link of changeSet.pull_requests ?? []) {
    const key = pullRequestEndpointKey(link.pull_request);
    if (key) keys.add(key);
  }
};

const pullRequestFromEndpoint = (endpoint: ContributionEndpoint): { key: string; pullRequest: PullRequestRef } | null => {
  const indexed = indexedContributionEndpoint(endpoint);
  return indexed?.kind === "pull_request" ? { key: indexed.id, pullRequest: indexed.pullRequest } : null;
};

const endpointQueueKey = (endpoint: AgentWorkIndexedEndpoint): string => `${endpoint.kind}:${endpoint.id}`;

type TraversableAgentWorkEndpoint = Extract<AgentWorkIndexedEndpoint, { kind: "task" | "session" | "run" | "worktree" }>;

const isTraversableEndpoint = (
  endpoint: AgentWorkIndexedEndpoint | null | undefined,
): endpoint is TraversableAgentWorkEndpoint =>
  endpoint?.kind === "task" || endpoint?.kind === "session" || endpoint?.kind === "run" || endpoint?.kind === "worktree";

const bucketForEndpoint = (
  graph: WorkspaceAgentWorkGraph,
  endpoint: TraversableAgentWorkEndpoint,
): AgentWorkEndpointBucket | undefined => {
  switch (endpoint.kind) {
    case "task":
      return graph.endpointIndexes.tasksById[endpoint.id];
    case "session":
      return graph.endpointIndexes.sessionsById[endpoint.id];
    case "run":
      return graph.endpointIndexes.runsById[endpoint.id];
    case "worktree":
      return graph.endpointIndexes.worktreesById[endpoint.id];
  }
};

const shouldIncludeWorktreeChangeSet = (
  graph: WorkspaceAgentWorkGraph,
  changeSetId: string,
  reachableContributionIds: Set<string>,
): boolean => {
  const changeSetBucket = graph.endpointIndexes.changeSetsById[changeSetId];
  if (!changeSetBucket || changeSetBucket.contributionIds.length === 0) return true;
  return changeSetBucket.contributionIds.some((contributionId) => reachableContributionIds.has(contributionId));
};

const sortedIds = (ids: Iterable<string>): string[] => Array.from(ids).sort((left, right) => left.localeCompare(right));

const taskSummaryFromDetail = (detail: AgentWorkTaskDetail): AgentWorkTaskSummary => ({
  taskId: detail.taskId,
  changeSetCount: detail.changeSetCount,
  contributionCount: detail.contributionCount,
  linkedPullRequestCount: detail.linkedPullRequestCount,
  latestUpdateTimestamp: detail.latestUpdateTimestamp,
});

export const projectAgentWorkForTask = (
  graph: WorkspaceAgentWorkGraph,
  taskId: string | null | undefined,
): AgentWorkTaskDetail => {
  const normalizedTaskId = String(taskId ?? "").trim();
  if (!normalizedTaskId) return emptyTaskDetail("");

  const changeSetIds = new Set<string>();
  const contributionIds = new Set<string>();
  const pullRequestKeys = new Set<string>();
  let latestUpdateTimestamp: string | null = null;
  const queuedEndpointKeys = new Set<string>();
  const queue: TraversableAgentWorkEndpoint[] = [{ kind: "task", id: normalizedTaskId }];
  queuedEndpointKeys.add(endpointQueueKey(queue[0]));

  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const endpoint = queue[cursor];
    const bucket = bucketForEndpoint(graph, endpoint);
    if (!bucket) continue;

    for (const changeSetId of bucket.changeSetIds) {
      if (endpoint.kind === "worktree" && !shouldIncludeWorktreeChangeSet(graph, changeSetId, contributionIds)) {
        continue;
      }
      changeSetIds.add(changeSetId);
    }

    if (endpoint.kind === "worktree") continue;

    for (const contributionId of bucket.contributionIds) {
      if (contributionIds.has(contributionId)) continue;
      contributionIds.add(contributionId);
      const contribution = graph.contributionsById[contributionId];
      if (!contribution) continue;
      latestUpdateTimestamp = addRecordTimestamps(latestUpdateTimestamp, contribution);
      addContributionPullRequestKeys(pullRequestKeys, contribution);
      if (contribution.change_set_id) changeSetIds.add(contribution.change_set_id);

      const subject = indexedContributionEndpoint(contribution.subject);
      const target = indexedContributionEndpoint(contribution.target);
      if (subject?.kind === "change_set") changeSetIds.add(subject.id);
      if (target?.kind === "change_set") changeSetIds.add(target.id);

      for (const linkedEndpoint of [subject, target]) {
        if (!isTraversableEndpoint(linkedEndpoint)) continue;
        const key = endpointQueueKey(linkedEndpoint);
        if (queuedEndpointKeys.has(key)) continue;
        queuedEndpointKeys.add(key);
        queue.push(linkedEndpoint);
      }
    }
  }

  for (const changeSetId of changeSetIds) {
    const changeSet = graph.changeSetsById[changeSetId];
    if (!changeSet) continue;
    latestUpdateTimestamp = addRecordTimestamps(latestUpdateTimestamp, changeSet);
    addChangeSetPullRequestKeys(pullRequestKeys, changeSet);
  }

  const changeSetIdList = sortedIds(changeSetIds);
  const contributionIdList = sortedIds(contributionIds);
  const linkedPullRequestKeys = sortedIds(pullRequestKeys);
  const changeSets = changeSetIdList.flatMap((changeSetId) => {
    const changeSet = graph.changeSetsById[changeSetId];
    return changeSet ? [changeSet] : [];
  });
  const contributions = contributionIdList.flatMap((contributionId) => {
    const contribution = graph.contributionsById[contributionId];
    return contribution ? [contribution] : [];
  });
  const linkedPullRequests = linkedPullRequestKeys.map((key) =>
    projectLinkedPullRequest(graph, key, changeSetIds, contributionIds),
  );

  return {
    taskId: normalizedTaskId,
    changeSetCount: changeSetIds.size,
    contributionCount: contributionIds.size,
    linkedPullRequestCount: pullRequestKeys.size,
    latestUpdateTimestamp,
    counts: {
      changeSets: changeSetIds.size,
      contributions: contributionIds.size,
      linkedPullRequests: pullRequestKeys.size,
    },
    changeSetIds: changeSetIdList,
    contributionIds: contributionIdList,
    linkedPullRequestKeys,
    changeSets,
    contributions,
    linkedPullRequests,
  };
};

const projectLinkedPullRequest = (
  graph: WorkspaceAgentWorkGraph,
  key: string,
  taskChangeSetIds: Set<string>,
  taskContributionIds: Set<string>,
): AgentWorkLinkedPullRequest => {
  const indexedBucket = graph.endpointIndexes.pullRequestsByKey[key];
  const changeSetIds = new Set<string>();
  const contributionIds = new Set<string>();
  const linksByKey = new Map<string, PullRequestLink>();
  let pullRequest = indexedBucket?.pullRequest ?? null;
  let latestUpdateTimestamp: string | null = null;

  for (const changeSetId of taskChangeSetIds) {
    const changeSet = graph.changeSetsById[changeSetId];
    if (!changeSet) continue;
    for (const link of changeSet.pull_requests ?? []) {
      const linkKey = pullRequestEndpointKey(link.pull_request);
      if (linkKey !== key) continue;
      changeSetIds.add(changeSetId);
      pullRequest ??= link.pull_request;
      linksByKey.set(JSON.stringify([link.kind ?? "", link.url ?? "", link.title ?? "", link.state ?? ""]), link);
      latestUpdateTimestamp = addRecordTimestamps(latestUpdateTimestamp, changeSet);
    }
  }

  for (const contributionId of taskContributionIds) {
    const contribution = graph.contributionsById[contributionId];
    if (!contribution) continue;
    for (const endpoint of [contribution.subject, contribution.target]) {
      const endpointPullRequest = pullRequestFromEndpoint(endpoint);
      if (endpointPullRequest?.key !== key) continue;
      contributionIds.add(contributionId);
      pullRequest ??= endpointPullRequest.pullRequest;
      latestUpdateTimestamp = addRecordTimestamps(latestUpdateTimestamp, contribution);
    }
  }

  return {
    key,
    pullRequest:
      pullRequest ??
      ({
        provider: "",
        owner: "",
        repo: "",
        number: 0,
      } satisfies PullRequestRef),
    links: Array.from(linksByKey.values()).sort((left, right) =>
      JSON.stringify(left).localeCompare(JSON.stringify(right)),
    ),
    changeSetIds: sortedIds(changeSetIds),
    contributionIds: sortedIds(contributionIds),
    latestUpdateTimestamp,
  };
};

export const summarizeAgentWorkForTask = (
  graph: WorkspaceAgentWorkGraph,
  taskId: string | null | undefined,
): AgentWorkTaskSummary => taskSummaryFromDetail(projectAgentWorkForTask(graph, taskId));

const BOARD_LANE_ORDER: WorkbenchTaskBoardLaneId[] = ["active", "needs-review", "archived", "other"];

const BOARD_LANE_TITLES: Record<WorkbenchTaskBoardLaneId, string> = {
  active: "Active",
  "needs-review": "Needs review",
  archived: "Archived",
  other: "Other",
};

const ACTIVE_TASK_STATUSES = new Set(["active", "running", "queued", "starting", "working", "in_progress", "in-progress"]);
const REVIEW_ENDPOINT_KINDS = new Set<AgentWorkIndexedEndpoint["kind"]>(["check", "review_attestation"]);

const taskStatusText = (item: WorkspaceActiveSnapshotItem): string => String(item.task.status ?? "").trim().toLowerCase();

const hasActiveSession = (item: WorkspaceActiveSnapshotItem): boolean =>
  Boolean(item.task.has_active_session) ||
  item.sessions.some(
    (session) => session.activity?.is_working || String(session.session.status ?? "").toLowerCase() === "running",
  );

const taskNeedsReview = (detail: AgentWorkTaskDetail): boolean => {
  if (detail.linkedPullRequestCount > 0) return true;
  return detail.contributions.some((contribution) => {
    if (contribution.role === "reviewed" || contribution.role === "validated") return true;
    const subject = indexedContributionEndpoint(contribution.subject);
    const target = indexedContributionEndpoint(contribution.target);
    return REVIEW_ENDPOINT_KINDS.has(subject?.kind ?? "system") || REVIEW_ENDPOINT_KINDS.has(target?.kind ?? "system");
  });
};

export const selectWorkbenchTaskBoardLane = (
  item: WorkspaceActiveSnapshotItem,
  detail: AgentWorkTaskDetail,
): WorkbenchTaskBoardLaneId => {
  if (item.task.archived_at) return "archived";
  const status = taskStatusText(item);
  if (hasActiveSession(item) || ACTIVE_TASK_STATUSES.has(status)) return "active";
  // Until the task model exposes a review state, PR/check/review-attestation graph links are the review signal.
  if (taskNeedsReview(detail)) return "needs-review";
  return "other";
};

export const projectWorkbenchTaskBoard = (
  graph: WorkspaceAgentWorkGraph,
  items: WorkspaceActiveSnapshotItem[],
): WorkbenchTaskBoardProjection => {
  const laneById: Record<WorkbenchTaskBoardLaneId, WorkbenchTaskBoardLane> = {
    active: { id: "active", title: BOARD_LANE_TITLES.active, cards: [] },
    "needs-review": { id: "needs-review", title: BOARD_LANE_TITLES["needs-review"], cards: [] },
    archived: { id: "archived", title: BOARD_LANE_TITLES.archived, cards: [] },
    other: { id: "other", title: BOARD_LANE_TITLES.other, cards: [] },
  };
  const lanes = BOARD_LANE_ORDER.map((id) => laneById[id]);
  const cardsByTaskId: Record<string, WorkbenchTaskBoardCard> = {};

  for (const item of items) {
    const detail = projectAgentWorkForTask(graph, item.id);
    const laneId = selectWorkbenchTaskBoardLane(item, detail);
    const card: WorkbenchTaskBoardCard = {
      taskId: item.id,
      item,
      agentWorkSummary: taskSummaryFromDetail(detail),
      laneId,
      sortAtMs: Number.isFinite(item.sortAtMs) ? item.sortAtMs : 0,
    };
    cardsByTaskId[item.id] = card;
    laneById[laneId].cards.push(card);
  }

  for (const lane of lanes) {
    lane.cards.sort((left, right) => right.sortAtMs - left.sortAtMs || left.taskId.localeCompare(right.taskId));
  }

  return {
    lanes,
    cardsByTaskId,
  };
};
