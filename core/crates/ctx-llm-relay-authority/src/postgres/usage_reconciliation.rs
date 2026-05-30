use ctx_llm_relay_contract::RelayRequestState;
use sqlx::Postgres;

use crate::api::FinalizeRequest;
use crate::store::AuthorityError;

use super::conversions::state_to_str;
use super::ledger::{insert_state_event_tx, insert_usage_event_tx, update_reservation_status_tx};
use super::reservations::ReservationIdentity;

pub(super) async fn record_unknown_usage_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    request: &FinalizeRequest,
    reservation: &ReservationIdentity,
    previous: RelayRequestState,
) -> Result<(), AuthorityError> {
    if previous == RelayRequestState::ProviderStarted {
        insert_state_event_tx(
            tx,
            &request.request_id,
            Some(&reservation.reservation_id),
            Some(previous),
            RelayRequestState::StreamBrokenUsageUnknown,
            None,
        )
        .await?;
        update_reservation_status_tx(
            tx,
            &request.request_id,
            RelayRequestState::StreamBrokenUsageUnknown,
            request.provider_request_id.as_deref(),
            None,
            false,
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
