use ctx_llm_relay_contract::{CreditAllocation, CreditAllocationSettlement, CreditGrant};
use sqlx::{postgres::PgRow, Postgres, Row};
use uuid::Uuid;

use crate::api::ReserveRequest;
use crate::store::AuthorityError;

use super::conversions::{cents_from_i64, checked_i64, credit_source_from_str};

pub(super) struct ReservationIdentity {
    pub(super) reservation_id: String,
    pub(super) billing_subject_id: String,
    pub(super) provider_id: Option<String>,
    pub(super) model_id: Option<String>,
    pub(super) route_id: Option<String>,
    pub(super) pricing_version: String,
    pub(super) max_estimated_cents: u64,
}

pub(super) fn existing_reservation_matches(
    row: &PgRow,
    request: &ReserveRequest,
    reservation_cents: u64,
) -> Result<bool, AuthorityError> {
    let max_estimated_cents = cents_from_i64(
        row.try_get("max_estimated_cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )?;
    let reserved_cents = cents_from_i64(
        row.try_get("reserved_cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )?;
    Ok(row
        .try_get::<String, _>("run_grant_jti")
        .map_err(|err| AuthorityError::Store(err.to_string()))?
        == request.grant.jti
        && row
            .try_get::<String, _>("delegation_jti")
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
            .try_get::<Option<String>, _>("ctx_account_id")
            .map_err(|err| AuthorityError::Store(err.to_string()))?
            == request.grant.ctx_account_id
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
        && reserved_cents == reservation_cents)
}

pub(super) async fn reserved_cents_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
) -> Result<u64, AuthorityError> {
    let row = sqlx::query(
        "select coalesce(sum(reserved_cents), 0)::bigint as cents from ctx.usage_reservation_credit_allocations where reservation_id = $1::uuid",
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

pub(super) async fn load_credit_grants_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    billing_subject_id: &str,
) -> Result<Vec<CreditGrant>, AuthorityError> {
    let rows = sqlx::query(
        r#"
        select g.id::text as id, g.billing_subject_id::text as billing_subject_id,
               g.credit_source::text as source, g.original_cents::bigint as total_cents,
               g.remaining_cents::bigint as remaining_cents, g.created_at as issued_at,
               g.expires_at
        from ctx.credit_grants g
        where g.billing_subject_id = $1::uuid
          and g.active = true
          and g.remaining_cents > 0
        order by g.expires_at asc nulls last, g.created_at asc, g.id asc
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

pub(super) async fn insert_allocation_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
    allocation: &CreditAllocation,
) -> Result<(), AuthorityError> {
    sqlx::query(
        r#"
        insert into ctx.usage_reservation_credit_allocations
          (allocation_id, reservation_id, credit_grant_id, reserved_cents)
        values ($1, $2::uuid, $3::uuid, $4)
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(reservation_id)
    .bind(&allocation.grant_id)
    .bind(checked_i64(allocation.reserved_cents)?)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    let result = sqlx::query(
        r#"
        update ctx.credit_grants
        set remaining_cents = remaining_cents - $2
        where id = $1::uuid
          and remaining_cents >= $2
        "#,
    )
    .bind(&allocation.grant_id)
    .bind(checked_i64(allocation.reserved_cents)?)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    if result.rows_affected() != 1 {
        return Err(AuthorityError::Conflict(
            "credit grant balance changed before reservation allocation".to_string(),
        ));
    }
    Ok(())
}

pub(super) async fn settle_allocation_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
    settlement: &CreditAllocationSettlement,
) -> Result<(), AuthorityError> {
    let result = sqlx::query(
        r#"
        update ctx.usage_reservation_credit_allocations
        set debited_cents = debited_cents + $3,
            released_cents = released_cents + $4
        where reservation_id = $1::uuid
          and credit_grant_id = $2::uuid
          and debited_cents + released_cents + $3 + $4 <= reserved_cents
        "#,
    )
    .bind(reservation_id)
    .bind(&settlement.grant_id)
    .bind(checked_i64(settlement.finalized_cents)?)
    .bind(checked_i64(settlement.released_cents)?)
    .execute(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    if result.rows_affected() != 1 {
        return Err(AuthorityError::Conflict(
            "reservation allocation was already settled".to_string(),
        ));
    }
    if settlement.released_spendable_cents > 0 {
        sqlx::query(
            r#"
            update ctx.credit_grants
            set remaining_cents = remaining_cents + $2
            where id = $1::uuid
            "#,
        )
        .bind(&settlement.grant_id)
        .bind(checked_i64(settlement.released_spendable_cents)?)
        .execute(&mut **tx)
        .await
        .map_err(|err| AuthorityError::Store(err.to_string()))?;
    }
    Ok(())
}

pub(super) async fn reservation_identity_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request_id: &str,
) -> Result<ReservationIdentity, AuthorityError> {
    let row = sqlx::query(
        "select reservation_id::text as reservation_id, billing_subject_id::text as billing_subject_id, provider_id, model_id, route_id, pricing_version, max_estimated_cents::bigint as max_estimated_cents from ctx.usage_reservations where request_id = $1",
    )
    .bind(request_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?
    .ok_or(AuthorityError::NotFound)?;
    Ok(ReservationIdentity {
        reservation_id: row
            .try_get("reservation_id")
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

pub(super) async fn load_allocations_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    reservation_id: &str,
) -> Result<Vec<CreditAllocation>, AuthorityError> {
    let rows = sqlx::query(
        r#"
        select a.credit_grant_id::text as credit_grant_id,
               g.credit_source::text as credit_source,
               a.reserved_cents::bigint as reserved_cents,
               g.expires_at as grant_expires_at
        from ctx.usage_reservation_credit_allocations a
        join ctx.credit_grants g on g.id = a.credit_grant_id
        where a.reservation_id = $1::uuid
        order by a.created_at asc, a.allocation_id asc
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
                    row.try_get("reserved_cents")
                        .map_err(|err| AuthorityError::Store(err.to_string()))?,
                )?,
                grant_expires_at: row
                    .try_get("grant_expires_at")
                    .map_err(|err| AuthorityError::Store(err.to_string()))?,
            })
        })
        .collect()
}
