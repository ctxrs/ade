use sqlx::{Postgres, Row};

use crate::api::{FinalizeRequest, ReserveRequest};
use crate::store::AuthorityError;

use super::conversions::cents_for_tokens;
use super::reservations::ReservationIdentity;

pub(super) struct LiveGrantConfig {
    pub(super) reservation_cents: u64,
}

pub(super) async fn enforce_live_grant_config_tx(
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
        select route_id
        from ctx.route_configs
        where route_id = $1
          and billing_subject_id = $2::uuid
          and route_type = 'ctx_managed'
          and credential_owner = 'ctx'
          and auth_method = 'ctx_provider_key'
          and governance_level = 'hard_enforced'
          and status = 'enabled'
        "#,
    )
    .bind(&request.grant.route_id)
    .bind(&request.grant.billing_subject_id)
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
        select policy_version
        from ctx.route_policy_versions
        where billing_subject_id = $1::uuid
          and route_id = $2
          and policy_version = $3
          and status = 'active'
          and policy_document @> jsonb_build_object(
            'allowed_provider_model_pairs',
            jsonb_build_array(jsonb_build_object('provider_id', $4::text, 'model_id', $5::text))
          )
        "#,
    )
    .bind(&request.grant.billing_subject_id)
    .bind(&request.grant.route_id)
    .bind(&request.grant.policy_version)
    .bind(&request.grant.provider_id)
    .bind(&request.grant.model_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AuthorityError::Store(err.to_string()))?;
    if policy.is_none() {
        return Err(AuthorityError::Forbidden(
            "policy version is not active for the requested route/provider/model".to_string(),
        ));
    }

    let price = sqlx::query(
        r#"
        select p.id,
               p.input_microusd_per_1k_tokens,
               p.output_microusd_per_1k_tokens,
               p.request_overhead_cents
        from ctx.pricing_catalog_versions c
        join ctx.model_prices p on p.pricing_version = c.pricing_version
        where c.pricing_version = $1
          and c.status = 'active'
          and p.provider_id = $2
          and p.model_id = $3
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
            .try_get::<i64, _>("input_microusd_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative input price".to_string()))?;
    let output_price = u64::try_from(
        price
            .try_get::<i64, _>("output_microusd_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative output price".to_string()))?;
    let request_overhead_cents = u64::try_from(
        price
            .try_get::<i32, _>("request_overhead_cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative request overhead cents".to_string()))?;
    let token_cents = cents_for_tokens(
        estimated_input_tokens,
        estimated_output_tokens,
        input_price,
        output_price,
    )?;
    let reservation_cents = token_cents
        .checked_add(request_overhead_cents)
        .ok_or_else(|| AuthorityError::BadRequest("reservation cents overflow".to_string()))?;
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

pub(super) async fn billable_cents_tx(
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
        select input_microusd_per_1k_tokens, output_microusd_per_1k_tokens, request_overhead_cents
        from ctx.model_prices
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
        row.try_get::<i64, _>("input_microusd_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative input price".to_string()))?;
    let output_micros_per_1k = u64::try_from(
        row.try_get::<i64, _>("output_microusd_per_1k_tokens")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative output price".to_string()))?;
    let request_overhead_cents = u64::try_from(
        row.try_get::<i32, _>("request_overhead_cents")
            .map_err(|err| AuthorityError::Store(err.to_string()))?,
    )
    .map_err(|_| AuthorityError::Store("negative request overhead cents".to_string()))?;
    let token_cents = cents_for_tokens(
        input_tokens,
        output_tokens,
        input_micros_per_1k,
        output_micros_per_1k,
    )?;
    let cents = token_cents
        .checked_add(request_overhead_cents)
        .ok_or_else(|| AuthorityError::BadRequest("billable cents overflow".to_string()))?;
    Ok(cents.min(reservation.max_estimated_cents))
}
