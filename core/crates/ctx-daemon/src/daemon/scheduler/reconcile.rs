#[cfg(any(test, feature = "test-support"))]
mod provider_exit;
mod terminal_state;

#[cfg(any(test, feature = "test-support"))]
pub(in crate::daemon) use provider_exit::reconcile_turn_failed_on_provider_exit_with_host;
pub(in crate::daemon) use terminal_state::{
    reconcile_turn_terminal_state_with_host, DaemonTerminalStateReconcileHost,
    TerminalStateReconcileHost,
};
