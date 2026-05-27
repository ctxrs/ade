use std::sync::Arc;

use tokio::sync::broadcast;

use super::{route_capabilities::DaemonShutdownSignal, state::DaemonState};

#[derive(Clone)]
pub struct DaemonHandle {
    state: Arc<DaemonState>,
}

impl DaemonHandle {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.state.core.shutdown_tx.subscribe()
    }

    pub fn shutdown_signal(&self) -> DaemonShutdownSignal {
        DaemonShutdownSignal::new(self.state.core.shutdown_tx.clone())
    }
}
