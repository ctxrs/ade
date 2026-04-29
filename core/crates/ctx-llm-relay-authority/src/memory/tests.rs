use std::collections::HashMap;

use chrono::{DateTime, Duration, TimeZone, Utc};
use ctx_llm_relay_contract::{
    AccessContextKind, CreditGrant, CreditGrantSource, ProviderModelRef, RelayDelegationClaims,
    RouteAuthMethod, RouteType, RunGrantClaims, CONTROL_PLANE_ISSUER, RELAY_AUDIENCE,
};

use super::*;

fn grant(
    grant_id: &str,
    billing_subject_id: &str,
    cents: u64,
    expires_at: Option<DateTime<Utc>>,
) -> CreditGrant {
    CreditGrant {
        grant_id: grant_id.to_string(),
        billing_subject_id: billing_subject_id.to_string(),
        source: CreditGrantSource::PrepaidTopUp,
        total_cents: cents,
        remaining_cents: cents,
        issued_at: Utc.with_ymd_and_hms(2026, 4, 1, 0, 0, 0).unwrap(),
        expires_at,
    }
}

fn reserve_request(request_id: &str, cents: u64) -> ReserveRequest {
    let issued_at = Utc::now() - Duration::seconds(10);
    let expires_at = issued_at + Duration::minutes(5);
    let delegation = RelayDelegationClaims {
        jti: "delegation_1".to_string(),
        access_context_kind: AccessContextKind::Org,
        billing_subject_id: "bill_org_1".to_string(),
        ctx_user_id: "user_1".to_string(),
        ctx_account_id: Some("account_1".to_string()),
        ctx_org_id: Some("org_1".to_string()),
        ctx_membership_id: Some("membership_1".to_string()),
        daemon_id: "daemon_1".to_string(),
        daemon_public_key_thumbprint: "thumb_1".to_string(),
        daemon_public_key_jwk: None,
        allowed_route_ids: vec!["route_ctx".to_string()],
        allowed_provider_model_pairs: vec![ProviderModelRef {
            provider_id: "openai".to_string(),
            model_id: "gpt-5".to_string(),
        }],
        allowed_auth_methods: vec![RouteAuthMethod::CtxProviderKey],
        policy_version: "policy_v1".to_string(),
        pricing_version: "pricing_v1".to_string(),
        max_per_request_cents: 500,
        max_input_tokens: 32_000,
        max_output_tokens: 4_096,
        max_concurrent_requests: Some(4),
        issued_at,
        expires_at,
        issuer: CONTROL_PLANE_ISSUER.to_string(),
        audience: RELAY_AUDIENCE.to_string(),
    };
    let grant = RunGrantClaims {
        jti: format!("grant_{request_id}"),
        request_id: request_id.to_string(),
        access_context_kind: AccessContextKind::Org,
        billing_subject_id: "bill_org_1".to_string(),
        ctx_user_id: "user_1".to_string(),
        ctx_account_id: Some("account_1".to_string()),
        ctx_org_id: Some("org_1".to_string()),
        ctx_membership_id: Some("membership_1".to_string()),
        daemon_id: "daemon_1".to_string(),
        workspace_id: Some("workspace_1".to_string()),
        task_id: Some("task_1".to_string()),
        session_id: Some("session_1".to_string()),
        run_id: Some("run_1".to_string()),
        turn_id: Some("turn_1".to_string()),
        route_id: "route_ctx".to_string(),
        route_type: RouteType::CtxManaged,
        provider_id: "openai".to_string(),
        model_id: "gpt-5".to_string(),
        policy_version: "policy_v1".to_string(),
        pricing_version: "pricing_v1".to_string(),
        max_estimated_cents: cents,
        max_input_tokens: Some(4_000),
        max_output_tokens: Some(1_000),
        delegation_jti: "delegation_1".to_string(),
        delegation_hash: "hash".to_string(),
        issued_at: issued_at + Duration::seconds(1),
        expires_at: expires_at - Duration::seconds(1),
        issuer: "daemon_1".to_string(),
        audience: RELAY_AUDIENCE.to_string(),
    };
    ReserveRequest {
        delegation,
        grant,
        delegation_jws: None,
        grant_jws: None,
        estimated_input_tokens: Some(128),
        estimated_output_tokens: Some(64),
        idempotency_key: None,
    }
}

#[tokio::test]
async fn reserve_is_idempotent_until_provider_start() {
    let store = InMemoryAuthorityStore::seeded(InMemorySeed {
        credit_grants: vec![grant("grant_1", "bill_org_1", 1_000, None)],
        ..InMemorySeed::default()
    });
    let response = store
        .reserve(reserve_request("request_1", 100))
        .await
        .expect("reserve");
    let retry = store
        .reserve(reserve_request("request_1", 100))
        .await
        .expect("retry");
    assert!(!response.idempotent);
    assert!(retry.idempotent);
    assert_eq!(response.reservation_id, retry.reservation_id);
}

