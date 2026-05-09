use super::common::{parse_workspace_id, policy_api_error};
use super::*;

pub(in crate::api) async fn get_workspace_org_policy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Option<WorkspacePolicyOverlay>>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|err| {
            policy_api_error(
                StatusCode::NOT_FOUND,
                format!("workspace not found for org policy: {err:#}"),
            )
        })?;
    store
        .get_workspace_policy_overlay(workspace_id)
        .await
        .map(Json)
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load workspace org policy: {err:#}"),
            )
        })
}

pub(in crate::api) async fn upsert_workspace_org_policy(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(overlay): Json<WorkspacePolicyOverlay>,
) -> Result<Json<WorkspacePolicyOverlay>, (StatusCode, Json<ApiErrorResp>)> {
    let workspace_id = parse_workspace_id(&id)?;
    if overlay.workspace_id != workspace_id {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "workspace policy overlay workspace_id must match route workspace id",
        ));
    }
    let enrollment = state
        .global_store()
        .get_daemon_enrollment_by_org_id(overlay.org_id)
        .await
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load daemon enrollment: {err:#}"),
            )
        })?;
    if enrollment.is_none() {
        return Err(policy_api_error(
            StatusCode::CONFLICT,
            "daemon is not enrolled for this org",
        ));
    }
    let store = state
        .store_for_workspace(workspace_id)
        .await
        .map_err(|err| {
            policy_api_error(
                StatusCode::NOT_FOUND,
                format!("workspace not found for org policy: {err:#}"),
            )
        })?;
    store
        .upsert_workspace_policy_overlay(overlay)
        .await
        .map(Json)
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to upsert workspace org policy: {err:#}"),
            )
        })
}
