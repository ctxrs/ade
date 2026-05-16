use super::*;
use chrono::Utc;
use ctx_core::models::{
    DaemonEnrollment, DaemonEnrollmentStatus, OrgMembershipRole, PlanType, PolicySignatureAlgorithm,
};

#[tokio::test]
async fn daemon_enrollment_routes_do_not_return_policy_signing_keys() {
    let data_dir = tempfile::tempdir().unwrap();
    let fixture = test_daemon_fixture_for_test(data_dir.path(), None).await;
    let app = fixture.router();
    let org_id = ctx_core::ids::OrgId::new();
    let secret = "policy-signing-secret";
    let now = Utc::now();
    let enrollment = DaemonEnrollment {
        id: ctx_core::ids::DaemonEnrollmentId::new(),
        account_id: ctx_core::ids::AccountId::new(),
        org_id,
        org_membership_id: ctx_core::ids::OrgMembershipId::new(),
        membership_role: OrgMembershipRole::Owner,
        plan_type: PlanType::Team,
        status: DaemonEnrollmentStatus::Active,
        policy_signature_algorithm: PolicySignatureAlgorithm::Hs256,
        policy_signing_key: secret.to_string(),
        active_policy_snapshot_id: None,
        enrolled_at: now,
        updated_at: now,
        revoked_at: None,
    };

    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/orgs/{}/daemon_enrollment", org_id.0))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&enrollment).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body_text.contains(secret));
    assert!(!body_text.contains("\"policy_signing_key\":"));
    assert!(body_text.contains("policy_signing_key_present"));

    let req = Request::builder()
        .method("GET")
        .uri("/api/orgs/daemon_enrollments")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let body_text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body_text.contains(secret));
    assert!(!body_text.contains("\"policy_signing_key\":"));
    assert!(body_text.contains("policy_signing_key_present"));
}