#[tokio::test]
async fn reserve_rejects_same_request_id_with_different_grant() {
    let store = InMemoryAuthorityStore::seeded(InMemorySeed {
        credit_grants: vec![grant("grant_1", "bill_org_1", 1_000, None)],
        ..InMemorySeed::default()
    });
    store
        .reserve(reserve_request("request_mismatch", 100))
        .await
        .expect("reserve");

    let mut changed = reserve_request("request_mismatch", 100);
    changed.grant.jti = "grant_changed".to_string();
    let err = store
        .reserve(changed)
        .await
        .expect_err("same request id with different grant should reject");
    assert!(matches!(err, AuthorityError::Conflict(_)));
}

#[tokio::test]
async fn provider_start_then_finalize_records_stream_and_final_state() {
    let store = InMemoryAuthorityStore::seeded(InMemorySeed {
        credit_grants: vec![grant("grant_1", "bill_org_1", 1_000, None)],
        ..InMemorySeed::default()
    });
    store
        .reserve(reserve_request("request_2", 100))
        .await
        .expect("reserve");
    store
        .provider_started(ProviderStartedRequest {
            request_id: "request_2".to_string(),
            provider_request_id: Some("provider_req_1".to_string()),
            idempotency_key: None,
        })
        .await
        .expect("provider started");
    let snapshot = store
        .finalize(FinalizeRequest {
            request_id: "request_2".to_string(),
            billable_cents: 40,
            actual_input_tokens: Some(10),
            actual_output_tokens: Some(20),
            provider_cost_micros: Some(1_000),
            provider_request_id: None,
            usage_unknown: false,
            idempotency_key: None,
        })
        .await
        .expect("finalize");
    assert_eq!(snapshot.state, RelayRequestState::Finalized);
    assert!(snapshot
        .state_events
        .iter()
        .any(|event| event.to_state == RelayRequestState::StreamCompleted));
}

#[tokio::test]
async fn unknown_usage_stream_break_stays_open_for_reconciliation() {
    let store = InMemoryAuthorityStore::seeded(InMemorySeed {
        credit_grants: vec![grant("grant_1", "bill_org_1", 1_000, None)],
        ..InMemorySeed::default()
    });
    store
        .reserve(reserve_request("request_unknown", 100))
        .await
        .expect("reserve");
    store
        .provider_started(ProviderStartedRequest {
            request_id: "request_unknown".to_string(),
            provider_request_id: Some("provider_req_1".to_string()),
            idempotency_key: None,
        })
        .await
        .expect("provider started");
    let snapshot = store
        .finalize(FinalizeRequest {
            request_id: "request_unknown".to_string(),
            billable_cents: 100,
            actual_input_tokens: None,
            actual_output_tokens: None,
            provider_cost_micros: None,
            provider_request_id: None,
            usage_unknown: true,
            idempotency_key: None,
        })
        .await
        .expect("stream break");
    assert_eq!(snapshot.state, RelayRequestState::StreamBrokenUsageUnknown);
    assert!(snapshot
        .state_events
        .iter()
        .all(|event| event.to_state != RelayRequestState::Finalized));
}

#[tokio::test]
async fn missing_usage_tokens_stay_open_for_reconciliation() {
    let store = InMemoryAuthorityStore::seeded(InMemorySeed {
        credit_grants: vec![grant("grant_1", "bill_org_1", 1_000, None)],
        ..InMemorySeed::default()
    });
    store
        .reserve(reserve_request("request_missing_usage", 100))
        .await
        .expect("reserve");
    store
        .provider_started(ProviderStartedRequest {
            request_id: "request_missing_usage".to_string(),
            provider_request_id: Some("provider_req_1".to_string()),
            idempotency_key: None,
        })
        .await
        .expect("provider started");
    let snapshot = store
        .finalize(FinalizeRequest {
            request_id: "request_missing_usage".to_string(),
            billable_cents: 100,
            actual_input_tokens: None,
            actual_output_tokens: Some(20),
            provider_cost_micros: None,
            provider_request_id: None,
            usage_unknown: false,
            idempotency_key: None,
        })
        .await
        .expect("missing usage");
    assert_eq!(snapshot.state, RelayRequestState::StreamBrokenUsageUnknown);
}

#[tokio::test]
async fn per_user_cap_is_enforced() {
    let mut caps = HashMap::new();
    caps.insert(("bill_org_1".to_string(), "user_1".to_string()), 150);
    let store = InMemoryAuthorityStore::seeded(InMemorySeed {
        credit_grants: vec![grant("grant_1", "bill_org_1", 1_000, None)],
        per_user_caps_cents: caps,
        ..InMemorySeed::default()
    });
    store
        .reserve(reserve_request("request_3", 100))
        .await
        .expect("first reserve");
    let err = store
        .reserve(reserve_request("request_4", 100))
        .await
        .expect_err("cap should reject");
    assert!(matches!(err, AuthorityError::InsufficientCredits(_)));
}
