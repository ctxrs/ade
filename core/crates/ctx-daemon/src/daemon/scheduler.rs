use std::sync::Weak;

use tokio::sync::mpsc;

use ctx_core::models::Session;
pub use ctx_run_scheduler::{QueuedMessage, SchedulerCommand};

mod host;
mod lifecycle;
mod persistence;
mod reconcile;
mod runtime;
mod terminal;
mod worker;

pub(in crate::daemon) use host::{SessionSchedulerWorkerHost, SessionSchedulerWorkerHostParts};
pub use lifecycle::TurnStartProgress;
#[cfg(test)]
pub(in crate::daemon) use persistence::DaemonSchedulerPersistenceHost;
#[cfg(any(test, feature = "test-support"))]
pub(in crate::daemon) use reconcile::reconcile_turn_failed_on_provider_exit_with_host;
pub(in crate::daemon) use reconcile::{
    reconcile_turn_terminal_state_with_host, DaemonTerminalStateReconcileHost,
    TerminalStateReconcileHost,
};

pub(in crate::daemon) async fn session_worker(
    host_weak: Weak<SessionSchedulerWorkerHost>,
    session: Session,
    rx: mpsc::Receiver<SchedulerCommand>,
) {
    worker::session_worker(host_weak, session, rx).await;
}
