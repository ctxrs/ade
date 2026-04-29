use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ctx_llm_relay_contract::{
    allocate_credit_reservation, finalize_credit_reservation, release_credit_reservation,
    CreditAllocation, CreditGrant, CreditReservation, GrantValidationError, RelayRequestState,
};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::api::{
    FinalizeRequest, ProviderStartedRequest, RequestSnapshot, ReserveRequest, ReserveResponse,
    VoidRequest,
};
use crate::store::{AuthorityError, AuthorityStore, StateEvent};

#[derive(Debug, Clone, Default)]
pub struct InMemorySeed {
    pub credit_grants: Vec<CreditGrant>,
    pub per_user_caps_cents: HashMap<(String, String), u64>,
    pub billing_subject_caps_cents: HashMap<String, u64>,
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryAuthorityStore {
    inner: Arc<Mutex<InMemoryState>>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    credit_grants: HashMap<String, CreditGrant>,
    requests: HashMap<String, RequestRecord>,
    per_user_caps_cents: HashMap<(String, String), u64>,
    billing_subject_caps_cents: HashMap<String, u64>,
}

#[derive(Debug, Clone)]
struct RequestRecord {
    snapshot: RequestSnapshot,
    reservation: CreditReservation,
    fingerprint: ReservationFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReservationFingerprint {
    run_grant_jti: String,
    relay_delegation_jti: String,
    billing_subject_id: String,
    ctx_user_id: String,
    ctx_org_id: Option<String>,
    route_id: String,
    provider_id: String,
    model_id: String,
    pricing_version: String,
    max_estimated_cents: u64,
    max_input_tokens: Option<u32>,
    max_output_tokens: Option<u32>,
}

impl ReservationFingerprint {
    fn from_request(request: &ReserveRequest) -> Self {
        Self {
            run_grant_jti: request.grant.jti.clone(),
            relay_delegation_jti: request.grant.delegation_jti.clone(),
            billing_subject_id: request.grant.billing_subject_id.clone(),
            ctx_user_id: request.grant.ctx_user_id.clone(),
            ctx_org_id: request.grant.ctx_org_id.clone(),
            route_id: request.grant.route_id.clone(),
            provider_id: request.grant.provider_id.clone(),
            model_id: request.grant.model_id.clone(),
            pricing_version: request.grant.pricing_version.clone(),
            max_estimated_cents: request.grant.max_estimated_cents,
            max_input_tokens: request.grant.max_input_tokens,
            max_output_tokens: request.grant.max_output_tokens,
        }
    }
}

impl InMemoryAuthorityStore {
    pub fn seeded(seed: InMemorySeed) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InMemoryState {
                credit_grants: seed
                    .credit_grants
                    .into_iter()
                    .map(|grant| (grant.grant_id.clone(), grant))
                    .collect(),
                requests: HashMap::new(),
                per_user_caps_cents: seed.per_user_caps_cents,
                billing_subject_caps_cents: seed.billing_subject_caps_cents,
            })),
        }
    }
}

#[async_trait]
impl AuthorityStore for InMemoryAuthorityStore {
    async fn reserve(&self, request: ReserveRequest) -> Result<ReserveResponse, AuthorityError> {
        let now = Utc::now();
        request
            .grant
            .validate_against_delegation(&request.delegation, now)
            .map_err(map_grant_validation_error)?;

        let mut state = self.inner.lock().await;
        let fingerprint = ReservationFingerprint::from_request(&request);
        if let Some(existing) = state.requests.get(&request.grant.request_id) {
            if existing.fingerprint != fingerprint {
                return Err(AuthorityError::Conflict(
                    "request id was already reserved with different grant".to_string(),
                ));
            }
            if existing.snapshot.state == RelayRequestState::Reserved {
                return Ok(ReserveResponse {
                    request_id: existing.snapshot.request_id.clone(),
                    reservation_id: existing.snapshot.reservation_id.clone().ok_or_else(|| {
                        AuthorityError::Store("missing reservation id".to_string())
                    })?,
                    state: existing.snapshot.state.clone(),
                    reserved_cents: existing.snapshot.reserved_cents,
                    idempotent: true,
                });
            }
            return Err(AuthorityError::Conflict(
                "request id was already used after provider start".to_string(),
            ));
        }

        enforce_caps(&state, &request)?;
        let grants = state
            .credit_grants
            .values()
            .filter(|grant| grant.billing_subject_id == request.grant.billing_subject_id)
            .cloned()
            .collect::<Vec<_>>();
        let reservation =
            allocate_credit_reservation(&grants, request.grant.max_estimated_cents, now)
                .map_err(|err| AuthorityError::InsufficientCredits(err.to_string()))?;
        apply_allocations(&mut state.credit_grants, &reservation.allocations)?;

        let reservation_id = format!("res_{}", Uuid::new_v4());
        let state_event = StateEvent {
            request_id: request.grant.request_id.clone(),
            from_state: None,
            to_state: RelayRequestState::Reserved,
            reason: None,
        };
        let snapshot = RequestSnapshot {
            request_id: request.grant.request_id.clone(),
            reservation_id: Some(reservation_id.clone()),
            billing_subject_id: Some(request.grant.billing_subject_id.clone()),
            ctx_user_id: Some(request.grant.ctx_user_id.clone()),
            ctx_org_id: request.grant.ctx_org_id.clone(),
            route_id: Some(request.grant.route_id.clone()),
            provider_id: Some(request.grant.provider_id.clone()),
            model_id: Some(request.grant.model_id.clone()),
            state: RelayRequestState::Reserved,
            reserved_cents: reservation.total_reserved_cents(),
            provider_request_id: None,
            updated_at: now,
            state_events: vec![state_event],
        };
        state.requests.insert(
            request.grant.request_id.clone(),
            RequestRecord {
                snapshot: snapshot.clone(),
                reservation,
                fingerprint,
            },
        );

        Ok(ReserveResponse {
            request_id: snapshot.request_id,
            reservation_id,
            state: snapshot.state,
            reserved_cents: snapshot.reserved_cents,
            idempotent: false,
        })
    }

