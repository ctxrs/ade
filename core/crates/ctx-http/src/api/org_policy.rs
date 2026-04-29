use super::*;

fn policy_api_error(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn parse_org_id(raw: &str) -> Result<OrgId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw)
        .map(OrgId)
        .map_err(|_| policy_api_error(StatusCode::BAD_REQUEST, "invalid org id"))
}

fn parse_workspace_id(raw: &str) -> Result<WorkspaceId, (StatusCode, Json<ApiErrorResp>)> {
    uuid::Uuid::parse_str(raw)
        .map(WorkspaceId)
        .map_err(|_| policy_api_error(StatusCode::BAD_REQUEST, "invalid workspace id"))
}

#[derive(Debug, Serialize)]
pub(super) struct DaemonEnrollmentResponse {
    id: DaemonEnrollmentId,
    account_id: AccountId,
    org_id: OrgId,
    org_membership_id: OrgMembershipId,
    membership_role: OrgMembershipRole,
    plan_type: PlanType,
    status: DaemonEnrollmentStatus,
    policy_signature_algorithm: PolicySignatureAlgorithm,
    policy_signing_key_present: bool,
    active_policy_snapshot_id: Option<OrgPolicySnapshotId>,
    enrolled_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<DaemonEnrollment> for DaemonEnrollmentResponse {
    fn from(enrollment: DaemonEnrollment) -> Self {
        Self {
            id: enrollment.id,
            account_id: enrollment.account_id,
            org_id: enrollment.org_id,
            org_membership_id: enrollment.org_membership_id,
            membership_role: enrollment.membership_role,
            plan_type: enrollment.plan_type,
            status: enrollment.status,
            policy_signature_algorithm: enrollment.policy_signature_algorithm,
            policy_signing_key_present: !enrollment.policy_signing_key.trim().is_empty(),
            active_policy_snapshot_id: enrollment.active_policy_snapshot_id,
            enrolled_at: enrollment.enrolled_at,
            updated_at: enrollment.updated_at,
            revoked_at: enrollment.revoked_at,
        }
    }
}

pub(super) async fn list_daemon_enrollments(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DaemonEnrollmentResponse>>, (StatusCode, Json<ApiErrorResp>)> {
    state
        .global_store()
        .list_daemon_enrollments()
        .await
        .map(|enrollments| {
            Json(
                enrollments
                    .into_iter()
                    .map(DaemonEnrollmentResponse::from)
                    .collect(),
            )
        })
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to list daemon enrollments: {err:#}"),
            )
        })
}

pub(super) async fn upsert_daemon_enrollment(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
    Json(mut enrollment): Json<DaemonEnrollment>,
) -> Result<Json<DaemonEnrollmentResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let org_id = parse_org_id(&org_id)?;
    if enrollment.org_id != org_id {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "enrollment org_id must match route org id",
        ));
    }
    if !matches!(enrollment.plan_type, PlanType::Team | PlanType::Enterprise) {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "daemon enrollment requires a team or enterprise plan",
        ));
    }
    if enrollment.policy_signing_key.trim().is_empty() {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "daemon enrollment requires a policy signing key",
        ));
    }
    enrollment.updated_at = chrono::Utc::now();
    state
        .global_store()
        .upsert_daemon_enrollment(enrollment)
        .await
        .map(|enrollment| Json(DaemonEnrollmentResponse::from(enrollment)))
        .map_err(|err| {
            policy_api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to upsert daemon enrollment: {err:#}"),
            )
        })
}

pub(super) async fn cache_org_policy_snapshot(
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
    crate::policy_signature::verify_policy_snapshot_signature(&enrollment, &snapshot).map_err(
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

pub(super) async fn get_workspace_org_policy(
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

pub(super) async fn upsert_workspace_org_policy(
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
