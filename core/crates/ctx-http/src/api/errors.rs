use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct ApiErrorResp {
    pub(super) error: String,
}
