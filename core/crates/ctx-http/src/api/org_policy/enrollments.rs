use super::common::{parse_org_id, policy_api_error};
use super::*;

#[derive(Debug, Serialize)]
pub(in crate::api) struct DaemonEnrollmentResponse {
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

pub(in crate::api) async fn list_daemon_enrollments(
    State(state): State<CoreHandle>,
) -> Result<Json<Vec<DaemonEnrollmentResponse>>, (StatusCode, Json<ApiErrorResp>)> {
    state
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

pub(in crate::api) async fn upsert_daemon_enrollment(
    State(state): State<CoreHandle>,
    Path(org_id): Path<String>,
    Json(enrollment): Json<DaemonEnrollment>,
) -> Result<Json<DaemonEnrollmentResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let org_id = parse_org_id(&org_id)?;
    if enrollment.org_id != org_id {
        return Err(policy_api_error(
            StatusCode::BAD_REQUEST,
            "enrollment org_id must match route org id",
        ));
    }
    state
        .upsert_daemon_enrollment_checked(enrollment)
        .await
        .map(|enrollment| Json(DaemonEnrollmentResponse::from(enrollment)))
        .map_err(upsert_daemon_enrollment_error)
}

fn upsert_daemon_enrollment_error(
    error: UpsertDaemonEnrollmentError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        UpsertDaemonEnrollmentError::UnsupportedPlan => policy_api_error(
            StatusCode::BAD_REQUEST,
            "daemon enrollment requires a team or enterprise plan",
        ),
        UpsertDaemonEnrollmentError::MissingSigningKey => policy_api_error(
            StatusCode::BAD_REQUEST,
            "daemon enrollment requires a policy signing key",
        ),
        UpsertDaemonEnrollmentError::Store(error) => policy_api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to upsert daemon enrollment: {error:#}"),
        ),
    }
}
