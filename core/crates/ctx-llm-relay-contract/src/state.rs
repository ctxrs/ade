use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RelayRequestState {
    RejectedPreflight,
    Reserved,
    ProviderStarted,
    StreamCompleted,
    StreamBrokenUsageUnknown,
    Finalized,
    Voided,
    Reconciled,
}

impl RelayRequestState {
    pub fn can_transition_to(&self, next: &Self) -> bool {
        use RelayRequestState::{
            Finalized, ProviderStarted, Reconciled, RejectedPreflight, Reserved,
            StreamBrokenUsageUnknown, StreamCompleted, Voided,
        };

        match (self, next) {
            (RejectedPreflight, _) | (Voided, _) | (Reconciled, _) => false,
            (Reserved, ProviderStarted | Voided) => true,
            (ProviderStarted, StreamCompleted | StreamBrokenUsageUnknown | Voided) => true,
            (StreamCompleted, Finalized | Reconciled) => true,
            (StreamBrokenUsageUnknown, Finalized | Reconciled) => true,
            (Finalized, Reconciled) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RelayRequestState;

    #[test]
    fn relay_request_state_transitions_are_sane() {
        assert!(RelayRequestState::Reserved.can_transition_to(&RelayRequestState::ProviderStarted));
        assert!(RelayRequestState::ProviderStarted
            .can_transition_to(&RelayRequestState::StreamCompleted));
        assert!(RelayRequestState::StreamCompleted.can_transition_to(&RelayRequestState::Finalized));
        assert!(RelayRequestState::Finalized.can_transition_to(&RelayRequestState::Reconciled));

        assert!(
            !RelayRequestState::RejectedPreflight.can_transition_to(&RelayRequestState::Reserved)
        );
        assert!(!RelayRequestState::StreamCompleted.can_transition_to(&RelayRequestState::Reserved));
        assert!(!RelayRequestState::Voided.can_transition_to(&RelayRequestState::Reconciled));
    }
}
