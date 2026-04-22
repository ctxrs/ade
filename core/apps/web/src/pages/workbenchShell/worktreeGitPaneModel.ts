import type { DiffUnavailableReason, WorktreeVcsSnapshot, WorktreeVcsTouchedFile } from "@ctx/types";

import { getDiffSummaryStats } from "./useWorkbenchDiffPane";

export type GitPaneSectionKey = "staged" | "unstaged" | "untracked" | "changed";

export type GitPaneFileEntry = {
  path: string;
  origPath: string | null;
  indexStatus: string | null;
  worktreeStatus: string | null;
  section: GitPaneSectionKey;
};

export type GitPaneSection = {
  key: GitPaneSectionKey;
  label: string;
  count: number;
  files: GitPaneFileEntry[];
};

export type GitPaneModel = {
  badgeCount: number;
  totalCount: number;
  available: boolean;
  unavailableReason: DiffUnavailableReason | null;
  unavailableLabel: string | null;
  loading: boolean;
  computeError: string | null;
  listReady: boolean;
  sections: GitPaneSection[];
};

const SECTION_ORDER: GitPaneSectionKey[] = ["staged", "unstaged", "untracked", "changed"];

const SECTION_LABELS: Record<GitPaneSectionKey, string> = {
  staged: "Staged",
  unstaged: "Unstaged",
  untracked: "Untracked",
  changed: "Changed",
};

const normalizeStatus = (value: string | null | undefined): string | null => {
  const trimmed = String(value ?? "").trim();
  return trimmed.length > 0 ? trimmed : null;
};

const isUntracked = (entry: Pick<GitPaneFileEntry, "indexStatus" | "worktreeStatus">) =>
  entry.indexStatus === "?" || entry.worktreeStatus === "?";

const isStaged = (entry: Pick<GitPaneFileEntry, "indexStatus" | "worktreeStatus">) =>
  !!entry.indexStatus && entry.indexStatus !== "?" && entry.indexStatus !== " ";

const isUnstaged = (entry: Pick<GitPaneFileEntry, "indexStatus" | "worktreeStatus">) =>
  !!entry.worktreeStatus && entry.worktreeStatus !== "?" && entry.worktreeStatus !== " ";

export const classifyGitPaneEntry = (
  entry: Pick<GitPaneFileEntry, "indexStatus" | "worktreeStatus">,
): GitPaneSectionKey => {
  if (isUntracked(entry)) return "untracked";
  if (isStaged(entry)) return "staged";
  if (isUnstaged(entry)) return "unstaged";
  return "changed";
};

const unavailableLabelForReason = (reason: DiffUnavailableReason | null): string | null => {
  if (reason === "no_repo") return "No git repo detected for this workspace yet.";
  if (reason === "no_target_branch") return "Primary branch is not configured for this workspace.";
  return null;
};

const mergeInventoryEntries = (snapshot: WorktreeVcsSnapshot): GitPaneFileEntry[] => {
  const byPath = new Map<string, GitPaneFileEntry>();
  const upsert = (raw: WorktreeVcsTouchedFile) => {
    const path = String(raw.path ?? "").trim();
    if (!path) return;
    const next: GitPaneFileEntry = {
      path,
      origPath: raw.orig_path ?? null,
      indexStatus: normalizeStatus(raw.index_status),
      worktreeStatus: normalizeStatus(raw.worktree_status),
      section: "changed",
    };
    const prev = byPath.get(path);
    const merged: GitPaneFileEntry = prev
      ? {
          ...prev,
          origPath: prev.origPath ?? next.origPath,
          indexStatus: prev.indexStatus ?? next.indexStatus,
          worktreeStatus: prev.worktreeStatus ?? next.worktreeStatus,
          section: "changed",
        }
      : next;
    merged.section = classifyGitPaneEntry(merged);
    byPath.set(path, merged);
  };
  for (const entry of snapshot.git_status.entries ?? []) {
    upsert(entry);
  }
  for (const entry of snapshot.touched_files.items ?? []) {
    upsert(entry);
  }
  return Array.from(byPath.values()).sort((a, b) => a.path.localeCompare(b.path));
};

export const buildGitPaneModel = (snapshot: WorktreeVcsSnapshot | null): GitPaneModel => {
  if (!snapshot) {
    return {
      badgeCount: 0,
      totalCount: 0,
      available: true,
      unavailableReason: null,
      unavailableLabel: null,
      loading: true,
      computeError: null,
      listReady: false,
      sections: [],
    };
  }

  const available = snapshot.available !== false;
  const unavailableReason = snapshot.unavailable_reason ?? null;
  const unavailableLabel = available ? null : unavailableLabelForReason(unavailableReason);
  const computeError = available && snapshot.compute_state === "error" ? "Failed to compute diff summary." : null;
  const files = available ? mergeInventoryEntries(snapshot) : [];
  const touchedFilesState = snapshot.touched_files_state ?? "not_loaded";
  const summaryStats = getDiffSummaryStats(snapshot.summary as Record<string, unknown>);
  const badgeCount = available
    ? Math.max(0, Number((summaryStats.fileCount ?? snapshot.touched_files.total_count ?? 0) || 0))
    : 0;
  const totalCount = available ? Math.max(files.length, badgeCount) : 0;
  const loading = available && touchedFilesState === "loading";
  const listReady =
    files.length > 0 ||
    totalCount === 0 ||
    !available ||
    touchedFilesState === "ready" ||
    touchedFilesState === "stale" ||
    touchedFilesState === "error";

  const sections = SECTION_ORDER.map((key) => {
    const sectionFiles = files.filter((file) => file.section === key);
    return {
      key,
      label: SECTION_LABELS[key],
      count: sectionFiles.length,
      files: sectionFiles,
    };
  }).filter((section) => section.count > 0);

  return {
    badgeCount,
    totalCount,
    available,
    unavailableReason,
    unavailableLabel,
    loading,
    computeError,
    listReady,
    sections,
  };
};
