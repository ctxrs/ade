use chrono::{Duration, Utc};
use ctx_core::ids::{
    AccountId, DaemonEnrollmentId, OrgId, OrgMembershipId, OrgPolicySnapshotId, WorkspaceId,
};
use ctx_core::models::{
    ArchiveMode, ArchivePolicy, DaemonEnrollment, DaemonEnrollmentStatus, NetworkProfile,
    OrgMembershipRole, OrgPolicySnapshot, PlanType, PolicyFeatureState, PolicySignatureAlgorithm,
    RoutePolicy, RouteType, VcsKind, WorkspacePolicyOverlay,
};
use ctx_store::Store;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use std::collections::BTreeMap;

use crate::{
    cache_and_activate_org_policy_snapshot, get_daemon_enrollment_by_org_id,
    upsert_daemon_enrollment_checked, upsert_daemon_enrollment_unchecked,
    upsert_workspace_policy_overlay_checked, CacheOrgPolicySnapshotError,
    UpsertDaemonEnrollmentError, UpsertWorkspacePolicyOverlayError,
};

async fn setup_store(name: &str) -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::open(dir.path().join(format!("{name}.sqlite")))
        .await
        .expect("open store");
    (dir, store)
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
        "snapshot_sha256": crate::signature::policy_snapshot_digest_hex(snapshot)
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
    let (_dir, store) = setup_store("global").await;
    let org_id = OrgId::new();
    let enrollment = enrollment(org_id);
    let previous_updated_at = enrollment.updated_at;
    upsert_daemon_enrollment_unchecked(&store, enrollment.clone())
        .await
        .expect("seed enrollment");
    let mut snapshot = snapshot(org_id);
    snapshot.signature = sign_snapshot(&enrollment, &snapshot);

    let stored = cache_and_activate_org_policy_snapshot(&store, snapshot.clone())
        .await
        .expect("cache and activate snapshot");

    assert_eq!(stored.id, snapshot.id);
    let activated = get_daemon_enrollment_by_org_id(&store, org_id)
        .await
        .expect("load enrollment")
        .expect("enrollment exists");
    assert_eq!(activated.active_policy_snapshot_id, Some(snapshot.id));
    assert!(activated.updated_at >= previous_updated_at);
}

#[tokio::test]
async fn cache_and_activate_org_policy_snapshot_requires_enrollment() {
    let (_dir, store) = setup_store("global").await;
    let error = cache_and_activate_org_policy_snapshot(&store, snapshot(OrgId::new()))
        .await
        .expect_err("missing enrollment should fail");

    assert!(matches!(
        error,
        CacheOrgPolicySnapshotError::EnrollmentMissing
    ));
}

#[tokio::test]
async fn cache_and_activate_org_policy_snapshot_rejects_invalid_signature() {
    let (_dir, store) = setup_store("global").await;
    let org_id = OrgId::new();
    let enrollment = enrollment(org_id);
    upsert_daemon_enrollment_unchecked(&store, enrollment)
        .await
        .expect("seed enrollment");
    let mut snapshot = snapshot(org_id);
    snapshot.signature = "invalid".to_string();

    let error = cache_and_activate_org_policy_snapshot(&store, snapshot)
        .await
        .expect_err("invalid signature should fail");

    assert!(matches!(
        error,
        CacheOrgPolicySnapshotError::InvalidSignature { .. }
    ));
    let enrollment = get_daemon_enrollment_by_org_id(&store, org_id)
        .await
        .expect("load enrollment")
        .expect("enrollment exists");
    assert_eq!(enrollment.active_policy_snapshot_id, None);
}

