use super::*;

pub(in crate::api) async fn open_logs_folder(
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    logs::open_logs_folder(&state.core.data_root)
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
    State(state): State<Arc<AppState>>,
    Json(req): Json<DesktopLogReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let level = req.level.unwrap_or_else(|| "info".to_string());
    let line = format!(
        "{} [{level}] {}",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        req.message
    );
    logs::append_desktop_log_line(&state.core.data_root, &line)
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
