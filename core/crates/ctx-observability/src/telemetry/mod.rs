mod event;
mod runtime;

use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

pub use ctx_settings_model::default_telemetry_endpoint;
pub use event::{
    TelemetryConfig, TelemetryDelivery, TelemetryEvent, TelemetryOriginRuntime, TelemetryPlane,
    TelemetryProperties,
};
#[cfg(test)]
pub(crate) use runtime::{load_or_create_install_id, telemetry_state_path, TelemetryStateFile};
use runtime::{telemetry_worker, TelemetryCommand};

const TELEMETRY_STATE_FILE: &str = "telemetry.json";
const TELEMETRY_LOG_FILE: &str = "telemetry.jsonl";
const TELEMETRY_CHANNEL_SEND_TIMEOUT: Duration = Duration::from_millis(500);
const TELEMETRY_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Telemetry {
    tx: mpsc::Sender<TelemetryCommand>,
}

impl Telemetry {
    pub fn new(data_root: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel(512);
        tokio::spawn(async move {
            telemetry_worker(data_root, rx).await;
        });
        Self { tx }
    }

    pub async fn emit(&self, event: TelemetryEvent) {
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::Event(Box::new(event))),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("telemetry channel closed; dropping event"),
            Err(_) => tracing::warn!("telemetry channel blocked; dropping event"),
        }
    }

    pub async fn emit_many(&self, events: Vec<TelemetryEvent>) {
        if events.is_empty() {
            return;
        }
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::Events(events)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("telemetry channel closed; dropping event batch"),
            Err(_) => tracing::warn!("telemetry channel blocked; dropping event batch"),
        }
    }

    pub async fn update_config(&self, cfg: TelemetryConfig) {
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::UpdateConfig(cfg)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => tracing::warn!("telemetry channel closed; dropping config update"),
            Err(_) => tracing::warn!("telemetry channel blocked; dropping config update"),
        }
    }

    pub async fn flush(&self) {
        let (tx, rx) = oneshot::channel();
        match timeout(
            TELEMETRY_CHANNEL_SEND_TIMEOUT,
            self.tx.send(TelemetryCommand::Flush(tx)),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                tracing::warn!("telemetry channel closed; flush skipped");
                return;
            }
            Err(_) => {
                tracing::warn!("telemetry channel blocked; flush skipped");
                return;
            }
        }
        let _ = timeout(TELEMETRY_REQUEST_TIMEOUT, rx).await;
    }
}

#[cfg(test)]
mod tests;
