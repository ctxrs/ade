use super::common::{parse_org_id, policy_api_error};
use super::*;

pub(in crate::api) async fn cache_org_policy_snapshot(
    State(state): State<CoreHandle>,
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
    state
        .cache_and_activate_org_policy_snapshot(snapshot)
        .await
        .map(Json)
        .map_err(cache_org_policy_snapshot_error)
}

fn cache_org_policy_snapshot_error(
    error: CacheOrgPolicySnapshotError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        CacheOrgPolicySnapshotError::EnrollmentMissing => {
            policy_api_error(StatusCode::CONFLICT, "daemon is not enrolled for this org")
        }
        CacheOrgPolicySnapshotError::EnrollmentLoad(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to load daemon enrollment: {error:#}"),
        ),
        CacheOrgPolicySnapshotError::InvalidSignature { message } => policy_api_error(
            StatusCode::BAD_REQUEST,
            format!("invalid policy snapshot signature: {message}"),
        ),
        CacheOrgPolicySnapshotError::SnapshotStore(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to cache policy snapshot: {error:#}"),
        ),
        CacheOrgPolicySnapshotError::EnrollmentActivation(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to activate policy snapshot: {error:#}"),
        ),
    }
}
