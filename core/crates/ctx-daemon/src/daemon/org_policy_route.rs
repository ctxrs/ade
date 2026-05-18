use ctx_core::ids::{OrgId, WorkspaceId};
use ctx_core::models::{
    DaemonEnrollment, DaemonEnrollmentStatus, OrgMembershipRole, OrgPolicySnapshot, PlanType,
    PolicySignatureAlgorithm, WorkspacePolicyOverlay,
};
use serde::{Deserialize, Serialize};

use crate::daemon::org_policy::{
    CacheOrgPolicySnapshotError, UpsertDaemonEnrollmentError, UpsertWorkspacePolicyOverlayError,
    WorkspacePolicyOverlayError,
};
use crate::daemon::{CoreHandle, WorkspacesHandle};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrgPolicyOrgRouteParams {
    org_id: String,
}

impl OrgPolicyOrgRouteParams {
    pub fn new(org_id: impl Into<String>) -> Self {
        Self {
            org_id: org_id.into(),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrgPolicyWorkspaceRouteParams {
    workspace_id: String,
}

impl OrgPolicyWorkspaceRouteParams {
    pub fn new(workspace_id: impl Into<String>) -> Self {
        Self {
            workspace_id: workspace_id.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct UpsertDaemonEnrollmentRouteRequest(DaemonEnrollment);

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct CacheOrgPolicySnapshotRouteRequest(OrgPolicySnapshot);

#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct UpsertWorkspacePolicyOverlayRouteRequest(WorkspacePolicyOverlay);

#[derive(Debug, Clone, Serialize)]
pub struct DaemonEnrollmentRouteResponse {
    id: ctx_core::ids::DaemonEnrollmentId,
    account_id: ctx_core::ids::AccountId,
    org_id: OrgId,
    org_membership_id: ctx_core::ids::OrgMembershipId,
    membership_role: OrgMembershipRole,
    plan_type: PlanType,
    status: DaemonEnrollmentStatus,
    policy_signature_algorithm: PolicySignatureAlgorithm,
    policy_signing_key_present: bool,
    active_policy_snapshot_id: Option<ctx_core::ids::OrgPolicySnapshotId>,
    enrolled_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<DaemonEnrollment> for DaemonEnrollmentRouteResponse {
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

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct DaemonEnrollmentsRouteResponse(Vec<DaemonEnrollmentRouteResponse>);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct OrgPolicySnapshotRouteResponse(OrgPolicySnapshot);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct WorkspacePolicyOverlayRouteResponse(WorkspacePolicyOverlay);

#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct WorkspacePolicyOverlayOptionalRouteResponse(Option<WorkspacePolicyOverlay>);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OrgPolicyRouteErrorKind {
    BadRequest,
    Conflict,
    NotFound,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct OrgPolicyRouteError {
    kind: OrgPolicyRouteErrorKind,
    message: String,
}

impl OrgPolicyRouteError {
    fn new(kind: OrgPolicyRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(OrgPolicyRouteErrorKind::BadRequest, message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(OrgPolicyRouteErrorKind::Conflict, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(OrgPolicyRouteErrorKind::NotFound, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(OrgPolicyRouteErrorKind::Internal, message)
    }

    pub fn kind(&self) -> OrgPolicyRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

fn parse_org_route_id(params: OrgPolicyOrgRouteParams) -> Result<OrgId, OrgPolicyRouteError> {
    uuid::Uuid::parse_str(&params.org_id)
        .map(OrgId)
        .map_err(|_| OrgPolicyRouteError::bad_request("invalid org id"))
}

fn parse_workspace_route_id(
    params: OrgPolicyWorkspaceRouteParams,
) -> Result<WorkspaceId, OrgPolicyRouteError> {
    uuid::Uuid::parse_str(&params.workspace_id)
        .map(WorkspaceId)
        .map_err(|_| OrgPolicyRouteError::bad_request("invalid workspace id"))
}

fn upsert_daemon_enrollment_route_error(error: UpsertDaemonEnrollmentError) -> OrgPolicyRouteError {
    match error {
        UpsertDaemonEnrollmentError::UnsupportedPlan => {
            OrgPolicyRouteError::bad_request("daemon enrollment requires a team or enterprise plan")
        }
        UpsertDaemonEnrollmentError::MissingSigningKey => {
            OrgPolicyRouteError::bad_request("daemon enrollment requires a policy signing key")
        }
        UpsertDaemonEnrollmentError::Store(error) => {
            OrgPolicyRouteError::internal(format!("failed to upsert daemon enrollment: {error:#}"))
        }
    }
}

fn cache_org_policy_snapshot_route_error(
    error: CacheOrgPolicySnapshotError,
) -> OrgPolicyRouteError {
    match error {
        CacheOrgPolicySnapshotError::EnrollmentMissing => {
            OrgPolicyRouteError::conflict("daemon is not enrolled for this org")
        }
        CacheOrgPolicySnapshotError::EnrollmentLoad(error) => {
            OrgPolicyRouteError::internal(format!("failed to load daemon enrollment: {error:#}"))
        }
        CacheOrgPolicySnapshotError::InvalidSignature { message } => {
            OrgPolicyRouteError::bad_request(format!(
                "invalid policy snapshot signature: {message}"
            ))
        }
        CacheOrgPolicySnapshotError::SnapshotStore(error) => {
            OrgPolicyRouteError::internal(format!("failed to cache policy snapshot: {error:#}"))
        }
        CacheOrgPolicySnapshotError::EnrollmentActivation(error) => {
            OrgPolicyRouteError::internal(format!("failed to activate policy snapshot: {error:#}"))
        }
    }
}

fn workspace_policy_route_error(
    error: WorkspacePolicyOverlayError,
    message: &'static str,
) -> OrgPolicyRouteError {
    match error {
        WorkspacePolicyOverlayError::WorkspaceNotFound => {
            OrgPolicyRouteError::not_found("workspace not found for org policy")
        }
        WorkspacePolicyOverlayError::Store(error) => {
            OrgPolicyRouteError::internal(format!("{message}: {error:#}"))
        }
    }
}

fn upsert_workspace_policy_route_error(
    error: UpsertWorkspacePolicyOverlayError,
) -> OrgPolicyRouteError {
    match error {
        UpsertWorkspacePolicyOverlayError::EnrollmentMissing => {
            OrgPolicyRouteError::conflict("daemon is not enrolled for this org")
        }
        UpsertWorkspacePolicyOverlayError::EnrollmentLoad(error) => {
            OrgPolicyRouteError::internal(format!("failed to load daemon enrollment: {error:#}"))
        }
        UpsertWorkspacePolicyOverlayError::WorkspaceNotFound => {
            OrgPolicyRouteError::not_found("workspace not found for org policy")
        }
        UpsertWorkspacePolicyOverlayError::Store(error) => OrgPolicyRouteError::internal(format!(
            "failed to upsert workspace org policy: {error:#}"
        )),
    }
}

impl CoreHandle {
    pub async fn list_daemon_enrollments_for_route(
        &self,
    ) -> Result<DaemonEnrollmentsRouteResponse, OrgPolicyRouteError> {
        self.list_daemon_enrollments()
            .await
            .map(|enrollments| {
                DaemonEnrollmentsRouteResponse(
                    enrollments
                        .into_iter()
                        .map(DaemonEnrollmentRouteResponse::from)
                        .collect(),
                )
            })
            .map_err(|error| {
                OrgPolicyRouteError::internal(format!(
                    "failed to list daemon enrollments: {error:#}"
                ))
            })
    }

    pub async fn upsert_daemon_enrollment_for_route(
        &self,
        params: OrgPolicyOrgRouteParams,
        request: UpsertDaemonEnrollmentRouteRequest,
    ) -> Result<DaemonEnrollmentRouteResponse, OrgPolicyRouteError> {
        let org_id = parse_org_route_id(params)?;
        let enrollment = request.0;
        if enrollment.org_id != org_id {
            return Err(OrgPolicyRouteError::bad_request(
                "enrollment org_id must match route org id",
            ));
        }
        self.upsert_daemon_enrollment_checked(enrollment)
            .await
            .map(DaemonEnrollmentRouteResponse::from)
            .map_err(upsert_daemon_enrollment_route_error)
    }

    pub async fn cache_org_policy_snapshot_for_route(
        &self,
        params: OrgPolicyOrgRouteParams,
        request: CacheOrgPolicySnapshotRouteRequest,
    ) -> Result<OrgPolicySnapshotRouteResponse, OrgPolicyRouteError> {
        let org_id = parse_org_route_id(params)?;
        let snapshot = request.0;
        if snapshot.org_id != org_id {
            return Err(OrgPolicyRouteError::bad_request(
                "policy snapshot org_id must match route org id",
            ));
        }
        self.cache_and_activate_org_policy_snapshot(snapshot)
            .await
            .map(OrgPolicySnapshotRouteResponse)
            .map_err(cache_org_policy_snapshot_route_error)
    }
}

impl WorkspacesHandle {
    pub async fn get_workspace_policy_overlay_for_route(
        &self,
        params: OrgPolicyWorkspaceRouteParams,
    ) -> Result<WorkspacePolicyOverlayOptionalRouteResponse, OrgPolicyRouteError> {
        let workspace_id = parse_workspace_route_id(params)?;
        self.get_workspace_policy_overlay(workspace_id)
            .await
            .map(WorkspacePolicyOverlayOptionalRouteResponse)
            .map_err(|error| {
                workspace_policy_route_error(error, "failed to load workspace org policy")
            })
    }

    pub async fn upsert_workspace_policy_overlay_for_route(
        &self,
        params: OrgPolicyWorkspaceRouteParams,
        request: UpsertWorkspacePolicyOverlayRouteRequest,
    ) -> Result<WorkspacePolicyOverlayRouteResponse, OrgPolicyRouteError> {
        let workspace_id = parse_workspace_route_id(params)?;
        let overlay = request.0;
        if overlay.workspace_id != workspace_id {
            return Err(OrgPolicyRouteError::bad_request(
                "workspace policy overlay workspace_id must match route workspace id",
            ));
        }
        self.upsert_workspace_policy_overlay_checked(overlay)
            .await
            .map(WorkspacePolicyOverlayRouteResponse)
            .map_err(upsert_workspace_policy_route_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use ctx_core::ids::{AccountId, DaemonEnrollmentId, OrgMembershipId, OrgPolicySnapshotId};
    use ctx_core::models::{
        ArchiveMode, ArchivePolicy, DaemonEnrollmentStatus, NetworkProfile, OrgMembershipRole,
        PolicyFeatureState, PolicySignatureAlgorithm, RoutePolicy, RouteType, VcsKind,
    };
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    use crate::test_support::TestDaemon;

    async fn test_daemon() -> (tempfile::TempDir, TestDaemon) {
        let temp = tempdir().expect("tempdir");
        let daemon = TestDaemon::new_for_test(
            temp.path().to_path_buf(),
            "http://127.0.0.1:4567".to_string(),
        )
        .await
        .expect("test daemon");
        (temp, daemon)
    }

    fn enrollment(org_id: OrgId) -> DaemonEnrollment {
        let now = Utc::now();
        DaemonEnrollment {
            id: DaemonEnrollmentId::new(),
            account_id: AccountId::new(),
            org_id,
            org_membership_id: OrgMembershipId::new(),
            membership_role: OrgMembershipRole::Owner,
            plan_type: PlanType::Team,
            status: DaemonEnrollmentStatus::Active,
            policy_signature_algorithm: PolicySignatureAlgorithm::Hs256,
            policy_signing_key: "policy-signing-secret".to_string(),
            active_policy_snapshot_id: None,
            enrolled_at: now,
            updated_at: now,
            revoked_at: None,
        }
    }

    fn snapshot(org_id: OrgId) -> OrgPolicySnapshot {
        let now = Utc::now();
        OrgPolicySnapshot {
            id: OrgPolicySnapshotId::new(),
            org_id,
            policy_version: "2026-05-16.1".to_string(),
            issued_at: now,
            expires_at: now + Duration::minutes(30),
            grace_expires_at: now + Duration::minutes(60),
            allowed_providers: Some(vec!["fake".to_string()]),
            allowed_models: BTreeMap::new(),
            required_execution_environment: None,
            allowed_network_profiles: vec![NetworkProfile::LlmOnly],
            route_policy: RoutePolicy {
                allowed_route_types: vec![RouteType::UserProviderAccount],
            },
            archive_policy: ArchivePolicy {
                mode: ArchiveMode::OrgSummary,
            },
            features: BTreeMap::from([("org_policy".to_string(), PolicyFeatureState::Enabled)]),
            signature: String::new(),
        }
    }

    fn sign_snapshot(enrollment: &DaemonEnrollment, snapshot: &OrgPolicySnapshot) -> String {
        let claims = serde_json::json!({
            "aud": "ctx.org_policy_snapshot",
            "exp": (Utc::now() + Duration::minutes(60)).timestamp(),
            "org_id": snapshot.org_id.0.to_string(),
            "policy_version": snapshot.policy_version,
            "snapshot_sha256": ctx_org_policy::signature::policy_snapshot_digest_hex(snapshot)
                .expect("snapshot digest"),
        });
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(enrollment.policy_signing_key.as_bytes()),
        )
        .expect("sign snapshot")
    }

    fn overlay(workspace_id: WorkspaceId, org_id: OrgId) -> WorkspacePolicyOverlay {
        WorkspacePolicyOverlay {
            workspace_id,
            org_id,
            allowed_providers: Some(vec!["fake".to_string()]),
            allowed_models: BTreeMap::new(),
            required_execution_environment: None,
            allowed_network_profiles: None,
            allowed_route_types: None,
            features: BTreeMap::new(),
        }
    }

    #[test]
    fn route_params_parse_ids_and_classify_invalid_values() {
        let org_id = OrgId::new();
        assert_eq!(
            parse_org_route_id(OrgPolicyOrgRouteParams::new(org_id.0.to_string())).unwrap(),
            org_id
        );
        let org_error = parse_org_route_id(OrgPolicyOrgRouteParams::new("not-an-org")).unwrap_err();
        assert_eq!(org_error.kind(), OrgPolicyRouteErrorKind::BadRequest);
        assert_eq!(org_error.message(), "invalid org id");

        let workspace_id = WorkspaceId::new();
        assert_eq!(
            parse_workspace_route_id(OrgPolicyWorkspaceRouteParams::new(
                workspace_id.0.to_string()
            ))
            .unwrap(),
            workspace_id
        );
        let workspace_error =
            parse_workspace_route_id(OrgPolicyWorkspaceRouteParams::new("not-a-workspace"))
                .unwrap_err();
        assert_eq!(workspace_error.kind(), OrgPolicyRouteErrorKind::BadRequest);
        assert_eq!(workspace_error.message(), "invalid workspace id");
    }

    #[test]
    fn route_request_wrappers_preserve_unknown_field_compatibility() {
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();

        let enrollment_value = serde_json::to_value(enrollment(org_id)).expect("enrollment json");
        let mut enrollment_object = enrollment_value.as_object().expect("object").clone();
        enrollment_object.insert("unknown_field".to_string(), json!("ignored"));
        serde_json::from_value::<UpsertDaemonEnrollmentRouteRequest>(json!(enrollment_object))
            .expect("enrollment route request allows unknown fields");

        let snapshot_value = serde_json::to_value(snapshot(org_id)).expect("snapshot json");
        let mut snapshot_object = snapshot_value.as_object().expect("object").clone();
        snapshot_object.insert("unknown_field".to_string(), json!("ignored"));
        serde_json::from_value::<CacheOrgPolicySnapshotRouteRequest>(json!(snapshot_object))
            .expect("snapshot route request allows unknown fields");

        let overlay_value =
            serde_json::to_value(overlay(workspace_id, org_id)).expect("overlay json");
        let mut overlay_object = overlay_value.as_object().expect("object").clone();
        overlay_object.insert("unknown_field".to_string(), json!("ignored"));
        serde_json::from_value::<UpsertWorkspacePolicyOverlayRouteRequest>(json!(overlay_object))
            .expect("overlay route request allows unknown fields");
    }

    #[test]
    fn enrollment_route_response_redacts_signing_key() {
        let response = DaemonEnrollmentRouteResponse::from(enrollment(OrgId::new()));
        let value = serde_json::to_value(response).expect("response json");
        assert!(value.get("policy_signing_key").is_none());
        assert_eq!(
            value
                .get("policy_signing_key_present")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn route_response_wrappers_preserve_json_shapes() {
        let org_id = OrgId::new();
        let enrollment_response = DaemonEnrollmentRouteResponse::from(enrollment(org_id));
        let list = DaemonEnrollmentsRouteResponse(vec![enrollment_response]);
        assert!(serde_json::to_value(list).unwrap().is_array());

        let snapshot_value =
            serde_json::to_value(OrgPolicySnapshotRouteResponse(snapshot(org_id))).unwrap();
        let org_id_text = org_id.0.to_string();
        assert_eq!(
            snapshot_value
                .get("org_id")
                .and_then(|value| value.as_str()),
            Some(org_id_text.as_str())
        );

        let workspace_id = WorkspaceId::new();
        let overlay_value = serde_json::to_value(WorkspacePolicyOverlayRouteResponse(overlay(
            workspace_id,
            org_id,
        )))
        .unwrap();
        let workspace_id_text = workspace_id.0.to_string();
        assert_eq!(
            overlay_value
                .get("workspace_id")
                .and_then(|value| value.as_str()),
            Some(workspace_id_text.as_str())
        );

        assert_eq!(
            serde_json::to_value(WorkspacePolicyOverlayOptionalRouteResponse(None)).unwrap(),
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn enrollment_route_checks_route_body_mismatch_first() {
        let (_temp, daemon) = test_daemon().await;
        let route_org_id = OrgId::new();
        let mut request = enrollment(OrgId::new());
        request.plan_type = PlanType::Pro;

        let error = daemon
            .handle()
            .core()
            .upsert_daemon_enrollment_for_route(
                OrgPolicyOrgRouteParams::new(route_org_id.0.to_string()),
                UpsertDaemonEnrollmentRouteRequest(request),
            )
            .await
            .expect_err("route/body mismatch should fail first");

        assert_eq!(error.kind(), OrgPolicyRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "enrollment org_id must match route org id");
    }

    #[tokio::test]
    async fn snapshot_route_checks_route_body_mismatch_first() {
        let (_temp, daemon) = test_daemon().await;
        let error = daemon
            .handle()
            .core()
            .cache_org_policy_snapshot_for_route(
                OrgPolicyOrgRouteParams::new(OrgId::new().0.to_string()),
                CacheOrgPolicySnapshotRouteRequest(snapshot(OrgId::new())),
            )
            .await
            .expect_err("route/body mismatch should fail first");

        assert_eq!(error.kind(), OrgPolicyRouteErrorKind::BadRequest);
        assert_eq!(
            error.message(),
            "policy snapshot org_id must match route org id"
        );
    }

    #[tokio::test]
    async fn overlay_route_checks_route_body_mismatch_first() {
        let (_temp, daemon) = test_daemon().await;
        let error = daemon
            .handle()
            .workspaces()
            .upsert_workspace_policy_overlay_for_route(
                OrgPolicyWorkspaceRouteParams::new(WorkspaceId::new().0.to_string()),
                UpsertWorkspacePolicyOverlayRouteRequest(overlay(WorkspaceId::new(), OrgId::new())),
            )
            .await
            .expect_err("route/body mismatch should fail first");

        assert_eq!(error.kind(), OrgPolicyRouteErrorKind::BadRequest);
        assert_eq!(
            error.message(),
            "workspace policy overlay workspace_id must match route workspace id"
        );
    }

    #[tokio::test]
    async fn route_facades_classify_domain_errors() {
        let (_temp, daemon) = test_daemon().await;
        let org_id = OrgId::new();

        let snapshot_error = daemon
            .handle()
            .core()
            .cache_org_policy_snapshot_for_route(
                OrgPolicyOrgRouteParams::new(org_id.0.to_string()),
                CacheOrgPolicySnapshotRouteRequest(snapshot(org_id)),
            )
            .await
            .expect_err("missing enrollment should conflict");
        assert_eq!(snapshot_error.kind(), OrgPolicyRouteErrorKind::Conflict);
        assert_eq!(
            snapshot_error.message(),
            "daemon is not enrolled for this org"
        );

        let workspace = daemon
            .global_store()
            .create_workspace(
                "workspace".to_string(),
                daemon
                    .data_root()
                    .join("workspace")
                    .to_string_lossy()
                    .to_string(),
                VcsKind::Git,
            )
            .await
            .expect("create workspace");
        let overlay_error = daemon
            .handle()
            .workspaces()
            .upsert_workspace_policy_overlay_for_route(
                OrgPolicyWorkspaceRouteParams::new(workspace.id.0.to_string()),
                UpsertWorkspacePolicyOverlayRouteRequest(overlay(workspace.id, OrgId::new())),
            )
            .await
            .expect_err("missing enrollment should conflict");
        assert_eq!(overlay_error.kind(), OrgPolicyRouteErrorKind::Conflict);
        assert_eq!(
            overlay_error.message(),
            "daemon is not enrolled for this org"
        );

        let org_id = OrgId::new();
        daemon
            .handle()
            .core()
            .upsert_daemon_enrollment_unchecked(enrollment(org_id))
            .await
            .expect("seed enrollment");
        let missing_workspace_id = WorkspaceId::new();
        let missing_workspace_error = daemon
            .handle()
            .workspaces()
            .upsert_workspace_policy_overlay_for_route(
                OrgPolicyWorkspaceRouteParams::new(missing_workspace_id.0.to_string()),
                UpsertWorkspacePolicyOverlayRouteRequest(overlay(missing_workspace_id, org_id)),
            )
            .await
            .expect_err("missing workspace should return not found");
        assert_eq!(
            missing_workspace_error.kind(),
            OrgPolicyRouteErrorKind::NotFound
        );
        assert_eq!(
            missing_workspace_error.message(),
            "workspace not found for org policy"
        );
    }

    #[tokio::test]
    async fn snapshot_route_classifies_invalid_signature() {
        let (_temp, daemon) = test_daemon().await;
        let org_id = OrgId::new();
        let enrollment = enrollment(org_id);
        daemon
            .handle()
            .core()
            .upsert_daemon_enrollment_unchecked(enrollment)
            .await
            .expect("seed enrollment");
        let mut snapshot = snapshot(org_id);
        snapshot.signature = "invalid".to_string();

        let error = daemon
            .handle()
            .core()
            .cache_org_policy_snapshot_for_route(
                OrgPolicyOrgRouteParams::new(org_id.0.to_string()),
                CacheOrgPolicySnapshotRouteRequest(snapshot),
            )
            .await
            .expect_err("invalid signature should fail");

        assert_eq!(error.kind(), OrgPolicyRouteErrorKind::BadRequest);
        assert!(error
            .message()
            .starts_with("invalid policy snapshot signature:"));
    }

    #[tokio::test]
    async fn snapshot_route_persists_signed_snapshot() {
        let (_temp, daemon) = test_daemon().await;
        let org_id = OrgId::new();
        let enrollment = enrollment(org_id);
        daemon
            .handle()
            .core()
            .upsert_daemon_enrollment_unchecked(enrollment.clone())
            .await
            .expect("seed enrollment");
        let mut snapshot = snapshot(org_id);
        snapshot.signature = sign_snapshot(&enrollment, &snapshot);

        let response = daemon
            .handle()
            .core()
            .cache_org_policy_snapshot_for_route(
                OrgPolicyOrgRouteParams::new(org_id.0.to_string()),
                CacheOrgPolicySnapshotRouteRequest(snapshot.clone()),
            )
            .await
            .expect("cache snapshot");

        let value = serde_json::to_value(response).expect("response json");
        let snapshot_id_text = snapshot.id.0.to_string();
        assert_eq!(
            value.get("id").and_then(|value| value.as_str()),
            Some(snapshot_id_text.as_str())
        );
    }
}
