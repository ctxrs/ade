use std::sync::Weak;
use std::time::Instant;

use tokio::sync::mpsc;

use ctx_core::ids::MessageId;
use ctx_core::models::{Message, Session};
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;

use crate::daemon::DaemonState;

mod lifecycle;
mod persistence;
mod reconcile;
mod runtime;
mod terminal;
mod worker;

pub use lifecycle::TurnStartProgress;
pub use reconcile::{reconcile_turn_failed_on_provider_exit, reconcile_turn_terminal_state};

#[derive(Debug)]
pub struct QueuedMessage {
    pub message: Message,
    pub enqueued_at: Instant,
    pub run_id: Option<String>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SchedulerCommand {
    Enqueue(QueuedMessage),
    RemoveQueued(MessageId),
    Cancel,
    Interrupt(InterruptTelemetryContext),
    StorageEmergency,
}

pub async fn session_worker(
    state_weak: Weak<DaemonState>,
    session: Session,
    rx: mpsc::Receiver<SchedulerCommand>,
) {
    worker::session_worker(state_weak, session, rx).await;
}
