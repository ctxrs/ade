use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Duration;
use uuid::Uuid;

use ctx_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo,
    ProviderRestartMode, ProviderSessionSweepConfig, ProviderSessionSweepStats, ProviderStatus,
    ProviderTurnOutcome, RunHandle, TurnInput,
};
use crate::events::NormalizedEvent;

mod config;
mod normalize;
mod normalize_tool_payload;
mod policy;
mod probe;
mod protocol;
mod runtime;
mod session_pool;
#[cfg(test)]
mod tests;
mod unknown_event;

use self::runtime::{resolve_explicit_command_path, CrpAgentConfig};
#[cfg(test)]
use self::session_pool::session_shutdown_reason;
use self::session_pool::{CrpPromptRequest, CrpSessionPool};

pub use self::protocol::{CrpModelInfo, CrpModelsProbe};
pub(crate) use self::runtime::rewrite_bundled_path_for_linux;

const CRP_VERSION: u32 = 1;
const CRP_MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CRP_MODEL_PROBE_TIMEOUT_CONTAINER: Duration = Duration::from_secs(45);
const CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT_CONTAINER: Duration = Duration::from_secs(5);
pub(super) const CRP_CANCEL_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const CODEX_CRP_DUMP_CODEX_EVENTS_ENV: &str = "CODEX_CRP_DUMP_CODEX_EVENTS_PATH";
const CODEX_CRP_DUMP_CRP_EVENTS_ENV: &str = "CODEX_CRP_DUMP_CRP_EVENTS_PATH";

pub(super) fn auth_required_notice_payload_from_stderr(_auth_url: &str) -> serde_json::Value {
    json!({
        "kind": "auth_required",
        "code": "auth_required",
        "message": "Authentication required.",
        "source": "crp_stderr",
    })
}

#[derive(Clone)]
pub struct Tier1CrpAdapter {
    id: String,
    command: String,
    pool: Arc<CrpSessionPool>,
}

impl Tier1CrpAdapter {
    fn new(id: &str, command: &str, args: Vec<String>, supports_session_status: bool) -> Self {
        let agent = CrpAgentConfig {
            provider_id: id.to_string(),
            command: command.to_string(),
            args: args.clone(),
        };
        Self {
            id: id.to_string(),
            command: command.to_string(),
            pool: Arc::new(CrpSessionPool::new(agent, supports_session_status)),
        }
    }

    pub fn from_raw(id: &str, command: String, args: Vec<String>) -> Self {
        Self::new(id, &command, args, true)
    }

    pub fn from_provider_runtime(id: &str, command: String, args: Vec<String>) -> Self {
        Self::new(id, &command, args, false)
    }

    pub fn from_raw_with_session_status(
        id: &str,
        command: String,
        args: Vec<String>,
        supports_session_status: bool,
    ) -> Self {
        Self::new(id, &command, args, supports_session_status)
    }

    pub fn codex() -> Self {
        Self::from_provider_runtime("codex", "codex".to_string(), vec![])
    }

    pub fn claude() -> Self {
        Self::from_provider_runtime("claude-crp", "claude-crp".to_string(), vec![])
    }
}

