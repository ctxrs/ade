use chrono::Utc;
use ctx_core::ids::{OrgId, WorkspaceId};
use ctx_core::models::{DaemonEnrollment, OrgPolicySnapshot, PlanType, WorkspacePolicyOverlay};

use crate::daemon::{CoreHandle, WorkspaceStoreAccessError, WorkspacesHandle};

#[derive(Debug)]
pub enum CacheOrgPolicySnapshotError {
    EnrollmentMissing,
    EnrollmentLoad(anyhow::Error),
    InvalidSignature { message: String },
    SnapshotStore(anyhow::Error),
    EnrollmentActivation(anyhow::Error),
}

#[derive(Debug)]
pub enum WorkspacePolicyOverlayError {
    WorkspaceNotFound,
    Store(anyhow::Error),
}

#[derive(Debug)]
pub enum UpsertWorkspacePolicyOverlayError {
    EnrollmentMissing,
    EnrollmentLoad(anyhow::Error),
    WorkspaceNotFound,
    Store(anyhow::Error),
}

#[derive(Debug)]
pub enum UpsertDaemonEnrollmentError {
    UnsupportedPlan,
    MissingSigningKey,
    Store(anyhow::Error),
}

fn workspace_policy_store_error(error: WorkspaceStoreAccessError) -> WorkspacePolicyOverlayError {
    match error {
        WorkspaceStoreAccessError::NotFound => WorkspacePolicyOverlayError::WorkspaceNotFound,
        WorkspaceStoreAccessError::Unavailable(error) => WorkspacePolicyOverlayError::Store(error),
    }
}

fn upsert_workspace_policy_overlay_error(
    error: WorkspacePolicyOverlayError,
) -> UpsertWorkspacePolicyOverlayError {
    match error {
        WorkspacePolicyOverlayError::WorkspaceNotFound => {
            UpsertWorkspacePolicyOverlayError::WorkspaceNotFound
        }
        WorkspacePolicyOverlayError::Store(error) => {
            UpsertWorkspacePolicyOverlayError::Store(error)
        }
    }
}

impl CoreHandle {
    pub async fn list_daemon_enrollments(&self) -> anyhow::Result<Vec<DaemonEnrollment>> {
        self.global_store().list_daemon_enrollments().await
    }

    pub(in crate::daemon) async fn upsert_daemon_enrollment_unchecked(
        &self,
        enrollment: DaemonEnrollment,
    ) -> anyhow::Result<DaemonEnrollment> {
        self.global_store()
            .upsert_daemon_enrollment(enrollment)
            .await
    }

    pub async fn upsert_daemon_enrollment_checked(
        &self,
        mut enrollment: DaemonEnrollment,
    ) -> Result<DaemonEnrollment, UpsertDaemonEnrollmentError> {
        if !matches!(enrollment.plan_type, PlanType::Team | PlanType::Enterprise) {
            return Err(UpsertDaemonEnrollmentError::UnsupportedPlan);
        }
        if enrollment.policy_signing_key.trim().is_empty() {
            return Err(UpsertDaemonEnrollmentError::MissingSigningKey);
        }
        enrollment.updated_at = Utc::now();
        self.upsert_daemon_enrollment_unchecked(enrollment)
            .await
            .map_err(UpsertDaemonEnrollmentError::Store)
    }

    pub async fn get_daemon_enrollment_by_org_id(
        &self,
        org_id: OrgId,
    ) -> anyhow::Result<Option<DaemonEnrollment>> {
        self.global_store()
            .get_daemon_enrollment_by_org_id(org_id)
            .await
    }

    pub async fn upsert_org_policy_snapshot(
        &self,
        snapshot: OrgPolicySnapshot,
    ) -> anyhow::Result<OrgPolicySnapshot> {
        self.global_store()
            .upsert_org_policy_snapshot(snapshot)
            .await
    }

    pub async fn cache_and_activate_org_policy_snapshot(
        &self,
        snapshot: OrgPolicySnapshot,
    ) -> Result<OrgPolicySnapshot, CacheOrgPolicySnapshotError> {
        let Some(mut enrollment) = self
            .global_store()
            .get_daemon_enrollment_by_org_id(snapshot.org_id)
            .await
            .map_err(CacheOrgPolicySnapshotError::EnrollmentLoad)?
        else {
            return Err(CacheOrgPolicySnapshotError::EnrollmentMissing);
        };

        ctx_org_policy::signature::verify_policy_snapshot_signature(&enrollment, &snapshot)
            .map_err(|error| CacheOrgPolicySnapshotError::InvalidSignature {
                message: format!("{error:#}"),
            })?;

        let stored = self
            .global_store()
            .upsert_org_policy_snapshot(snapshot)
            .await
            .map_err(CacheOrgPolicySnapshotError::SnapshotStore)?;
        enrollment.active_policy_snapshot_id = Some(stored.id);
        enrollment.updated_at = Utc::now();
        self.upsert_daemon_enrollment_unchecked(enrollment)
            .await
            .map_err(CacheOrgPolicySnapshotError::EnrollmentActivation)?;
        Ok(stored)
    }
}