#[tokio::test]
async fn upsert_workspace_policy_overlay_checked_requires_enrollment() {
    let (_global_dir, global_store) = setup_store("global").await;
    let (_workspace_dir, workspace_store) = setup_store("workspace").await;
    let workspace = workspace_store
        .create_workspace(
            "workspace".to_string(),
            "/tmp/workspace".to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    let error = upsert_workspace_policy_overlay_checked(
        &global_store,
        &workspace_store,
        overlay(workspace.id, OrgId::new()),
    )
    .await
    .expect_err("missing enrollment should fail");

    assert!(matches!(
        error,
        UpsertWorkspacePolicyOverlayError::EnrollmentMissing
    ));
}

#[tokio::test]
async fn upsert_workspace_policy_overlay_checked_persists_overlay() {
    let (_global_dir, global_store) = setup_store("global").await;
    let (_workspace_dir, workspace_store) = setup_store("workspace").await;
    let org_id = OrgId::new();
    upsert_daemon_enrollment_unchecked(&global_store, enrollment(org_id))
        .await
        .expect("seed enrollment");
    let workspace = workspace_store
        .create_workspace(
            "workspace".to_string(),
            "/tmp/workspace".to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");

    let stored = upsert_workspace_policy_overlay_checked(
        &global_store,
        &workspace_store,
        overlay(workspace.id, org_id),
    )
    .await
    .expect("upsert overlay");

    assert_eq!(stored.workspace_id, workspace.id);
    assert_eq!(stored.org_id, org_id);
}

#[tokio::test]
async fn upsert_daemon_enrollment_checked_rejects_unsupported_plan_without_persisting() {
    let (_dir, store) = setup_store("global").await;
    let org_id = OrgId::new();
    let mut enrollment = enrollment(org_id);
    enrollment.plan_type = PlanType::Pro;

    let error = upsert_daemon_enrollment_checked(&store, enrollment)
        .await
        .expect_err("unsupported plan should fail");

    assert!(matches!(
        error,
        UpsertDaemonEnrollmentError::UnsupportedPlan
    ));
    assert!(
        get_daemon_enrollment_by_org_id(&store, org_id)
            .await
            .expect("load enrollment")
            .is_none(),
        "invalid enrollment should not persist"
    );
}

#[tokio::test]
async fn upsert_daemon_enrollment_checked_rejects_blank_key_without_overwriting() {
    let (_dir, store) = setup_store("global").await;
    let org_id = OrgId::new();
    let original = upsert_daemon_enrollment_checked(&store, enrollment(org_id))
        .await
        .expect("seed enrollment");
    let mut replacement = original.clone();
    replacement.policy_signing_key = "   ".to_string();
    replacement.plan_type = PlanType::Enterprise;

    let error = upsert_daemon_enrollment_checked(&store, replacement)
        .await
        .expect_err("blank signing key should fail");

    assert!(matches!(
        error,
        UpsertDaemonEnrollmentError::MissingSigningKey
    ));
    let reloaded = get_daemon_enrollment_by_org_id(&store, org_id)
        .await
        .expect("load enrollment")
        .expect("enrollment exists");
    assert_eq!(reloaded.policy_signing_key, original.policy_signing_key);
    assert_eq!(reloaded.plan_type, original.plan_type);
    assert_eq!(reloaded.updated_at, original.updated_at);
}

#[tokio::test]
async fn upsert_daemon_enrollment_checked_refreshes_timestamp_and_preserves_fields() {
    let (_dir, store) = setup_store("global").await;
    let org_id = OrgId::new();
    let mut enrollment = enrollment(org_id);
    enrollment.plan_type = PlanType::Enterprise;
    enrollment.updated_at = Utc::now() - Duration::minutes(5);
    let previous_updated_at = enrollment.updated_at;
    let enrollment_id = enrollment.id;
    let membership_id = enrollment.org_membership_id;
    let started_at = Utc::now();

    let stored = upsert_daemon_enrollment_checked(&store, enrollment)
        .await
        .expect("checked enrollment upsert");

    assert_eq!(stored.id, enrollment_id);
    assert_eq!(stored.org_membership_id, membership_id);
    assert_eq!(stored.plan_type, PlanType::Enterprise);
    assert!(stored.updated_at >= started_at);
    assert!(stored.updated_at >= previous_updated_at);
    let reloaded = get_daemon_enrollment_by_org_id(&store, org_id)
        .await
        .expect("load enrollment")
        .expect("enrollment exists");
    assert_eq!(reloaded.id, enrollment_id);
    assert_eq!(reloaded.org_membership_id, membership_id);
    assert_eq!(reloaded.plan_type, PlanType::Enterprise);
    assert_eq!(reloaded.updated_at, stored.updated_at);
}
