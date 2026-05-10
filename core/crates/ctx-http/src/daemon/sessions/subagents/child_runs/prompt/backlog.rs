use ctx_core::models::SessionTurnStatus;

pub(super) fn turn_status_has_input_backlog(status: &SessionTurnStatus) -> bool {
    matches!(
        status,
        SessionTurnStatus::Queued | SessionTurnStatus::Starting | SessionTurnStatus::Running
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_backlog_includes_starting_turns() {
        assert!(turn_status_has_input_backlog(&SessionTurnStatus::Starting));
        assert!(turn_status_has_input_backlog(&SessionTurnStatus::Running));
        assert!(turn_status_has_input_backlog(&SessionTurnStatus::Queued));
        assert!(!turn_status_has_input_backlog(
            &SessionTurnStatus::Completed
        ));
    }
}
