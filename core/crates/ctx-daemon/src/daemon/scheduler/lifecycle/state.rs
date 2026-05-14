use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant as TokioInstant;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_providers::adapters::{ProviderAdapter, RunHandle};
use ctx_providers::events::NormalizedEvent;

pub struct RunningTurn {
    pub adapter: Arc<dyn ProviderAdapter>,
    pub handle: RunHandle,
    pub run_id: RunId,
    pub turn_id: TurnId,
    pub message_id: MessageId,
    pub provider_id: String,
    pub model_id: String,
    pub execution_environment_label: String,
    pub session_root_kind: String,
    pub event_tx: mpsc::Sender<NormalizedEvent>,
    pub events_done: Option<oneshot::Receiver<()>>,
    pub start_progress: watch::Receiver<TurnStartProgress>,
    pub start_deadline: TokioInstant,
    pub mcp_token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnStartProgress {
    Pending,
    Started,
    Terminal,
}

#[derive(Clone, Copy)]
pub enum StopReason {
    Cancel,
    Interrupt,
    StorageEmergency,
}