    async fn provider_started(
        &self,
        request: ProviderStartedRequest,
    ) -> Result<RequestSnapshot, AuthorityError> {
        let mut state = self.inner.lock().await;
        let record = state
            .requests
            .get_mut(&request.request_id)
            .ok_or(AuthorityError::NotFound)?;
        if record.snapshot.state == RelayRequestState::ProviderStarted {
            if let Some(provider_request_id) = request.provider_request_id {
                record.snapshot.provider_request_id = Some(provider_request_id);
            }
            return Ok(record.snapshot.clone());
        }
        transition(
            &mut record.snapshot,
            RelayRequestState::ProviderStarted,
            None,
        )?;
        if let Some(provider_request_id) = request.provider_request_id {
            record.snapshot.provider_request_id = Some(provider_request_id);
        }
        Ok(record.snapshot.clone())
    }

    async fn finalize(&self, request: FinalizeRequest) -> Result<RequestSnapshot, AuthorityError> {
        let now = Utc::now();
        let mut state = self.inner.lock().await;
        let existing = state
            .requests
            .get(&request.request_id)
            .ok_or(AuthorityError::NotFound)?;
        if !matches!(
            existing.snapshot.state,
            RelayRequestState::ProviderStarted
                | RelayRequestState::StreamCompleted
                | RelayRequestState::StreamBrokenUsageUnknown
        ) {
            return Err(AuthorityError::Conflict(
                "finalize requires provider-started or stream-complete state".to_string(),
            ));
        }
        if request.usage_unknown
            || request.actual_input_tokens.is_none()
            || request.actual_output_tokens.is_none()
        {
            let record = state
                .requests
                .get_mut(&request.request_id)
                .ok_or(AuthorityError::NotFound)?;
            if record.snapshot.state == RelayRequestState::ProviderStarted {
                transition(
                    &mut record.snapshot,
                    RelayRequestState::StreamBrokenUsageUnknown,
                    None,
                )?;
                if let Some(provider_request_id) = request.provider_request_id {
                    record.snapshot.provider_request_id = Some(provider_request_id);
                }
                record.snapshot.updated_at = now;
                return Ok(record.snapshot.clone());
            }
            if record.snapshot.state == RelayRequestState::StreamBrokenUsageUnknown {
                return Ok(record.snapshot.clone());
            }
            return Err(AuthorityError::Conflict(
                "unknown usage finalization requires provider-started state".to_string(),
            ));
        }
        let reservation = existing.reservation.clone();
        let settlements = finalize_credit_reservation(&reservation, request.billable_cents, now)
            .map_err(|err| AuthorityError::Conflict(err.to_string()))?;
        release_settlements(&mut state.credit_grants, settlements)?;

        let record = state
            .requests
            .get_mut(&request.request_id)
            .ok_or(AuthorityError::NotFound)?;
        if record.snapshot.state == RelayRequestState::ProviderStarted {
            transition(
                &mut record.snapshot,
                RelayRequestState::StreamCompleted,
                None,
            )?;
        }
        transition(&mut record.snapshot, RelayRequestState::Finalized, None)?;
        if let Some(provider_request_id) = request.provider_request_id {
            record.snapshot.provider_request_id = Some(provider_request_id);
        }
        record.snapshot.updated_at = now;
        Ok(record.snapshot.clone())
    }

