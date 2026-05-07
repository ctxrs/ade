use ctx_core::models::MessageDelivery;

const QUEUED_MESSAGES_DISABLED_MESSAGE: &str = "Queued messages are disabled.";
const TURN_ALREADY_RUNNING_MESSAGE: &str =
    "A turn is already running. Stop it or wait for it to finish.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageDeliveryResolutionError {
    QueuedMessagesDisabled,
    TurnAlreadyRunning,
}

impl MessageDeliveryResolutionError {
    pub fn message(self) -> &'static str {
        match self {
            MessageDeliveryResolutionError::QueuedMessagesDisabled => {
                QUEUED_MESSAGES_DISABLED_MESSAGE
            }
            MessageDeliveryResolutionError::TurnAlreadyRunning => TURN_ALREADY_RUNNING_MESSAGE,
        }
    }
}

pub fn resolve_message_delivery(
    requested_delivery: Option<MessageDelivery>,
    session_running: bool,
    queued_enabled: bool,
) -> Result<MessageDelivery, MessageDeliveryResolutionError> {
    match requested_delivery {
        Some(MessageDelivery::Queued) if queued_enabled => Ok(MessageDelivery::Queued),
        Some(MessageDelivery::Queued) => {
            Err(MessageDeliveryResolutionError::QueuedMessagesDisabled)
        }
        None if session_running && queued_enabled => Ok(MessageDelivery::Queued),
        Some(MessageDelivery::Immediate) | None if session_running => {
            Err(MessageDeliveryResolutionError::TurnAlreadyRunning)
        }
        Some(MessageDelivery::Immediate) | None => Ok(MessageDelivery::Immediate),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_queueing_when_queue_feature_is_disabled() {
        assert!(matches!(
            resolve_message_delivery(None, true, false),
            Err(MessageDeliveryResolutionError::TurnAlreadyRunning)
        ));
        assert!(matches!(
            resolve_message_delivery(Some(MessageDelivery::Queued), false, false),
            Err(MessageDeliveryResolutionError::QueuedMessagesDisabled)
        ));
    }

    #[test]
    fn queues_only_when_feature_enabled() {
        assert!(matches!(
            resolve_message_delivery(None, true, true),
            Ok(MessageDelivery::Queued)
        ));
        assert!(matches!(
            resolve_message_delivery(Some(MessageDelivery::Queued), true, true),
            Ok(MessageDelivery::Queued)
        ));
        assert!(matches!(
            resolve_message_delivery(None, false, false),
            Ok(MessageDelivery::Immediate)
        ));
        assert!(matches!(
            resolve_message_delivery(Some(MessageDelivery::Immediate), true, true),
            Err(MessageDeliveryResolutionError::TurnAlreadyRunning)
        ));
    }

    #[test]
    fn errors_expose_user_facing_messages() {
        assert_eq!(
            MessageDeliveryResolutionError::QueuedMessagesDisabled.message(),
            "Queued messages are disabled."
        );
        assert_eq!(
            MessageDeliveryResolutionError::TurnAlreadyRunning.message(),
            "A turn is already running. Stop it or wait for it to finish."
        );
    }
}
