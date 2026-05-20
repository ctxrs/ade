use std::sync::Weak;

use tokio::sync::mpsc;

use ctx_core::models::Session;
pub use ctx_run_scheduler::{QueuedMessage, SchedulerCommand};

use crate::daemon::DaemonState;

mod lifecycle;
mod persistence;
mod reconcile;
mod runtime;
mod terminal;
mod worker;

pub use lifecycle::TurnStartProgress;
pub use reconcile::{reconcile_turn_failed_on_provider_exit, reconcile_turn_terminal_state};

pub async fn session_worker(
    state_weak: Weak<DaemonState>,
    session: Session,
    rx: mpsc::Receiver<SchedulerCommand>,
) {
    worker::session_worker(state_weak, session, rx).await;
}
