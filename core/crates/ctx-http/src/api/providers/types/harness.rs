use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct MatrixRefreshResponse {
    pub(in crate::api::providers) provider_count: usize,
    pub(in crate::api::providers) generated_at: Option<String>,
    pub(in crate::api::providers) source: String,
    pub(in crate::api::providers) degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::api::providers) last_error: Option<String>,
}
