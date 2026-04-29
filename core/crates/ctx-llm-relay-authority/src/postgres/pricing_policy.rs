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
