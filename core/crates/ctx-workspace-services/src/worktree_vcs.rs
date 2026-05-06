use std::collections::HashSet;

use anyhow::Result;
use ctx_core::models::{
    WorktreeVcsComputeState, WorktreeVcsFreshness, WorktreeVcsGitStatusSummary,
    WorktreeVcsSnapshot, WorktreeVcsSummary, WorktreeVcsTouchedFile, WorktreeVcsTouchedFiles,
    WorktreeVcsTouchedFilesState,
};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSnapshot {
    pub raw: String,
    pub summary_line: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    pub entries: Vec<GitStatusEntry>,
    pub entries_total_count: i64,
    pub entries_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    pub index_status: String,
    pub worktree_status: String,
}

pub const WORKTREE_VCS_TOUCHED_FILES_CAP: usize = 200;
// Above this count, the product surfaces an exact summary but does not compute
// or stream file-by-file review inventory.
pub const WORKTREE_VCS_REVIEWABLE_FILE_LIMIT: i64 = 300;
pub const WORKTREE_VCS_SNAPSHOT_SCHEMA_VERSION: i64 = 2;

pub fn build_touched_files(entries: &[WorktreeVcsTouchedFile]) -> WorktreeVcsTouchedFiles {
    let total_count = entries.len() as i64;
    let truncated = entries.len() > WORKTREE_VCS_TOUCHED_FILES_CAP;
    let mut items = Vec::new();
    for entry in entries.iter().take(WORKTREE_VCS_TOUCHED_FILES_CAP) {
        items.push(entry.clone());
    }
    WorktreeVcsTouchedFiles {
        items,
        truncated,
        total_count: Some(total_count),
    }
}

pub fn build_large_change_set_touched_files(file_count: i64) -> WorktreeVcsTouchedFiles {
    WorktreeVcsTouchedFiles {
        items: Vec::new(),
        truncated: true,
        total_count: Some(file_count),
    }
}

pub fn build_git_status_entries(entries: &[GitStatusEntry]) -> Vec<WorktreeVcsTouchedFile> {
    let mut out = Vec::new();
    for entry in entries.iter().take(WORKTREE_VCS_TOUCHED_FILES_CAP) {
        out.push(WorktreeVcsTouchedFile {
            path: entry.path.clone(),
            orig_path: entry.orig_path.clone(),
            index_status: Some(entry.index_status.clone()),
            worktree_status: Some(entry.worktree_status.clone()),
        });
    }
    out
}

pub fn build_git_status_summary(
    snapshot: &GitStatusSnapshot,
    entries: Vec<WorktreeVcsTouchedFile>,
) -> WorktreeVcsGitStatusSummary {
    WorktreeVcsGitStatusSummary {
        raw: String::new(),
        summary_line: snapshot.summary_line.clone(),
        branch: snapshot.branch.clone(),
        upstream: snapshot.upstream.clone(),
        ahead: snapshot.ahead,
        behind: snapshot.behind,
        detached: snapshot.detached,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked: snapshot.untracked,
        entries,
    }
}

pub fn summary_from_file_count(file_count: i64) -> WorktreeVcsSummary {
    WorktreeVcsSummary {
        file_count: Some(file_count),
        line_additions: None,
        line_deletions: None,
        line_count: None,
    }
}

pub fn snapshot_for_durable_cache(snapshot: &WorktreeVcsSnapshot) -> WorktreeVcsSnapshot {
    let mut durable = snapshot.clone();
    durable.touched_files = WorktreeVcsTouchedFiles::default();
    durable.touched_files_state = WorktreeVcsTouchedFilesState::NotLoaded;
    durable.git_status.raw.clear();
    durable.git_status.entries.clear();
    durable
}

pub fn summary_has_counts(summary: &WorktreeVcsSummary) -> bool {
    summary.file_count.is_some()
        || summary.line_additions.is_some()
        || summary.line_deletions.is_some()
        || summary.line_count.is_some()
}

pub fn derive_worktree_vcs_freshness(
    compute_state: &WorktreeVcsComputeState,
    summary: &WorktreeVcsSummary,
) -> WorktreeVcsFreshness {
    match compute_state {
        WorktreeVcsComputeState::Ready => WorktreeVcsFreshness::Fresh,
        WorktreeVcsComputeState::Error => WorktreeVcsFreshness::Error,
        WorktreeVcsComputeState::Computing => {
            if summary_has_counts(summary) {
                WorktreeVcsFreshness::Stale
            } else {
                WorktreeVcsFreshness::Refreshing
            }
        }
    }
}

pub fn count_diff_paths(
    entries: Vec<(String, String, Option<String>)>,
    untracked: Vec<String>,
) -> Result<i64> {
    let mut seen = HashSet::new();
    for (status, path, _) in entries {
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.to_string()) {
            continue;
        }
        if status.chars().next().is_none() {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        }
    }
    for path in untracked {
        let path = path.trim();
        if !path.is_empty() {
            seen.insert(path.to_string());
        }
    }
    Ok(seen.len() as i64)
}

pub fn build_diff_path_states(
    entries: Vec<(String, String, Option<String>)>,
    untracked: Vec<String>,
) -> Result<Vec<(String, Option<String>, String)>> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (status, path, orig_path) in entries {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(status_kind) = status.chars().next() else {
            anyhow::bail!("vcs diff returned an empty status for {path}");
        };
        out.push((path, orig_path, status_kind.to_string()));
    }
    for path in untracked {
        let path = path.trim().to_string();
        if path.is_empty() {
            continue;
        }
        if !seen.insert(path.clone()) {
            continue;
        }
        out.push((path, None, "?".to_string()));
    }
    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_diff_path_states_deduplicates_untracked_paths_already_in_diff() -> Result<()> {
        let out = build_diff_path_states(
            vec![("D".to_string(), "src/example.rs".to_string(), None)],
            vec!["src/example.rs".to_string()],
        )?;

        assert_eq!(
            out,
            vec![("src/example.rs".to_string(), None, "D".to_string())]
        );
        Ok(())
    }

    #[test]
    fn count_diff_paths_deduplicates_untracked_paths_already_in_diff() -> Result<()> {
        let count = count_diff_paths(
            vec![("D".to_string(), "src/example.rs".to_string(), None)],
            vec!["src/example.rs".to_string()],
        )?;

        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn large_change_set_touched_files_stays_truthful_without_rows() {
        let touched_files =
            build_large_change_set_touched_files(WORKTREE_VCS_REVIEWABLE_FILE_LIMIT + 1);

        assert!(touched_files.items.is_empty());
        assert!(touched_files.truncated);
        assert_eq!(
            touched_files.total_count,
            Some(WORKTREE_VCS_REVIEWABLE_FILE_LIMIT + 1)
        );
    }
}
