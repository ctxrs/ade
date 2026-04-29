use async_trait::async_trait;
use chrono::Utc;
use ctx_llm_relay_contract::{
    allocate_credit_reservation, finalize_credit_reservation, release_credit_reservation,
    CreditReservation, RelayRequestState,
};
use sqlx::{Pool, Postgres, Row};
use uuid::Uuid;

use crate::api::{
    FinalizeRequest, ProviderStartedRequest, RequestSnapshot, ReserveRequest, ReserveResponse,
    VoidRequest,
};
use crate::store::{AuthorityError, AuthorityStore};

mod conversions;
mod ledger;
mod pricing_policy;
mod reservations;
mod spend_limits;
mod usage_reconciliation;

use conversions::{cents_from_i64, checked_i64, ensure_transition, state_to_str};
use ledger::{
    insert_credit_event_tx, insert_state_event_tx, insert_usage_event_tx,
    latest_provider_request_id_pool, latest_state_tx, load_state_events_pool,
};
use pricing_policy::{billable_cents_tx, enforce_live_grant_config_tx};
use reservations::{
    existing_reservation_matches, insert_allocation_tx, load_allocations_tx, load_credit_grants_tx,
    reservation_identity_tx, reserved_cents_tx,
};
use spend_limits::enforce_spend_limits_tx;
use usage_reconciliation::record_unknown_usage_tx;

#[derive(Debug, Clone)]
pub struct PostgresAuthorityStore {
    pool: Pool<Postgres>,
}

impl PostgresAuthorityStore {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &Pool<Postgres> {
        &self.pool
    }
}

