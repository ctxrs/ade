use std::sync::Mutex;

use super::policy::now_ms;

mod heartbeat;
mod incidents;
mod model;
mod snapshot;

pub(super) use model::{HeartbeatTimeoutEvaluation, PreparedRecoveryIncident};
use model::DesktopWebviewRecoveryState;

#[derive(Debug, Default)]
pub(crate) struct DesktopWebviewRecoveryController {
    inner: Mutex<DesktopWebviewRecoveryState>,
}

#[cfg(test)]
mod tests;
