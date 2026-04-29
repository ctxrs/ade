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
mod tests;