#[async_trait]
impl AuthorityStore for PostgresAuthorityStore {
    async fn reserve(&self, request: ReserveRequest) -> Result<ReserveResponse, AuthorityError> {
        let now = Utc::now();
        request
            .grant
            .validate_against_delegation(&request.delegation, now)
            .map_err(|err| AuthorityError::Forbidden(err.to_string()))?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let live_config = enforce_live_grant_config_tx(&mut tx, &request).await?;

        if let Some(existing) = sqlx::query(
            r#"
            select id, run_grant_jti, relay_delegation_jti, billing_subject_id, ctx_user_id,
                   ctx_org_id, route_id, provider_id, model_id, pricing_version,
                   max_estimated_cents, max_input_tokens, max_output_tokens
            from public.usage_reservations
            where request_id = $1
            "#,
        )
        .bind(&request.grant.request_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|err| AuthorityError::Store(err.to_string()))?
        {
            if !existing_reservation_matches(&existing, &request, live_config.reservation_cents)? {
                return Err(AuthorityError::Conflict(
                    "request id was already reserved with different grant".to_string(),
                ));
            }
            let state = latest_state_tx(&mut tx, &request.grant.request_id).await?;
            if state == Some(RelayRequestState::Reserved) {
                let reservation_id: String = existing
                    .try_get("id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?;
                let reserved_cents = reserved_cents_tx(&mut tx, &reservation_id).await?;
                tx.commit()
                    .await
                    .map_err(|err| AuthorityError::Store(err.to_string()))?;
                return Ok(ReserveResponse {
                    request_id: request.grant.request_id,
                    reservation_id,
                    state: RelayRequestState::Reserved,
                    reserved_cents,
                    idempotent: true,
                });
            }
            return Err(AuthorityError::Conflict(
                "request id was already used after provider start".to_string(),
            ));
        }

        enforce_spend_limits_tx(
            &mut tx,
            &request.grant.billing_subject_id,
            &request.grant.ctx_user_id,
            live_config.reservation_cents,
            now,
        )
        .await?;
        let grants = load_credit_grants_tx(&mut tx, &request.grant.billing_subject_id).await?;
        let reservation = allocate_credit_reservation(&grants, live_config.reservation_cents, now)
            .map_err(|err| AuthorityError::InsufficientCredits(err.to_string()))?;
        let reservation_id = format!("res_{}", Uuid::new_v4());
        let expires_at = request.grant.expires_at;
        sqlx::query(
            r#"
            insert into public.usage_reservations
              (id, request_id, run_grant_jti, relay_delegation_jti, billing_subject_id,
               ctx_user_id, ctx_org_id, route_id, provider_id, model_id, pricing_version,
               max_estimated_cents, max_input_tokens, max_output_tokens, expires_at)
            values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            "#,
        )
        .bind(&reservation_id)
        .bind(&request.grant.request_id)
        .bind(&request.grant.jti)
        .bind(&request.grant.delegation_jti)
        .bind(&request.grant.billing_subject_id)
        .bind(&request.grant.ctx_user_id)
        .bind(&request.grant.ctx_org_id)
        .bind(&request.grant.route_id)
        .bind(&request.grant.provider_id)
        .bind(&request.grant.model_id)
        .bind(&request.grant.pricing_version)
        .bind(checked_i64(live_config.reservation_cents)?)
        .bind(request.grant.max_input_tokens.map(i64::from))
        .bind(request.grant.max_output_tokens.map(i64::from))
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .map_err(|err| AuthorityError::Store(err.to_string()))?;

        for allocation in &reservation.allocations {
            insert_allocation_tx(&mut tx, &reservation_id, allocation).await?;
            insert_credit_event_tx(
                &mut tx,
                &request.grant.billing_subject_id,
                Some(&allocation.grant_id),
                Some(&request.grant.request_id),
                Some(&reservation_id),
                "reserve",
                -checked_i64(allocation.reserved_cents)?,
            )
            .await?;
        }
        insert_state_event_tx(
            &mut tx,
            &request.grant.request_id,
            None,
            RelayRequestState::Reserved,
            "authority",
            None,
        )
        .await?;
        insert_usage_event_tx(
            &mut tx,
            &request.grant.request_id,
            Some(&reservation_id),
            &request.grant.billing_subject_id,
            "reserved",
            Some(&request.grant.provider_id),
            Some(&request.grant.model_id),
            Some(&request.grant.route_id),
            None,
            None,
            None,
            None,
            None,
            None,
            Some(live_config.reservation_cents),
        )
        .await?;
        tx.commit()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;

        Ok(ReserveResponse {
            request_id: request.grant.request_id,
            reservation_id,
            state: RelayRequestState::Reserved,
            reserved_cents: reservation.total_reserved_cents(),
            idempotent: false,
        })
    }

    async fn provider_started(
        &self,
        request: ProviderStartedRequest,
    ) -> Result<RequestSnapshot, AuthorityError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let previous = latest_state_tx(&mut tx, &request.request_id)
            .await?
            .ok_or(AuthorityError::NotFound)?;
        if previous == RelayRequestState::ProviderStarted {
            if request.provider_request_id.is_some() {
                let reservation = reservation_identity_tx(&mut tx, &request.request_id).await?;
                insert_usage_event_tx(
                    &mut tx,
                    &request.request_id,
                    Some(&reservation.reservation_id),
                    &reservation.billing_subject_id,
                    "provider_started",
                    reservation.provider_id.as_deref(),
                    reservation.model_id.as_deref(),
                    reservation.route_id.as_deref(),
                    request.provider_request_id.as_deref(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            }
            tx.commit()
                .await
                .map_err(|err| AuthorityError::Store(err.to_string()))?;
            return self.get_request(&request.request_id).await;
        }
        ensure_transition(&previous, &RelayRequestState::ProviderStarted)?;
        insert_state_event_tx(
            &mut tx,
            &request.request_id,
            Some(previous),
            RelayRequestState::ProviderStarted,
            "worker",
            None,
        )
        .await?;
        let reservation = reservation_identity_tx(&mut tx, &request.request_id).await?;
        insert_usage_event_tx(
            &mut tx,
            &request.request_id,
            Some(&reservation.reservation_id),
            &reservation.billing_subject_id,
            "provider_started",
            reservation.provider_id.as_deref(),
            reservation.model_id.as_deref(),
            reservation.route_id.as_deref(),
            request.provider_request_id.as_deref(),
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        self.get_request(&request.request_id).await
    }

    async fn finalize(&self, request: FinalizeRequest) -> Result<RequestSnapshot, AuthorityError> {
        let now = Utc::now();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let previous = latest_state_tx(&mut tx, &request.request_id)
            .await?
            .ok_or(AuthorityError::NotFound)?;
        if !matches!(
            previous,
            RelayRequestState::ProviderStarted
                | RelayRequestState::StreamCompleted
                | RelayRequestState::StreamBrokenUsageUnknown
        ) {
            return Err(AuthorityError::Conflict(
                "finalize requires provider-started or stream-complete state".to_string(),
            ));
        }
        let reservation = reservation_identity_tx(&mut tx, &request.request_id).await?;
        if request.usage_unknown
            || request.actual_input_tokens.is_none()
            || request.actual_output_tokens.is_none()
        {
            record_unknown_usage_tx(&mut tx, &request, &reservation, previous).await?;
            tx.commit()
                .await
                .map_err(|err| AuthorityError::Store(err.to_string()))?;
            return self.get_request(&request.request_id).await;
        }
        let allocations = load_allocations_tx(&mut tx, &reservation.reservation_id).await?;
        let credit_reservation = CreditReservation {
            requested_cents: allocations
                .iter()
                .map(|allocation| allocation.reserved_cents)
                .sum(),
            allocations,
        };
        let billable_cents = billable_cents_tx(&mut tx, &reservation, &request).await?;
        let settlements = finalize_credit_reservation(&credit_reservation, billable_cents, now)
            .map_err(|err| AuthorityError::Conflict(err.to_string()))?;
        for settlement in settlements {
            if settlement.released_spendable_cents > 0 {
                insert_credit_event_tx(
                    &mut tx,
                    &reservation.billing_subject_id,
                    Some(&settlement.grant_id),
                    Some(&request.request_id),
                    Some(&reservation.reservation_id),
                    "release",
                    checked_i64(settlement.released_spendable_cents)?,
                )
                .await?;
            }
        }
        let stream_state = RelayRequestState::StreamCompleted;
        let final_from_state = if previous == RelayRequestState::ProviderStarted {
            stream_state.clone()
        } else {
            previous.clone()
        };
        if previous == RelayRequestState::ProviderStarted {
            insert_state_event_tx(
                &mut tx,
                &request.request_id,
                Some(previous),
                stream_state.clone(),
                "worker",
                None,
            )
            .await?;
        }
        insert_state_event_tx(
            &mut tx,
            &request.request_id,
            Some(final_from_state),
            RelayRequestState::Finalized,
            "authority",
            None,
        )
        .await?;
        insert_usage_event_tx(
            &mut tx,
            &request.request_id,
            Some(&reservation.reservation_id),
            &reservation.billing_subject_id,
            state_to_str(&stream_state),
            reservation.provider_id.as_deref(),
            reservation.model_id.as_deref(),
            reservation.route_id.as_deref(),
            request.provider_request_id.as_deref(),
            request.actual_input_tokens.map(i64::from),
            request.actual_output_tokens.map(i64::from),
            request.provider_cost_micros,
            Some(billable_cents),
            None,
            None,
        )
        .await?;
        insert_usage_event_tx(
            &mut tx,
            &request.request_id,
            Some(&reservation.reservation_id),
            &reservation.billing_subject_id,
            "finalized",
            reservation.provider_id.as_deref(),
            reservation.model_id.as_deref(),
            reservation.route_id.as_deref(),
            request.provider_request_id.as_deref(),
            request.actual_input_tokens.map(i64::from),
            request.actual_output_tokens.map(i64::from),
            request.provider_cost_micros,
            Some(billable_cents),
            None,
            None,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        self.get_request(&request.request_id).await
    }

    async fn void(&self, request: VoidRequest) -> Result<RequestSnapshot, AuthorityError> {
        let now = Utc::now();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let previous = latest_state_tx(&mut tx, &request.request_id)
            .await?
            .ok_or(AuthorityError::NotFound)?;
        if previous != RelayRequestState::Reserved {
            return Err(AuthorityError::Conflict(
                "void is only allowed before provider start".to_string(),
            ));
        }
        let reservation = reservation_identity_tx(&mut tx, &request.request_id).await?;
        let allocations = load_allocations_tx(&mut tx, &reservation.reservation_id).await?;
        let credit_reservation = CreditReservation {
            requested_cents: allocations
                .iter()
                .map(|allocation| allocation.reserved_cents)
                .sum(),
            allocations,
        };
        for settlement in release_credit_reservation(&credit_reservation, now) {
            if settlement.released_spendable_cents > 0 {
                insert_credit_event_tx(
                    &mut tx,
                    &reservation.billing_subject_id,
                    Some(&settlement.grant_id),
                    Some(&request.request_id),
                    Some(&reservation.reservation_id),
                    "release",
                    checked_i64(settlement.released_spendable_cents)?,
                )
                .await?;
            }
        }
        insert_state_event_tx(
            &mut tx,
            &request.request_id,
            Some(previous),
            RelayRequestState::Voided,
            "authority",
            request.reason.as_deref(),
        )
        .await?;
        insert_usage_event_tx(
            &mut tx,
            &request.request_id,
            Some(&reservation.reservation_id),
            &reservation.billing_subject_id,
            "voided",
            reservation.provider_id.as_deref(),
            reservation.model_id.as_deref(),
            reservation.route_id.as_deref(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;
        tx.commit()
            .await
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        self.get_request(&request.request_id).await
    }

    async fn get_request(&self, request_id: &str) -> Result<RequestSnapshot, AuthorityError> {
        let row = sqlx::query(
            r#"
            select r.id, r.request_id, r.billing_subject_id, r.ctx_user_id, r.ctx_org_id,
                   r.route_id, r.provider_id, r.model_id,
                   coalesce(sum(a.allocated_cents), 0)::bigint as reserved_cents
            from public.usage_reservations r
            left join public.usage_reservation_allocations a on a.reservation_id = r.id
            where r.request_id = $1
            group by r.id
            "#,
        )
        .bind(request_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| AuthorityError::Store(err.to_string()))?
        .ok_or(AuthorityError::NotFound)?;
        let state_events = load_state_events_pool(&self.pool, request_id).await?;
        let state = state_events
            .last()
            .map(|event| event.to_state.clone())
            .ok_or(AuthorityError::NotFound)?;
        Ok(RequestSnapshot {
            request_id: row
                .try_get("request_id")
                .map_err(|err| AuthorityError::Store(err.to_string()))?,
            reservation_id: Some(
                row.try_get("id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            ),
            billing_subject_id: Some(
                row.try_get("billing_subject_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            ),
            ctx_user_id: Some(
                row.try_get("ctx_user_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            ),
            ctx_org_id: row
                .try_get("ctx_org_id")
                .map_err(|err| AuthorityError::Store(err.to_string()))?,
            route_id: Some(
                row.try_get("route_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            ),
            provider_id: Some(
                row.try_get("provider_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            ),
            model_id: Some(
                row.try_get("model_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            ),
            state,
            reserved_cents: cents_from_i64(
                row.try_get("reserved_cents")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            )?,
            provider_request_id: latest_provider_request_id_pool(&self.pool, request_id).await?,
            updated_at: Utc::now(),
            state_events,
        })
    }
}