    async fn void(&self, request: VoidRequest) -> Result<RequestSnapshot, AuthorityError> {
        let now = Utc::now();
        let mut state = self.inner.lock().await;
        let reservation = state
            .requests
            .get(&request.request_id)
            .ok_or(AuthorityError::NotFound)?;
        if reservation.snapshot.state != RelayRequestState::Reserved {
            return Err(AuthorityError::Conflict(
                "void is only allowed before provider start".to_string(),
            ));
        }
        let settlements = release_credit_reservation(&reservation.reservation, now);
        release_settlements(&mut state.credit_grants, settlements)?;

        let record = state
            .requests
            .get_mut(&request.request_id)
            .ok_or(AuthorityError::NotFound)?;
        transition(
            &mut record.snapshot,
            RelayRequestState::Voided,
            request.reason,
        )?;
        record.snapshot.updated_at = now;
        Ok(record.snapshot.clone())
    }

    async fn get_request(&self, request_id: &str) -> Result<RequestSnapshot, AuthorityError> {
        let state = self.inner.lock().await;
        state
            .requests
            .get(request_id)
            .map(|record| record.snapshot.clone())
            .ok_or(AuthorityError::NotFound)
    }
}

fn map_grant_validation_error(err: GrantValidationError) -> AuthorityError {
    AuthorityError::Forbidden(err.to_string())
}

fn enforce_caps(state: &InMemoryState, request: &ReserveRequest) -> Result<(), AuthorityError> {
    let subject_id = request.grant.billing_subject_id.clone();
    let user_id = request.grant.ctx_user_id.clone();
    let requested = request.grant.max_estimated_cents;

    if let Some(cap) = state
        .per_user_caps_cents
        .get(&(subject_id.clone(), user_id.clone()))
    {
        let used = state
            .requests
            .values()
            .filter(|record| {
                record.snapshot.billing_subject_id.as_deref() == Some(subject_id.as_str())
                    && record.snapshot.ctx_user_id.as_deref() == Some(user_id.as_str())
                    && record.snapshot.state != RelayRequestState::Voided
            })
            .map(|record| record.snapshot.reserved_cents)
            .sum::<u64>();
        if used.saturating_add(requested) > *cap {
            return Err(AuthorityError::InsufficientCredits(
                "per-user cap would be exceeded".to_string(),
            ));
        }
    }

    if let Some(cap) = state.billing_subject_caps_cents.get(&subject_id) {
        let used = state
            .requests
            .values()
            .filter(|record| {
                record.snapshot.billing_subject_id.as_deref() == Some(subject_id.as_str())
                    && record.snapshot.state != RelayRequestState::Voided
            })
            .map(|record| record.snapshot.reserved_cents)
            .sum::<u64>();
        if used.saturating_add(requested) > *cap {
            return Err(AuthorityError::InsufficientCredits(
                "billing subject cap would be exceeded".to_string(),
            ));
        }
    }

    Ok(())
}

fn apply_allocations(
    grants: &mut HashMap<String, CreditGrant>,
    allocations: &[CreditAllocation],
) -> Result<(), AuthorityError> {
    for allocation in allocations {
        let grant = grants.get_mut(&allocation.grant_id).ok_or_else(|| {
            AuthorityError::Store(format!("missing credit grant {}", allocation.grant_id))
        })?;
        if grant.remaining_cents < allocation.reserved_cents {
            return Err(AuthorityError::InsufficientCredits(
                "credit grant was over-allocated".to_string(),
            ));
        }
        grant.remaining_cents -= allocation.reserved_cents;
    }
    Ok(())
}

fn release_settlements(
    grants: &mut HashMap<String, CreditGrant>,
    settlements: Vec<ctx_llm_relay_contract::CreditAllocationSettlement>,
) -> Result<(), AuthorityError> {
    for settlement in settlements {
        let grant = grants.get_mut(&settlement.grant_id).ok_or_else(|| {
            AuthorityError::Store(format!("missing credit grant {}", settlement.grant_id))
        })?;
        grant.remaining_cents = grant
            .remaining_cents
            .saturating_add(settlement.released_spendable_cents);
    }
    Ok(())
}

fn transition(
    snapshot: &mut RequestSnapshot,
    next: RelayRequestState,
    reason: Option<String>,
) -> Result<(), AuthorityError> {
    if !snapshot.state.can_transition_to(&next) {
        return Err(AuthorityError::Conflict(format!(
            "invalid state transition from {:?} to {:?}",
            snapshot.state, next
        )));
    }
    let previous = snapshot.state.clone();
    snapshot.state = next.clone();
    snapshot.updated_at = Utc::now();
    snapshot.state_events.push(StateEvent {
        request_id: snapshot.request_id.clone(),
        from_state: Some(previous),
        to_state: next,
        reason,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
