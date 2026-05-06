use ctx_core::models::{
    DiffUnavailableReason, WorktreeVcsBaseResolutionKind, WorktreeVcsTargetSource,
};

pub fn is_no_vcs_repo_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_lowercase();
    lower.contains("no vcs repo found")
        || lower.contains("not a git repository")
        || lower.contains("is not a git repo")
        || lower.contains("not inside a jj repo")
}

#[derive(Clone)]
pub struct WorktreeDiffBaseResolution {
    pub base_commit_sha: String,
    pub head_commit_sha: Option<String>,
    pub target_branch: Option<String>,
    pub target_branch_commit_sha: Option<String>,
    pub target_source: Option<WorktreeVcsTargetSource>,
    pub kind: WorktreeVcsBaseResolutionKind,
    pub error: Option<String>,
    pub unavailable_reason: Option<DiffUnavailableReason>,
    pub explicit_target: bool,
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn no_vcs_repo_classifier_matches_supported_vcs_messages() {
        for message in [
            "no vcs repo found",
            "fatal: not a git repository",
            "path is not a git repo",
            "not inside a jj repo",
        ] {
            assert!(is_no_vcs_repo_error(&anyhow!(message)));
        }
    }

    #[test]
    fn no_vcs_repo_classifier_rejects_unrelated_errors() {
        assert!(!is_no_vcs_repo_error(&anyhow!("permission denied")));
    }
}
