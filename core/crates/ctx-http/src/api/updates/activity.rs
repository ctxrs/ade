use super::*;

#[derive(Debug, Serialize)]
pub(in crate::api) struct UpdateActivityResp {
    activity: crate::daemon::DaemonTurnActivitySummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    managed_daemon_auto_update: Option<ctx_update_service::ManagedDaemonAutoUpdateStatus>,
}

pub(in crate::api) async fn update_activity(
    State(state): State<Arc<AppState>>,
) -> Result<Json<UpdateActivityResp>, (StatusCode, Json<ApiErrorResp>)> {
    let activity = crate::daemon::daemon_turn_activity_summary(&state)
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            )
        })?;
    let managed_daemon_auto_update =
        ctx_update_service::managed_daemon_auto_update_status_snapshot(&state.core.data_root).await;
    Ok(Json(UpdateActivityResp {
        activity,
        managed_daemon_auto_update,
    }))
}
