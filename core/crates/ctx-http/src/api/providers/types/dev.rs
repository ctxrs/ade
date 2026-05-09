use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct DevRestartProvidersReq {
    pub(in crate::api::providers) mode: String,
    #[serde(default)]
    pub(in crate::api::providers) reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DevRestartProvidersResp {
    pub(in crate::api::providers) mode: String,
    pub(in crate::api::providers) results: Vec<DevRestartProvidersResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DevRestartProvidersResult {
    pub(in crate::api::providers) provider_id: String,
    pub(in crate::api::providers) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::api::providers) message: Option<String>,
}
