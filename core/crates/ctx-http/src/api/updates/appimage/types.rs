use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(in crate::api) struct DownloadAppImageReq {
    #[serde(default)]
    pub(super) channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct DownloadAppImageResp {
    pub(super) downloaded_path: String,
    pub(super) can_apply_in_place: bool,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ApplyAppImageReq {
    pub(super) confirm: bool,
    #[serde(default)]
    pub(super) channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct ApplyAppImageResp {
    pub(super) applied: bool,
    pub(super) target_path: Option<String>,
    pub(super) message: String,
}
