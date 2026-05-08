use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant as TokioInstant;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_providers::adapters::{ProviderAdapter, RunHandle};
use ctx_providers::events::NormalizedEvent;

pub(crate) struct RunningTurn {
    pub(crate) adapter: Arc<dyn ProviderAdapter>,
    pub(crate) handle: RunHandle,
    pub(crate) run_id: RunId,
    pub(crate) turn_id: TurnId,
    pub(crate) message_id: MessageId,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) execution_environment_label: String,
    pub(crate) session_root_kind: String,
    pub(crate) event_tx: mpsc::Sender<NormalizedEvent>,
    pub(crate) events_done: Option<oneshot::Receiver<()>>,
    pub(crate) start_progress: watch::Receiver<TurnStartProgress>,
    pub(crate) start_deadline: TokioInstant,
    pub(crate) mcp_token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnStartProgress {
    Pending,
    Started,
    Terminal,
}

#[derive(Clone, Copy)]
pub(crate) enum StopReason {
    Cancel,
    Interrupt,
    StorageEmergency,
}