impl WorkspacesHandle {
    pub async fn get_workspace_policy_overlay(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<WorkspacePolicyOverlay>, WorkspacePolicyOverlayError> {
        let store = self
            .existing_workspace_store(workspace_id)
            .await
            .map_err(workspace_policy_store_error)?;
        store
            .get_workspace_policy_overlay(workspace_id)
            .await
            .map_err(WorkspacePolicyOverlayError::Store)
    }

    pub async fn upsert_workspace_policy_overlay(
        &self,
        overlay: WorkspacePolicyOverlay,
    ) -> Result<WorkspacePolicyOverlay, WorkspacePolicyOverlayError> {
        let store = self
            .existing_workspace_store(overlay.workspace_id)
            .await
            .map_err(workspace_policy_store_error)?;
        store
            .upsert_workspace_policy_overlay(overlay)
            .await
            .map_err(WorkspacePolicyOverlayError::Store)
    }

    pub async fn upsert_workspace_policy_overlay_checked(
        &self,
        overlay: WorkspacePolicyOverlay,
    ) -> Result<WorkspacePolicyOverlay, UpsertWorkspacePolicyOverlayError> {
        let enrollment = self
            .state
            .global_store()
            .get_daemon_enrollment_by_org_id(overlay.org_id)
            .await
            .map_err(UpsertWorkspacePolicyOverlayError::EnrollmentLoad)?;
        if enrollment.is_none() {
            return Err(UpsertWorkspacePolicyOverlayError::EnrollmentMissing);
        }
        self.upsert_workspace_policy_overlay(overlay)
            .await
            .map_err(upsert_workspace_policy_overlay_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use ctx_core::ids::{
        AccountId, DaemonEnrollmentId, OrgId, OrgMembershipId, OrgPolicySnapshotId, WorkspaceId,
    };
    use ctx_core::models::{
        ArchiveMode, ArchivePolicy, DaemonEnrollmentStatus, NetworkProfile, OrgMembershipRole,
        PlanType, PolicyFeatureState, PolicySignatureAlgorithm, RoutePolicy, RouteType, VcsKind,
    };
    use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
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

    #[tokio::test]
    async fn cache_and_activate_org_policy_snapshot_updates_enrollment() {
        let (_temp, daemon) = test_daemon().await;
        let core = daemon.handle().core();
        let org_id = OrgId::new();
        let enrollment = enrollment(org_id);
        let previous_updated_at = enrollment.updated_at;
        core.upsert_daemon_enrollment_unchecked(enrollment.clone())
            .await
            .expect("seed enrollment");
        let mut snapshot = snapshot(org_id);
        snapshot.signature = sign_snapshot(&enrollment, &snapshot);

        let stored = core
            .cache_and_activate_org_policy_snapshot(snapshot.clone())
            .await
            .expect("cache and activate snapshot");

        assert_eq!(stored.id, snapshot.id);
        let activated = core
            .get_daemon_enrollment_by_org_id(org_id)
            .await
            .expect("load enrollment")
            .expect("enrollment exists");
        assert_eq!(activated.active_policy_snapshot_id, Some(snapshot.id));
        assert!(activated.updated_at >= previous_updated_at);
    }

    #[tokio::test]
    async fn cache_and_activate_org_policy_snapshot_requires_enrollment() {
        let (_temp, daemon) = test_daemon().await;
        let error = daemon
            .handle()
            .core()
            .cache_and_activate_org_policy_snapshot(snapshot(OrgId::new()))
            .await
            .expect_err("missing enrollment should fail");

        assert!(matches!(
            error,
            CacheOrgPolicySnapshotError::EnrollmentMissing
        ));
    }

    #[tokio::test]
    async fn cache_and_activate_org_policy_snapshot_rejects_invalid_signature() {
        let (_temp, daemon) = test_daemon().await;
        let core = daemon.handle().core();
        let org_id = OrgId::new();
        let enrollment = enrollment(org_id);
        core.upsert_daemon_enrollment_unchecked(enrollment)
            .await
            .expect("seed enrollment");
        let mut snapshot = snapshot(org_id);
        snapshot.signature = "invalid".to_string();

        let error = core
            .cache_and_activate_org_policy_snapshot(snapshot)
            .await
            .expect_err("invalid signature should fail");

        assert!(matches!(
            error,
            CacheOrgPolicySnapshotError::InvalidSignature { .. }
        ));
        let enrollment = core
            .get_daemon_enrollment_by_org_id(org_id)
            .await
            .expect("load enrollment")
            .expect("enrollment exists");
        assert_eq!(enrollment.active_policy_snapshot_id, None);
    }

    #[tokio::test]
    async fn upsert_workspace_policy_overlay_checked_requires_enrollment() {
        let (_temp, daemon) = test_daemon().await;
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

        let error = daemon
            .handle()
            .workspaces()
            .upsert_workspace_policy_overlay_checked(overlay(workspace.id, OrgId::new()))
            .await
            .expect_err("missing enrollment should fail");

        assert!(matches!(
            error,
            UpsertWorkspacePolicyOverlayError::EnrollmentMissing
        ));
    }

    #[tokio::test]
    async fn upsert_workspace_policy_overlay_checked_rejects_missing_workspace() {
        let (_temp, daemon) = test_daemon().await;
        let org_id = OrgId::new();
        daemon
            .handle()
            .core()
            .upsert_daemon_enrollment_unchecked(enrollment(org_id))
            .await
            .expect("seed enrollment");

        let error = daemon
            .handle()
            .workspaces()
            .upsert_workspace_policy_overlay_checked(overlay(WorkspaceId::new(), org_id))
            .await
            .expect_err("missing workspace should fail");

        assert!(matches!(
            error,
            UpsertWorkspacePolicyOverlayError::WorkspaceNotFound
        ));
    }

    #[tokio::test]
    async fn upsert_workspace_policy_overlay_checked_persists_overlay() {
        let (_temp, daemon) = test_daemon().await;
        let org_id = OrgId::new();
        daemon
            .handle()
            .core()
            .upsert_daemon_enrollment_unchecked(enrollment(org_id))
            .await
            .expect("seed enrollment");
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

        let stored = daemon
            .handle()
            .workspaces()
            .upsert_workspace_policy_overlay_checked(overlay(workspace.id, org_id))
            .await
            .expect("upsert overlay");

        assert_eq!(stored.workspace_id, workspace.id);
        assert_eq!(stored.org_id, org_id);
    }

    #[tokio::test]
    async fn upsert_daemon_enrollment_checked_rejects_unsupported_plan_without_persisting() {
        let (_temp, daemon) = test_daemon().await;
        let core = daemon.handle().core();
        let org_id = OrgId::new();
        let mut enrollment = enrollment(org_id);
        enrollment.plan_type = PlanType::Pro;

        let error = core
            .upsert_daemon_enrollment_checked(enrollment)
            .await
            .expect_err("unsupported plan should fail");

        assert!(matches!(
            error,
            UpsertDaemonEnrollmentError::UnsupportedPlan
        ));
        assert!(
            core.get_daemon_enrollment_by_org_id(org_id)
                .await
                .expect("load enrollment")
                .is_none(),
            "invalid enrollment should not persist"
        );
    }

    #[tokio::test]
    async fn upsert_daemon_enrollment_checked_rejects_blank_key_without_overwriting() {
        let (_temp, daemon) = test_daemon().await;
        let core = daemon.handle().core();
        let org_id = OrgId::new();
        let original = core
            .upsert_daemon_enrollment_checked(enrollment(org_id))
            .await
            .expect("seed enrollment");
        let mut replacement = original.clone();
        replacement.policy_signing_key = "   ".to_string();
        replacement.plan_type = PlanType::Enterprise;

        let error = core
            .upsert_daemon_enrollment_checked(replacement)
            .await
            .expect_err("blank signing key should fail");

        assert!(matches!(
            error,
            UpsertDaemonEnrollmentError::MissingSigningKey
        ));
        let reloaded = core
            .get_daemon_enrollment_by_org_id(org_id)
            .await
            .expect("load enrollment")
            .expect("enrollment exists");
        assert_eq!(reloaded.policy_signing_key, original.policy_signing_key);
        assert_eq!(reloaded.plan_type, original.plan_type);
        assert_eq!(reloaded.updated_at, original.updated_at);
    }

    #[tokio::test]
    async fn upsert_daemon_enrollment_checked_refreshes_timestamp_and_preserves_fields() {
        let (_temp, daemon) = test_daemon().await;
        let core = daemon.handle().core();
        let org_id = OrgId::new();
        let mut enrollment = enrollment(org_id);
        enrollment.plan_type = PlanType::Enterprise;
        enrollment.updated_at = Utc::now() - Duration::minutes(5);
        let previous_updated_at = enrollment.updated_at;
        let enrollment_id = enrollment.id;
        let membership_id = enrollment.org_membership_id;
        let started_at = Utc::now();

        let stored = core
            .upsert_daemon_enrollment_checked(enrollment)
            .await
            .expect("checked enrollment upsert");

        assert_eq!(stored.id, enrollment_id);
        assert_eq!(stored.org_membership_id, membership_id);
        assert_eq!(stored.plan_type, PlanType::Enterprise);
        assert!(stored.updated_at >= started_at);
        assert!(stored.updated_at >= previous_updated_at);
        let reloaded = core
            .get_daemon_enrollment_by_org_id(org_id)
            .await
            .expect("load enrollment")
            .expect("enrollment exists");
        assert_eq!(reloaded.id, enrollment_id);
        assert_eq!(reloaded.org_membership_id, membership_id);
        assert_eq!(reloaded.plan_type, PlanType::Enterprise);
        assert_eq!(reloaded.updated_at, stored.updated_at);
    }
}