#[async_trait]
impl ProviderAdapter for Tier1CrpAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        let detected_path = resolve_explicit_command_path(&self.command);
        let installed = detected_path.is_some();
        let mut diagnostics = Vec::new();
        if !installed {
            diagnostics.push(format!(
                "CRP runtime executable not found: {}",
                self.command
            ));
        }

        Ok(ProviderStatus {
            provider_id: self.id.clone(),
            installed,
            detected_path: detected_path.map(|p| p.to_string_lossy().to_string()),
            version: None,
            capabilities: if installed {
                Some(default_caps(&self.id))
            } else {
                None
            },
            health: if installed {
                ProviderHealth::Ok
            } else {
                ProviderHealth::Missing
            },
            diagnostics,
            details: HashMap::new(),
            usability: crate::adapters::ProviderUsability::default(),
        })
    }

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        // Fail fast with a clear error if the runtime isn't available.
        // `inspect()` already checks this, but some call paths can attempt runs even after a stale status.
        if resolve_explicit_command_path(&self.command).is_none() {
            anyhow::bail!("CRP runtime executable not found: {}", self.command);
        }
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let (outcome_tx, outcome_rx) = oneshot::channel::<ProviderTurnOutcome>();
        let pool = Arc::clone(&self.pool);
        let error_sink = event_sink.clone();
        let join = tokio::spawn(async move {
            let session_key = env
                .get("CTX_SESSION_ID")
                .cloned()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let request = CrpPromptRequest {
                session_key,
                input,
                workdir,
                env,
                event_sink,
                cancel_rx,
            };
            let outcome = match pool.prompt(request).await {
                Ok(outcome) => outcome,
                Err(err) => {
                    let emitted = error_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Error,
                            payload_json: json!({ "message": err.to_string() }),
                        })
                        .await
                        .is_ok();
                    ProviderTurnOutcome::failed_with_context(
                        err.to_string(),
                        None,
                        None,
                        None,
                        emitted,
                    )
                }
            };
            let _ = outcome_tx.send(outcome);
            pool.trigger_background_reap();
            let _ = done_tx.send(());
        });
        let abort = join.abort_handle();

        Ok(RunHandle {
            done: done_rx,
            outcome: outcome_rx,
            cancel: Some(cancel_tx),
            abort: Some(abort),
        })
    }

    async fn cancel(&self, handle: &mut RunHandle) -> Result<()> {
        if let Some(cancel) = handle.cancel.take() {
            let _ = cancel.send(());
        }
        let done = tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle.done).await;
        if done.is_err() {
            if let Some(abort) = handle.abort.take() {
                abort.abort();
            }
        }
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        self.pool.list_processes().await
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        match mode {
            ProviderRestartMode::Immediate => self.pool.restart_immediate(reason).await,
            ProviderRestartMode::Drain => self.pool.restart_drain(reason).await,
        }
        Ok(())
    }

    async fn has_live_session(&self, session_key: &str) -> bool {
        self.pool.has_session(session_key).await
    }

    fn supports_resume(&self) -> bool {
        matches!(
            self.id.as_str(),
            "codex" | "codex-crp" | "claude" | "claude-crp"
        )
    }

    async fn set_session_pinned(&self, session_key: String, pinned: bool) -> Result<()> {
        self.pool.set_session_pinned(session_key, pinned);
        Ok(())
    }

    async fn set_session_model(&self, session_key: String, model_id: String) -> Result<()> {
        self.pool.set_session_model(session_key, model_id).await
    }

    async fn authenticate_session(
        &self,
        session_key: String,
        workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        self.pool
            .authenticate_session(session_key, workdir, env, method_id, event_sink)
            .await
    }

    async fn reap_idle_sessions(
        &self,
        config: ProviderSessionSweepConfig,
    ) -> Result<ProviderSessionSweepStats> {
        Ok(self.pool.reap_idle_sessions(config).await)
    }
}

fn default_caps(id: &str) -> ProviderCapabilities {
    ProviderCapabilities {
        stream_events: true,
        stream_format: "crp-jsonl".into(),
        has_turn_boundaries: true,
        has_tool_call_ids: true,
        has_file_change_events: false,
        has_command_events: false,
        supports_resume: matches!(id, "codex" | "codex-crp" | "claude" | "claude-crp"),
        supports_stable_session_id: true,
        supports_fork_or_rewind: false,
        supports_headless: true,
        supports_server_mode: false,
        supports_interactive_tui: false,
        supports_private_state_dir: false,
        supports_sandbox_flags: false,
        supports_approval_flags: false,
        notes: vec![],
    }
}

pub async fn probe_crp_models(
    provider_id: &str,
    command: String,
    args: Vec<String>,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<CrpModelsProbe> {
    probe::probe_crp_models(probe::CrpModelsProbeRequest {
        provider_id: provider_id.to_string(),
        command,
        args,
        workdir,
        env,
        host_timeout: CRP_MODEL_PROBE_TIMEOUT,
        container_timeout: CRP_MODEL_PROBE_TIMEOUT_CONTAINER,
        crp_version: CRP_VERSION,
    })
    .await
}

pub async fn probe_crp_runtime_launch(
    provider_id: &str,
    command: String,
    args: Vec<String>,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<()> {
    probe::probe_crp_runtime_launch(
        provider_id,
        command,
        args,
        workdir,
        env,
        CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT,
        CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT_CONTAINER,
    )
    .await
}
