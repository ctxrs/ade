use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct MergeQueueSubmitReq {
    #[serde(default)]
    pub(in crate::api) session_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) worktree_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) worktree_root: Option<String>,
    #[serde(default)]
    pub(in crate::api) target_branch: Option<String>,
    #[serde(default)]
    pub(in crate::api) message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct MergeQueueListParams {
    pub(in crate::api) workspace_id: String,
    #[serde(default)]
    pub(in crate::api) limit: Option<i64>,
}
