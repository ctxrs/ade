use super::common::{parse_org_id, policy_api_error};
use super::*;

pub(in crate::api) async fn cache_org_policy_snapshot(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
    Json(snapshot): Json<OrgPolicySnapshot>,
) -> Result<Json<OrgPolicySnapshot>, (StatusCode, Json<ApiErrorResp>)> {
    let org_id = parse_org_id(&org_id)?;
    if snapshot.org_id != org_id {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "policy snapshot org_id must match route org id",
        ));
    }
    let Some(mut enrollment) = state
        .global_store()
        .get_daemon_enrollment_by_org_id(org_id)
        .await
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to load daemon enrollment: {err:#}"),
            )
        })?
    else {
        return Err(policy_api_error(
            StatusCode::CONFLICT,
            "daemon is not enrolled for this org",
        ));
    };
    ctx_org_policy::signature::verify_policy_snapshot_signature(&enrollment, &snapshot).map_err(
        |err| {
            policy_api_error(
                StatusCode::BAD_REQUEST,
                format!("invalid policy snapshot signature: {err:#}"),
            )
        },
    )?;
    let stored = state
        .global_store()
        .upsert_org_policy_snapshot(snapshot)
        .await
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to cache policy snapshot: {err:#}"),
            )
        })?;
    enrollment.active_policy_snapshot_id = Some(stored.id);
    enrollment.updated_at = chrono::Utc::now();
    state
        .global_store()
        .upsert_daemon_enrollment(enrollment)
        .await
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to activate policy snapshot: {err:#}"),
            )
        })?;
    Ok(Json(stored))
}
