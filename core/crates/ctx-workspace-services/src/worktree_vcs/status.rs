use anyhow::Result;

use super::{GitStatusEntry, GitStatusSnapshot};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeVcsStructuredStatus {
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

#[async_trait::async_trait]
pub trait WorktreeVcsStatusSource: Send + Sync {
    async fn has_vcs_repo(&self) -> Result<bool>;

    async fn load_structured_status(
        &self,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> Result<WorktreeVcsStructuredStatus>;
}

pub async fn worktree_has_vcs_repo_from_source(
    source: &impl WorktreeVcsStatusSource,
) -> Result<bool> {
    source.has_vcs_repo().await
}

pub async fn load_git_status_snapshot_from_source(
    source: &impl WorktreeVcsStatusSource,
    include_untracked_files: bool,
    include_entries: bool,
) -> Result<GitStatusSnapshot> {
    let structured = source
        .load_structured_status(include_untracked_files, include_entries)
        .await?;
    Ok(git_status_snapshot_from_structured(
        structured,
        include_entries,
    ))
}

pub fn git_status_snapshot_from_structured(
    structured: WorktreeVcsStructuredStatus,
    include_entries: bool,
) -> GitStatusSnapshot {
    GitStatusSnapshot {
        raw: structured.raw,
        summary_line: structured.summary_line,
        branch: structured.branch,
        upstream: structured.upstream,
        ahead: structured.ahead,
        behind: structured.behind,
        detached: structured.detached,
        staged: structured.staged,
        unstaged: structured.unstaged,
        untracked: structured.untracked,
        entries: if include_entries {
            structured.entries
        } else {
            Vec::new()
        },
        entries_total_count: structured.entries_total_count,
        entries_truncated: structured.entries_truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn structured_status() -> WorktreeVcsStructuredStatus {
        WorktreeVcsStructuredStatus {
            raw: "## main...origin/main [ahead 1]".to_string(),
            summary_line: "## main...origin/main [ahead 1]".to_string(),
            branch: Some("main".to_string()),
            upstream: Some("origin/main".to_string()),
            ahead: 1,
            behind: 0,
            detached: false,
            staged: 1,
            unstaged: 2,
            untracked: 3,
            entries: vec![GitStatusEntry {
                path: "src/lib.rs".to_string(),
                orig_path: None,
                index_status: "M".to_string(),
                worktree_status: ".".to_string(),
            }],
            entries_total_count: 1,
            entries_truncated: false,
        }
    }

    #[test]
    fn status_snapshot_conversion_preserves_counts_and_entries() {
        let snapshot = git_status_snapshot_from_structured(structured_status(), true);

        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.upstream.as_deref(), Some("origin/main"));
        assert_eq!(snapshot.ahead, 1);
        assert_eq!(snapshot.staged, 1);
        assert_eq!(snapshot.unstaged, 2);
        assert_eq!(snapshot.untracked, 3);
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries_total_count, 1);
    }

    #[test]
    fn status_snapshot_conversion_can_drop_entries() {
        let snapshot = git_status_snapshot_from_structured(structured_status(), false);

        assert!(snapshot.entries.is_empty());
        assert_eq!(snapshot.entries_total_count, 1);
    }
}
