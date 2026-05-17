use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct MergeQueueListParams {
    pub(in crate::api) workspace_id: String,
    #[serde(default)]
    pub(in crate::api) limit: Option<i64>,
}
