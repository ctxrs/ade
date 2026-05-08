use super::super::*;
use ctx_session_service::message_delivery::{
    resolve_message_delivery as resolve_message_delivery_policy, MessageDeliveryResolutionError,
};

const QUEUED_MESSAGES_ENABLED_ENV: &str = "CTX_QUEUED_MESSAGES_ENABLED";

fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .and_then(|v| match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
}

pub(super) fn queued_messages_enabled() -> bool {
    env_bool(QUEUED_MESSAGES_ENABLED_ENV).unwrap_or(false)
}

pub(super) fn resolve_message_delivery(
    requested_delivery: Option<MessageDelivery>,
    session_running: bool,
    queued_enabled: bool,
) -> Result<MessageDelivery, ApiErr> {
    resolve_message_delivery_policy(requested_delivery, session_running, queued_enabled).map_err(
        |error| match error {
            MessageDeliveryResolutionError::QueuedMessagesDisabled => {
                api_error(StatusCode::BAD_REQUEST, error.message())
            }
            MessageDeliveryResolutionError::TurnAlreadyRunning => {
                api_error(StatusCode::CONFLICT, error.message())
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_resolution_does_not_queue_when_queue_feature_is_disabled() {
        let err = resolve_message_delivery(None, true, false)
            .expect_err("running sessions should reject implicit queueing while disabled");
        let (status, Json(body)) = err;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(
            body.error,
            "A turn is already running. Stop it or wait for it to finish."
        );

        let err = resolve_message_delivery(Some(MessageDelivery::Queued), false, false)
            .expect_err("explicit queued delivery should be gated server-side");
        let (status, Json(body)) = err;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, "Queued messages are disabled.");
    }

    #[test]
    fn delivery_resolution_queues_only_when_feature_enabled() {
        assert!(matches!(
            resolve_message_delivery(None, true, true).expect("implicit queueing enabled"),
            MessageDelivery::Queued
        ));
        assert!(matches!(
            resolve_message_delivery(Some(MessageDelivery::Queued), true, true)
                .expect("explicit queueing enabled"),
            MessageDelivery::Queued
        ));
        assert!(matches!(
            resolve_message_delivery(None, false, false).expect("idle sessions send immediately"),
            MessageDelivery::Immediate
        ));

        let err = resolve_message_delivery(Some(MessageDelivery::Immediate), true, true)
            .expect_err("explicit immediate delivery must not be silently queued");
        let (status, _) = err;
        assert_eq!(status, StatusCode::CONFLICT);
    }
}
