use super::*;
use ctx_daemon::daemon::CoreHandle;

pub(in crate::api) async fn open_logs_folder(
    State(state): State<CoreHandle>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    state.open_logs_folder().await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: error.to_string(),
            }),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct DesktopLogReq {
    level: Option<String>,
    message: String,
}

pub(in crate::api) async fn append_desktop_log(
    State(state): State<CoreHandle>,
    Json(req): Json<DesktopLogReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    state
        .append_desktop_log(req.level, req.message)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: error.to_string(),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}
