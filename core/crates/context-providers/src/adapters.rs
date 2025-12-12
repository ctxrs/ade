use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::events::NormalizedEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub stream_events: bool,
    pub stream_format: String,

    pub has_turn_boundaries: bool,
    pub has_tool_call_ids: bool,
    pub has_file_change_events: bool,
    pub has_command_events: bool,

    pub supports_resume: bool,
    pub supports_stable_session_id: bool,
    pub supports_fork_or_rewind: bool,

    pub supports_headless: bool,
    pub supports_server_mode: bool,
    pub supports_acp: bool,
    pub supports_interactive_tui: bool,

    pub supports_private_state_dir: bool,

    pub supports_sandbox_flags: bool,
    pub supports_approval_flags: bool,

    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealth {
    Ok,
    Missing,
    Misconfigured,
    UnsupportedVersion,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub provider_id: String,
    pub installed: bool,
    pub detected_path: Option<String>,
    pub version: Option<String>,
    pub capabilities: Option<ProviderCapabilities>,
    pub health: ProviderHealth,
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub details: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct TurnInput {
    pub content: String,
}

#[derive(Debug)]
pub struct RunHandle {
    pub join: JoinHandle<()>,
    pub cancel: Option<oneshot::Sender<()>>,
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    async fn inspect(&self) -> Result<ProviderStatus>;

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: tokio::sync::mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle>;

    async fn cancel(&self, handle: RunHandle) -> Result<()>;
}
