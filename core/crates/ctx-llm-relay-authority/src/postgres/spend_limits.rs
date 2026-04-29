use chrono::{DateTime, Utc};
use sqlx::{Postgres, Row};

use crate::store::AuthorityError;

use super::conversions::cents_from_i64;

pub(super) async fn enforce_spend_limits_tx(
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
