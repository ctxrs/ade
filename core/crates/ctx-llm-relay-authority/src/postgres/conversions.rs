use ctx_llm_relay_contract::{CreditGrantSource, RelayRequestState};

use crate::store::AuthorityError;

pub(super) fn ensure_transition(
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

pub(super) fn checked_i64(value: u64) -> Result<i64, AuthorityError> {
    i64::try_from(value).map_err(|_| AuthorityError::BadRequest("amount exceeds i64".to_string()))
}

pub(super) fn cents_from_i64(value: i64) -> Result<u64, AuthorityError> {
    u64::try_from(value).map_err(|_| AuthorityError::Store("negative cents value".to_string()))
}

pub(super) fn cents_for_tokens(
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

pub(super) fn credit_source_from_str(value: &str) -> Result<CreditGrantSource, AuthorityError> {
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

pub(super) fn credit_source_to_str(value: &CreditGrantSource) -> &'static str {
    match value {
        CreditGrantSource::PromoTrial => "promo_trial",
        CreditGrantSource::SubscriptionIncluded => "subscription_included",
        CreditGrantSource::EnterpriseCommit => "enterprise_commit",
        CreditGrantSource::PrepaidTopUp => "prepaid_top_up",
    }
}

pub(super) fn state_from_str(value: &str) -> Result<RelayRequestState, AuthorityError> {
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

pub(super) fn state_to_str(value: &RelayRequestState) -> &'static str {
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
