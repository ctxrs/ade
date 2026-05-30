use ctx_llm_relay_contract::RelayRequestState;
use sqlx::{Pool, Postgres, Row};
use uuid::Uuid;

use crate::store::{AuthorityError, StateEvent};

use super::conversions::{checked_i64, state_from_str, state_to_str};

pub(super) async fn latest_state_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
) -> Result<Option<RelayRequestState>, AuthorityError> {
    let row = sqlx::query(
        r#"
        select to_state::text as state
        from ctx.request_state_events
        where request_id = $1
        order by occurred_at desc, event_id desc
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

pub(super) async fn insert_credit_event_tx(
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
        insert into ctx.credit_ledger_events
          (event_id, billing_subject_id, credit_grant_id, request_id, usage_reservation_id, event_type, amount_cents)
        values ($1, $2::uuid, $3::uuid, $4, $5::uuid, $6, $7)
        "#,
    )
    .bind(Uuid::new_v4())
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

pub(super) async fn insert_state_event_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
    reservation_id: Option<&str>,
    from_state: Option<RelayRequestState>,
    to_state: RelayRequestState,
    reason: Option<&str>,
) -> Result<(), AuthorityError> {
    sqlx::query(
        r#"
        insert into ctx.request_state_events
          (event_id, request_id, reservation_id, from_state, to_state, reason)
        values ($1, $2, $3::uuid, $4, $5, $6)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(reservation_id)
    .bind(from_state.as_ref().map(state_to_str))
    .bind(state_to_str(&to_state))
    .bind(reason)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(())
}

pub(super) async fn update_reservation_status_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
    state: RelayRequestState,
    provider_request_id: Option<&str>,
    actual_billable_cents: Option<u64>,
    finalized: bool,
) -> Result<(), AuthorityError> {
    let result = sqlx::query(
        r#"
        update ctx.usage_reservations
        set status = $2,
            provider_request_id = coalesce($3::text, provider_request_id),
            actual_billable_cents = coalesce($4::integer, actual_billable_cents),
            finalized_at = case when $5 then now() else finalized_at end,
            updated_at = now()
        where request_id = $1
        "#,
    )
    .bind(request_id)
    .bind(state_to_str(&state))
    .bind(provider_request_id)
    .bind(actual_billable_cents.map(checked_i64).transpose()?)
    .bind(finalized)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    if result.rows_affected() != 1 {
        return Err(AuthorityError::NotFound);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_usage_event_tx(
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
        insert into ctx.usage_ledger_events
          (event_id, request_id, reservation_id, event_type, amount_cents, input_tokens, output_tokens, metadata)
        values ($1, $2, $3::uuid, $4, $5, $6, $7, $8)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(reservation_id)
    .bind(event_kind)
    .bind(billable_cents.map(checked_i64).transpose()?)
    .bind(actual_input_tokens.or(estimated_input_tokens))
    .bind(
        actual_output_tokens
            .map(u64::try_from)
            .transpose()
            .map_err(|_| AuthorityError::Store("negative output token count".to_string()))?
            .or(estimated_output_tokens)
            .map(checked_i64)
            .transpose()?,
    )
    .bind(serde_json::json!({
        "billing_subject_id": billing_subject_id,
        "provider_id": provider_id,
        "model_id": model_id,
        "route_id": route_id,
        "provider_request_id": provider_request_id,
        "provider_cost_micros": provider_cost_micros,
        "estimated_input_tokens": estimated_input_tokens,
        "estimated_output_tokens": estimated_output_tokens,
    }))
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    Ok(())
}

pub(super) async fn load_state_events_pool(
    pool: &Pool<Postgres>,
    request_id: &str,
) -> Result<Vec<StateEvent>, AuthorityError> {
    let rows = sqlx::query(
        r#"
        select from_state::text as from_state, to_state::text as to_state, reason
        from ctx.request_state_events
        where request_id = $1
        order by occurred_at asc, event_id asc
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

pub(super) async fn latest_provider_request_id_pool(
    pool: &Pool<Postgres>,
    request_id: &str,
) -> Result<Option<String>, AuthorityError> {
    let row = sqlx::query(
        r#"
        select provider_request_id
        from ctx.usage_reservations
        where request_id = $1 and provider_request_id is not null
        order by updated_at desc, reservation_id desc
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
