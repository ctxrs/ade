use serde::{Deserialize, Serialize};

use ctx_harness_sources::{HarnessApiShape, HarnessSourceKind};

#[derive(Debug, Deserialize)]
pub(crate) struct SelectHarnessSourceReq {
    pub(in crate::api::providers) source_kind: HarnessSourceKind,
    #[serde(default)]
    pub(in crate::api::providers) endpoint_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpsertHarnessEndpointReq {
    #[serde(default)]
    pub(in crate::api::providers) endpoint_id: Option<String>,
    pub(in crate::api::providers) name: String,
    #[serde(default)]
    pub(in crate::api::providers) base_url: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) api_shape: Option<HarnessApiShape>,
    #[serde(default)]
    pub(in crate::api::providers) auth_type: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) model_override: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) api_key: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) service_account_json: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) project_id: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) location: Option<String>,
    #[serde(default)]
    pub(in crate::api::providers) manual_model_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetEndpointManualModelsReq {
    #[serde(default)]
    pub(in crate::api::providers) model_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct MatrixRefreshResponse {
    pub(in crate::api::providers) provider_count: usize,
    pub(in crate::api::providers) generated_at: Option<String>,
    pub(in crate::api::providers) source: String,
    pub(in crate::api::providers) degraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::api::providers) last_error: Option<String>,
}
