use anyhow::Result;
use ctx_core::models::SessionGitStatusSummary;

use super::{GitStatusEntry, GitStatusSnapshot, WorktreeVcsCommitLookup};

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

#[async_trait::async_trait]
pub trait WorktreeVcsCommitLookupSource: Send + Sync {
    async fn resolve_commit(&self, reference: &str) -> Result<String>;
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

pub async fn resolve_worktree_vcs_commit_lookup_from_source(
    source: &impl WorktreeVcsCommitLookupSource,
    lookup: &WorktreeVcsCommitLookup,
) -> Result<Option<String>> {
    match lookup {
        WorktreeVcsCommitLookup::Resolved(commit) => Ok(Some(commit.clone())),
        WorktreeVcsCommitLookup::Missing => Ok(None),
        WorktreeVcsCommitLookup::Head => Ok(Some(source.resolve_commit("HEAD").await?)),
        WorktreeVcsCommitLookup::TargetBranch(target_branch) => {
            Ok(Some(source.resolve_commit(target_branch).await?))
        }
    }
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

pub fn session_git_status_summary_from_snapshot(
    snapshot: &GitStatusSnapshot,
) -> SessionGitStatusSummary {
    SessionGitStatusSummary {
        summary_line: snapshot.summary_line.clone(),
        branch: snapshot.branch.clone(),
        upstream: snapshot.upstream.clone(),
        ahead: snapshot.ahead,
        behind: snapshot.behind,
        detached: snapshot.detached,
        staged: snapshot.staged,
        unstaged: snapshot.unstaged,
        untracked: snapshot.untracked,
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

    #[test]
    fn session_git_status_summary_projection_preserves_route_persisted_fields() {
        let snapshot = git_status_snapshot_from_structured(structured_status(), true);
        let summary = session_git_status_summary_from_snapshot(&snapshot);

        assert_eq!(summary.summary_line, "## main...origin/main [ahead 1]");
        assert_eq!(summary.branch.as_deref(), Some("main"));
        assert_eq!(summary.upstream.as_deref(), Some("origin/main"));
        assert_eq!(summary.ahead, 1);
        assert_eq!(summary.behind, 0);
        assert!(!summary.detached);
        assert_eq!(summary.staged, 1);
        assert_eq!(summary.unstaged, 2);
        assert_eq!(summary.untracked, 3);
    }

    struct FakeCommitLookupSource;

    #[async_trait::async_trait]
    impl WorktreeVcsCommitLookupSource for FakeCommitLookupSource {
        async fn resolve_commit(&self, reference: &str) -> Result<String> {
            Ok(format!("resolved-{reference}"))
        }
    }

    #[tokio::test]
    async fn commit_lookup_source_resolves_only_live_requests() {
        let source = FakeCommitLookupSource;

        assert_eq!(
            resolve_worktree_vcs_commit_lookup_from_source(
                &source,
                &WorktreeVcsCommitLookup::Resolved("known".to_string())
            )
            .await
            .unwrap()
            .as_deref(),
            Some("known")
        );
        assert_eq!(
            resolve_worktree_vcs_commit_lookup_from_source(&source, &WorktreeVcsCommitLookup::Head)
                .await
                .unwrap()
                .as_deref(),
            Some("resolved-HEAD")
        );
        assert_eq!(
            resolve_worktree_vcs_commit_lookup_from_source(
                &source,
                &WorktreeVcsCommitLookup::TargetBranch("main".to_string())
            )
            .await
            .unwrap()
            .as_deref(),
            Some("resolved-main")
        );
        assert!(resolve_worktree_vcs_commit_lookup_from_source(
            &source,
            &WorktreeVcsCommitLookup::Missing
        )
        .await
        .unwrap()
        .is_none());
    }
}
