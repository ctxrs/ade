use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ctx_llm_relay_contract::{
    allocate_credit_reservation, finalize_credit_reservation, release_credit_reservation,
    CreditAllocation, CreditGrant, CreditGrantSource, CreditReservation, RelayRequestState,
};
use sqlx::{postgres::PgRow, Pool, Postgres, Row};
use uuid::Uuid;

use crate::api::{
    FinalizeRequest, ProviderStartedRequest, RequestSnapshot, ReserveRequest, ReserveResponse,
    VoidRequest,
};
use crate::store::{AuthorityError, AuthorityStore, StateEvent};

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

struct ReservationIdentity {
    reservation_id: String,
    billing_subject_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    route_id: Option<String>,
    pricing_version: String,
    max_estimated_cents: u64,
}

struct LiveGrantConfig {
    reservation_cents: u64,
}

fn existing_reservation_matches(
    row: &PgRow,
    request: &ReserveRequest,
    reservation_cents: u64,
) -> Result<bool, AuthorityError> {
    let max_estimated_cents = cents_from_i64(
        row.try_get("max_estimated_cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )?;
    let max_input_tokens: Option<i64> = row
        .try_get("max_input_tokens")
        .map_err(|err| AuthorityError::Store(err.to_string()))?;
    let max_output_tokens: Option<i64> = row
        .try_get("max_output_tokens")
        .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(row
        .try_get::<String, _>("run_grant_jti")
        .map_err(|err| AuthorityError::Store(err.to_string()))?
        == request.grant.jti
        && row
            .try_get::<String, _>("relay_delegation_jti")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.delegation_jti
        && row
            .try_get::<String, _>("billing_subject_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.billing_subject_id
        && row
            .try_get::<String, _>("ctx_user_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.ctx_user_id
        && row
            .try_get::<Option<String>, _>("ctx_org_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.ctx_org_id
        && row
            .try_get::<String, _>("route_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.route_id
        && row
            .try_get::<String, _>("provider_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.provider_id
        && row
            .try_get::<String, _>("model_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.model_id
        && row
            .try_get::<String, _>("pricing_version")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.pricing_version
        && max_estimated_cents == reservation_cents
        && max_input_tokens == request.grant.max_input_tokens.map(i64::from)
        && max_output_tokens == request.grant.max_output_tokens.map(i64::from))
}

async fn latest_state_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
) -> Result<Option<RelayRequestState>, AuthorityError> {
    let row = sqlx::query(
        r#"
        select to_state::text as state
        from public.request_state_events
        where request_id = $1
        order by created_at desc, id desc
        limit 1
        "#,
    )
    .bind(request_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    row.map(|row| {
        let state: String = row
            .try_get("state")
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        state_from_str(&state)
    })
    .transpose()
}

async fn reserved_cents_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
) -> Result<u64, AuthorityError> {
    let row = sqlx::query(
        "select coalesce(sum(allocated_cents), 0)::bigint as cents from public.usage_reservation_allocations where reservation_id = $1",
    )
    .bind(reservation_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    cents_from_i64(
        row.try_get("cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
}

async fn load_credit_grants_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    billing_subject_id: &str,
) -> Result<Vec<CreditGrant>, AuthorityError> {
    let rows = sqlx::query(
        r#"
        select g.id, g.billing_subject_id, g.source::text as source, g.total_cents,
               g.issued_at, g.expires_at,
               (
                 g.total_cents + coalesce(
                 (
                   select sum(e.amount_cents)::bigint
                   from public.credit_ledger_events e
                   where e.credit_grant_id = g.id
                 ),
                 0
                 )
               )::bigint as remaining_cents
        from public.credit_grants g
        where g.billing_subject_id = $1
        order by g.expires_at asc nulls last, g.issued_at asc, g.id asc
        for update
        "#,
    )
    .bind(billing_subject_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;

    rows.into_iter()
        .map(|row| {
            let source: String = row
                .try_get("source")
                .map_err(|err| AuthorityError::Store(err.to_string()))?;
            Ok(CreditGrant {
                grant_id: row
                    .try_get("id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
                billing_subject_id: row
                    .try_get("billing_subject_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
                source: credit_source_from_str(&source)?,
                total_cents: cents_from_i64(
                    row.try_get("total_cents")
                        .map_err(|err| AuthorityError::Store(err.to_string()))?,
                )?,
                remaining_cents: cents_from_i64(
                    row.try_get("remaining_cents")
                        .map_err(|err| AuthorityError::Store(err.to_string()))?,
                )?,
                issued_at: row
                    .try_get("issued_at")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
                expires_at: row
                    .try_get("expires_at")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            })
        })
        .collect()
}

async fn insert_allocation_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
    allocation: &CreditAllocation,
) -> Result<(), AuthorityError> {
    sqlx::query(
        r#"
        insert into public.usage_reservation_allocations
          (id, reservation_id, credit_grant_id, credit_source, allocated_cents, grant_expires_at)
        values ($1, $2, $3, $4::public.ctx_credit_source, $5, $6)
        "#,
    )
    .bind(format!("alloc_{}", Uuid::new_v4()))
    .bind(reservation_id)
    .bind(&allocation.grant_id)
    .bind(credit_source_to_str(&allocation.source))
    .bind(checked_i64(allocation.reserved_cents)?)
    .bind(allocation.grant_expires_at)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(())
}

async fn insert_credit_event_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    billing_subject_id: &str,
    credit_grant_id: Option<&str>,
    request_id: Option<&str>,
    reservation_id: Option<&str>,
    event_kind: &str,
    amount_cents: i64,
) -> Result<(), AuthorityError> {
    sqlx::query(
        r#"
        insert into public.credit_ledger_events
          (id, billing_subject_id, credit_grant_id, request_id, reservation_id, event_kind, amount_cents)
        values ($1, $2, $3, $4, $5, $6::public.ctx_credit_event_kind, $7)
        "#,
    )
    .bind(format!("cred_evt_{}", Uuid::new_v4()))
    .bind(billing_subject_id)
    .bind(credit_grant_id)
    .bind(request_id)
    .bind(reservation_id)
    .bind(event_kind)
    .bind(amount_cents)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(())
}

async fn insert_state_event_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
    from_state: Option<RelayRequestState>,
    to_state: RelayRequestState,
    actor: &str,
    reason: Option<&str>,
) -> Result<(), AuthorityError> {
    sqlx::query(
        r#"
        insert into public.request_state_events
          (id, request_id, from_state, to_state, actor, reason)
        values ($1, $2, $3::public.ctx_relay_request_state, $4::public.ctx_relay_request_state, $5, $6)
        "#,
    )
    .bind(format!("state_evt_{}", Uuid::new_v4()))
    .bind(request_id)
    .bind(from_state.as_ref().map(state_to_str))
    .bind(state_to_str(&to_state))
    .bind(actor)
    .bind(reason)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_usage_event_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
    reservation_id: Option<&str>,
    billing_subject_id: &str,
    event_kind: &str,
    provider_id: Option<&str>,
    model_id: Option<&str>,
    route_id: Option<&str>,
    provider_request_id: Option<&str>,
    actual_input_tokens: Option<i64>,
    actual_output_tokens: Option<i64>,
    provider_cost_micros: Option<u64>,
    billable_cents: Option<u64>,
    estimated_input_tokens: Option<i64>,
    estimated_output_tokens: Option<u64>,
) -> Result<(), AuthorityError> {
    sqlx::query(
        r#"
        insert into public.usage_ledger_events
          (id, request_id, reservation_id, billing_subject_id, event_kind, provider_id, model_id,
           route_id, provider_request_id, estimated_input_tokens, estimated_output_tokens,
           actual_input_tokens, actual_output_tokens, provider_cost_micros, billable_cents)
        values ($1, $2, $3, $4, $5::public.ctx_usage_event_kind, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        "#,
    )
    .bind(format!("usage_evt_{}", Uuid::new_v4()))
    .bind(request_id)
    .bind(reservation_id)
    .bind(billing_subject_id)
    .bind(event_kind)
    .bind(provider_id)
    .bind(model_id)
    .bind(route_id)
    .bind(provider_request_id)
    .bind(estimated_input_tokens)
    .bind(estimated_output_tokens.map(checked_i64).transpose()?)
    .bind(actual_input_tokens)
    .bind(actual_output_tokens)
    .bind(provider_cost_micros.map(checked_i64).transpose()?)
    .bind(billable_cents.map(checked_i64).transpose()?)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(())
}

async fn reservation_identity_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
) -> Result<ReservationIdentity, AuthorityError> {
    let row = sqlx::query(
        "select id, billing_subject_id, provider_id, model_id, route_id, pricing_version, max_estimated_cents from public.usage_reservations where request_id = $1",
    )
    .bind(request_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?
    .ok_or(AuthorityError::NotFound)?;
    Ok(ReservationIdentity {
        reservation_id: row
            .try_get("id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
        billing_subject_id: row
            .try_get("billing_subject_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
        provider_id: row
            .try_get("provider_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
        model_id: row
            .try_get("model_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
        route_id: row
            .try_get("route_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
        pricing_version: row
            .try_get("pricing_version")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
        max_estimated_cents: cents_from_i64(
            row.try_get("max_estimated_cents")
                .map_err(|err| AuthorityError::Store(err.to_string()))?,
        )?,
    })
}

async fn enforce_live_grant_config_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request: &ReserveRequest,
) -> Result<LiveGrantConfig, AuthorityError> {
    let estimated_input_tokens = request.estimated_input_tokens.ok_or_else(|| {
        AuthorityError::BadRequest("estimated_input_tokens is required".to_string())
    })?;
    let estimated_output_tokens = request.estimated_output_tokens.ok_or_else(|| {
        AuthorityError::BadRequest("estimated_output_tokens is required".to_string())
    })?;
    if request
        .grant
        .max_input_tokens
        .is_some_and(|max| estimated_input_tokens > max)
    {
        return Err(AuthorityError::Forbidden(
            "estimated input tokens exceed grant limit".to_string(),
        ));
    }
    if request
        .grant
        .max_output_tokens
        .is_some_and(|max| estimated_output_tokens > max)
    {
        return Err(AuthorityError::Forbidden(
            "estimated output tokens exceed grant limit".to_string(),
        ));
    }

    let route = sqlx::query(
        r#"
        select id
        from public.route_configs
        where id = $1
          and billing_subject_id = $2
          and route_type = 'ctx_managed'
          and credential_owner = 'ctx'
          and auth_method = 'ctx_provider_key'
          and governance_level = 'hard_enforced'
          and status = 'enabled'
          and provider_id = $3
          and (
            jsonb_array_length(model_allowlist) = 0
            or model_allowlist ? $4
            or model_allowlist @> jsonb_build_array(jsonb_build_object('model_id', $4::text))
            or model_allowlist @> jsonb_build_array(jsonb_build_object('provider_id', $3::text, 'model_id', $4::text))
          )
        for share
        "#,
    )
    .bind(&request.grant.route_id)
    .bind(&request.grant.billing_subject_id)
    .bind(&request.grant.provider_id)
    .bind(&request.grant.model_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    if route.is_none() {
        return Err(AuthorityError::Forbidden(
            "ctx-managed route is not enabled for the requested provider/model".to_string(),
        ));
    }

    let policy = sqlx::query(
        r#"
        select id
        from public.route_policy_versions
        where billing_subject_id = $1
          and policy_version = $2
          and is_active = true
        for share
        "#,
    )
    .bind(&request.grant.billing_subject_id)
    .bind(&request.grant.policy_version)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    if policy.is_none() {
        return Err(AuthorityError::Forbidden(
            "policy version is not active".to_string(),
        ));
    }

    let price = sqlx::query(
        r#"
        select p.id,
               p.customer_input_micros_per_1k_tokens,
               p.customer_output_micros_per_1k_tokens,
               p.request_overhead_input_tokens,
               p.function_schema_overhead_input_tokens
        from public.pricing_catalog_versions c
        join public.model_prices p on p.pricing_version = c.version
        where c.version = $1
          and c.status = 'active'
          and p.provider_id = $2
          and p.model_id = $3
        for share of c, p
        "#,
    )
    .bind(&request.grant.pricing_version)
    .bind(&request.grant.provider_id)
    .bind(&request.grant.model_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?
    .ok_or_else(|| {
        AuthorityError::Forbidden(
            "pricing version is not active for the requested provider/model".to_string(),
        )
    })?;
    let input_price = u64::try_from(
        price
            .try_get::<i64, _>("customer_input_micros_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative input price".to_string()))?;
    let output_price = u64::try_from(
        price
            .try_get::<i64, _>("customer_output_micros_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative output price".to_string()))?;
    let request_overhead = u32::try_from(
        price
            .try_get::<i32, _>("request_overhead_input_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative request overhead".to_string()))?;
    let function_overhead = u32::try_from(
        price
            .try_get::<i32, _>("function_schema_overhead_input_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative function overhead".to_string()))?;
    let priced_input_tokens = estimated_input_tokens
        .checked_add(request_overhead)
        .and_then(|value| value.checked_add(function_overhead))
        .ok_or_else(|| AuthorityError::BadRequest("estimated input tokens overflow".to_string()))?;
    let reservation_cents = cents_for_tokens(
        priced_input_tokens,
        estimated_output_tokens,
        input_price,
        output_price,
    )?;
    if reservation_cents == 0 {
        return Err(AuthorityError::Forbidden(
            "live pricing produced a zero-cent reservation".to_string(),
        ));
    }
    if reservation_cents > request.grant.max_estimated_cents {
        return Err(AuthorityError::Forbidden(
            "grant max_estimated_cents is below live pricing estimate".to_string(),
        ));
    }

    Ok(LiveGrantConfig { reservation_cents })
}

async fn enforce_spend_limits_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    billing_subject_id: &str,
    ctx_user_id: &str,
    requested_cents: u64,
    now: DateTime<Utc>,
) -> Result<(), AuthorityError> {
    let rows = sqlx::query(
        r#"
        select id, ctx_user_id, period_start, period_end, hard_limit_cents
        from public.billing_spend_limits
        where billing_subject_id = $1
          and status = 'active'
          and period_start <= $2
          and period_end > $2
          and (ctx_user_id is null or ctx_user_id = $3)
        order by ctx_user_id nulls first, id
        for update
        "#,
    )
    .bind(billing_subject_id)
    .bind(now)
    .bind(ctx_user_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;

    for row in rows {
        let limit_user_id: Option<String> = row
            .try_get("ctx_user_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let period_start: DateTime<Utc> = row
            .try_get("period_start")
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let period_end: DateTime<Utc> = row
            .try_get("period_end")
            .map_err(|err| AuthorityError::Store(err.to_string()))?;
        let hard_limit_cents = cents_from_i64(
            row.try_get("hard_limit_cents")
                .map_err(|err| AuthorityError::Store(err.to_string()))?,
        )?;
        let used_cents = spend_used_cents_tx(
            tx,
            billing_subject_id,
            limit_user_id.as_deref(),
            period_start,
            period_end,
        )
        .await?;
        if used_cents.saturating_add(requested_cents) > hard_limit_cents {
            return Err(AuthorityError::InsufficientCredits(
                "spend limit would be exceeded".to_string(),
            ));
        }
    }
    Ok(())
}

async fn spend_used_cents_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    billing_subject_id: &str,
    ctx_user_id: Option<&str>,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
) -> Result<u64, AuthorityError> {
    let row = sqlx::query(
        r#"
        with latest_state as (
          select distinct on (request_id)
                 request_id,
                 to_state::text as state
          from public.request_state_events
          order by request_id, created_at desc, id desc
        ),
        latest_finalized as (
          select distinct on (request_id)
                 request_id,
                 billable_cents
          from public.usage_ledger_events
          where event_kind = 'finalized'
            and billable_cents is not null
          order by request_id, created_at desc, id desc
        )
        select coalesce(sum(
          case
            when latest_state.state = 'voided' then 0
            when latest_state.state in ('finalized', 'reconciled') then coalesce(latest_finalized.billable_cents, 0)
            else reservations.max_estimated_cents
          end
        ), 0)::bigint as used_cents
        from public.usage_reservations reservations
        left join latest_state on latest_state.request_id = reservations.request_id
        left join latest_finalized on latest_finalized.request_id = reservations.request_id
        where reservations.billing_subject_id = $1
          and reservations.created_at >= $2
          and reservations.created_at < $3
          and ($4::text is null or reservations.ctx_user_id = $4)
        "#,
    )
    .bind(billing_subject_id)
    .bind(period_start)
    .bind(period_end)
    .bind(ctx_user_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    cents_from_i64(
        row.try_get("used_cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
}

async fn record_unknown_usage_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request: &FinalizeRequest,
    reservation: &ReservationIdentity,
    previous: RelayRequestState,
) -> Result<(), AuthorityError> {
    if previous == RelayRequestState::ProviderStarted {
        insert_state_event_tx(
            tx,
            &request.request_id,
            Some(previous),
            RelayRequestState::StreamBrokenUsageUnknown,
            "worker",
            None,
        )
        .await?;
        insert_usage_event_tx(
            tx,
            &request.request_id,
            Some(&reservation.reservation_id),
            &reservation.billing_subject_id,
            state_to_str(&RelayRequestState::StreamBrokenUsageUnknown),
            reservation.provider_id.as_deref(),
            reservation.model_id.as_deref(),
            reservation.route_id.as_deref(),
            request.provider_request_id.as_deref(),
            request.actual_input_tokens.map(i64::from),
            request.actual_output_tokens.map(i64::from),
            request.provider_cost_micros,
            None,
            None,
            None,
        )
        .await?;
        return Ok(());
    }
    if previous == RelayRequestState::StreamBrokenUsageUnknown {
        return Ok(());
    }
    Err(AuthorityError::Conflict(
        "unknown usage finalization requires provider-started state".to_string(),
    ))
}

async fn billable_cents_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation: &ReservationIdentity,
    request: &FinalizeRequest,
) -> Result<u64, AuthorityError> {
    if request.usage_unknown {
        return Err(AuthorityError::Conflict(
            "unknown usage cannot be finalized".to_string(),
        ));
    }
    let (Some(input_tokens), Some(output_tokens), Some(provider_id), Some(model_id)) = (
        request.actual_input_tokens,
        request.actual_output_tokens,
        reservation.provider_id.as_deref(),
        reservation.model_id.as_deref(),
    ) else {
        return Err(AuthorityError::Conflict(
            "finalization requires actual token counts".to_string(),
        ));
    };
    let row = sqlx::query(
        r#"
        select customer_input_micros_per_1k_tokens, customer_output_micros_per_1k_tokens
        from public.model_prices
        where pricing_version = $1
          and provider_id = $2
          and model_id = $3
        "#,
    )
    .bind(&reservation.pricing_version)
    .bind(provider_id)
    .bind(model_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?
    .ok_or_else(|| {
        AuthorityError::Store(format!(
            "missing model price for {provider_id}/{model_id} at {}",
            reservation.pricing_version
        ))
    })?;
    let input_micros_per_1k = u64::try_from(
        row.try_get::<i64, _>("customer_input_micros_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative input price".to_string()))?;
    let output_micros_per_1k = u64::try_from(
        row.try_get::<i64, _>("customer_output_micros_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative output price".to_string()))?;
    let cents = cents_for_tokens(
        input_tokens,
        output_tokens,
        input_micros_per_1k,
        output_micros_per_1k,
    )?;
    Ok(cents.min(reservation.max_estimated_cents))
}

async fn load_allocations_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
) -> Result<Vec<CreditAllocation>, AuthorityError> {
    let rows = sqlx::query(
        r#"
        select credit_grant_id, credit_source::text as credit_source, allocated_cents, grant_expires_at
        from public.usage_reservation_allocations
        where reservation_id = $1
        order by created_at asc, id asc
        "#,
    )
    .bind(reservation_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let source: String = row
                .try_get("credit_source")
                .map_err(|err| AuthorityError::Store(err.to_string()))?;
            Ok(CreditAllocation {
                grant_id: row
                    .try_get("credit_grant_id")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
                source: credit_source_from_str(&source)?,
                reserved_cents: cents_from_i64(
                    row.try_get("allocated_cents")
                        .map_err(|err| AuthorityError::Store(err.to_string()))?,
                )?,
                grant_expires_at: row
                    .try_get("grant_expires_at")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            })
        })
        .collect()
}

async fn load_state_events_pool(
    pool: &Pool<Postgres>,
    request_id: &str,
) -> Result<Vec<StateEvent>, AuthorityError> {
    let rows = sqlx::query(
        r#"
        select from_state::text as from_state, to_state::text as to_state, reason
        from public.request_state_events
        where request_id = $1
        order by created_at asc, id asc
        "#,
    )
    .bind(request_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    rows.into_iter()
        .map(|row| {
            let from_state: Option<String> = row
                .try_get("from_state")
                .map_err(|err| AuthorityError::Store(err.to_string()))?;
            let to_state: String = row
                .try_get("to_state")
                .map_err(|err| AuthorityError::Store(err.to_string()))?;
            Ok(StateEvent {
                request_id: request_id.to_string(),
                from_state: from_state.as_deref().map(state_from_str).transpose()?,
                to_state: state_from_str(&to_state)?,
                reason: row
                    .try_get("reason")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            })
        })
        .collect()
}

async fn latest_provider_request_id_pool(
    pool: &Pool<Postgres>,
    request_id: &str,
) -> Result<Option<String>, AuthorityError> {
    let row = sqlx::query(
        r#"
        select provider_request_id
        from public.usage_ledger_events
        where request_id = $1 and provider_request_id is not null
        order by created_at desc, id desc
        limit 1
        "#,
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    row.map(|row| {
        row.try_get("provider_request_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))
    })
    .transpose()
}

fn ensure_transition(
    previous: &RelayRequestState,
    next: &RelayRequestState,
) -> Result<(), AuthorityError> {
    if previous.can_transition_to(next) {
        Ok(())
    } else {
        Err(AuthorityError::Conflict(format!(
            "invalid state transition from {:?} to {:?}",
            previous, next
        )))
    }
}

fn checked_i64(value: u64) -> Result<i64, AuthorityError> {
    i64::try_from(value).map_err(|_| AuthorityError::BadRequest("amount exceeds i64".to_string()))
}

fn cents_from_i64(value: i64) -> Result<u64, AuthorityError> {
    u64::try_from(value).map_err(|_| AuthorityError::Store("negative cents value".to_string()))
}

fn cents_for_tokens(
    input_tokens: u32,
    output_tokens: u32,
    input_micros_per_1k: u64,
    output_micros_per_1k: u64,
) -> Result<u64, AuthorityError> {
    let numerator = u128::from(input_tokens) * u128::from(input_micros_per_1k)
        + u128::from(output_tokens) * u128::from(output_micros_per_1k);
    let cents = (numerator.saturating_add(9_999_999)) / 10_000_000;
    u64::try_from(cents)
        .map_err(|_| AuthorityError::BadRequest("billable cents exceed u64".to_string()))
}

fn credit_source_from_str(value: &str) -> Result<CreditGrantSource, AuthorityError> {
    match value {
        "promo_trial" => Ok(CreditGrantSource::PromoTrial),
        "subscription_included" => Ok(CreditGrantSource::SubscriptionIncluded),
        "enterprise_commit" => Ok(CreditGrantSource::EnterpriseCommit),
        "prepaid_top_up" => Ok(CreditGrantSource::PrepaidTopUp),
        _ => Err(AuthorityError::Store(format!(
            "unknown credit source {value}"
        ))),
    }
}

fn credit_source_to_str(value: &CreditGrantSource) -> &'static str {
    match value {
        CreditGrantSource::PromoTrial => "promo_trial",
        CreditGrantSource::SubscriptionIncluded => "subscription_included",
        CreditGrantSource::EnterpriseCommit => "enterprise_commit",
        CreditGrantSource::PrepaidTopUp => "prepaid_top_up",
    }
}

fn state_from_str(value: &str) -> Result<RelayRequestState, AuthorityError> {
    match value {
        "rejected_preflight" => Ok(RelayRequestState::RejectedPreflight),
        "reserved" => Ok(RelayRequestState::Reserved),
        "provider_started" => Ok(RelayRequestState::ProviderStarted),
        "stream_completed" => Ok(RelayRequestState::StreamCompleted),
        "stream_broken_usage_unknown" => Ok(RelayRequestState::StreamBrokenUsageUnknown),
        "finalized" => Ok(RelayRequestState::Finalized),
        "voided" => Ok(RelayRequestState::Voided),
        "reconciled" => Ok(RelayRequestState::Reconciled),
        _ => Err(AuthorityError::Store(format!(
            "unknown request state {value}"
        ))),
    }
}

fn state_to_str(value: &RelayRequestState) -> &'static str {
    match value {
        RelayRequestState::RejectedPreflight => "rejected_preflight",
        RelayRequestState::Reserved => "reserved",
        RelayRequestState::ProviderStarted => "provider_started",
        RelayRequestState::StreamCompleted => "stream_completed",
        RelayRequestState::StreamBrokenUsageUnknown => "stream_broken_usage_unknown",
        RelayRequestState::Finalized => "finalized",
        RelayRequestState::Voided => "voided",
        RelayRequestState::Reconciled => "reconciled",
    }
}
