use super::*;
use ctx_daemon::daemon::CoreHandle;

pub(in crate::api) async fn open_logs_folder(
    State(state): State<CoreHandle>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    logs::open_logs_folder(state.data_root())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
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
    let level = req.level.unwrap_or_else(|| "info".to_string());
    let line = format!(
        "{} [{level}] {}",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        req.message
    );
    logs::append_desktop_log_line(state.data_root(), &line)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}
