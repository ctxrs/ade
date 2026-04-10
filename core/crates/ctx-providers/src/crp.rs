use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Duration;
use uuid::Uuid;

use ctx_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo,
    ProviderRestartMode, ProviderSessionSweepConfig, ProviderSessionSweepStats, ProviderStatus,
    RunHandle, TurnInput,
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

use self::config::build_crp_session_config;
use self::normalize::{event_matches_session, map_crp_event, CachedToolInput};
use self::policy::{extract_auth_error_from_stderr_line, extract_auth_url_from_stderr_line};
use self::protocol::{CrpCommand, CrpEvent};
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
const CRP_AUTH_EVENT_FORWARD_TIMEOUT: Duration = Duration::from_secs(60 * 10);
const CRP_SESSION_MODEL_UPDATE_TIMEOUT: Duration = Duration::from_secs(5);
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
            if let Err(err) = pool.prompt(request).await {
                let _ = error_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({ "message": err.to_string() }),
                    })
                    .await;
            }
            pool.trigger_background_reap();
            let _ = done_tx.send(());
        });
        let abort = join.abort_handle();

        Ok(RunHandle {
            done: done_rx,
            cancel: Some(cancel_tx),
            abort: Some(abort),
        })
    }

    async fn cancel(&self, mut handle: RunHandle) -> Result<()> {
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

    async fn set_session_model(&self, session_key: String, model_id: String) -> Result<()> {
        let busy_guard = self.pool.session_busy_guard(session_key.clone());
        let session = self.pool.require_open_session(&session_key).await?;
        session.touch();
        let mut rx = session.process.events.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        if let Err(err) = session
            .process
            .send(CrpCommand::SessionSetModel {
                session_id: Some(session_key.clone()),
                model_id: Some(model_id.clone()),
            })
            .await
        {
            drop(busy_guard);
            self.pool
                .drain_session_if_needed(&session_key, &session)
                .await;
            self.pool.trigger_background_reap();
            return Err(err);
        }

        let result = tokio::time::timeout(CRP_SESSION_MODEL_UPDATE_TIMEOUT, async {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        let reason = shutdown_rx.borrow().clone().unwrap_or_else(|| "crp_shutdown".to_string());
                        anyhow::bail!("CRP runtime shut down while setting model: {reason}");
                    }
                    recv = rx.recv() => {
                        match recv {
                            Ok(env) => {
                                if !event_matches_session(&env.event, &session_key) {
                                    continue;
                                }
                                if let CrpEvent::SessionNotice { code, message, details, .. } = env.event {
                                    if code == "session_model_updated" {
                                        let selected = details
                                            .as_ref()
                                            .and_then(|value| value.get("model_id"))
                                            .and_then(|value| value.as_str())
                                            .unwrap_or(model_id.as_str());
                                        if selected == model_id {
                                            return Ok(());
                                        }
                                    }
                                    if code == "session_model_update_failed" {
                                        let detail = message.unwrap_or_else(|| {
                                            format!("provider rejected session model '{model_id}'")
                                        });
                                        anyhow::bail!("{detail}");
                                    }
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => {
                                anyhow::bail!("CRP runtime closed while waiting for session model update");
                            }
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for session model update"));
        drop(busy_guard);
        self.pool
            .drain_session_if_needed(&session_key, &session)
            .await;
        self.pool.trigger_background_reap();
        result??;
        Ok(())
    }

    async fn authenticate_session(
        &self,
        session_key: String,
        workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let busy_guard = self.pool.session_busy_guard(session_key.clone());
        let session = self
            .pool
            .get_or_create_session(&session_key, &workdir, &env)
            .await?;
        let mut rx = session.process.events.subscribe();
        let mut stderr_rx = session.process.stderr_lines.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        let auth_session_key = session_key.clone();
        if !session.opened.load(Ordering::SeqCst) && !session.opening.load(Ordering::SeqCst) {
            let config = build_crp_session_config(&env, &workdir)?;
            let provider_session_id = env
                .get("CTX_PROVIDER_SESSION_REF")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            session.opening.store(true, Ordering::SeqCst);
            if let Err(err) = session
                .process
                .send(CrpCommand::SessionOpen {
                    session_id: Some(session_key.clone()),
                    provider_session_id,
                    config: Some(config),
                })
                .await
            {
                session.opening.store(false, Ordering::SeqCst);
                drop(busy_guard);
                self.pool
                    .drain_session_if_needed(&auth_session_key, &session)
                    .await;
                self.pool.trigger_background_reap();
                return Err(err);
            }
        }
        if let Err(err) = session
            .process
            .send(CrpCommand::SessionAuthenticate {
                session_id: Some(session_key),
                method_id,
            })
            .await
        {
            if !session.opened.load(Ordering::SeqCst) {
                session.opening.store(false, Ordering::SeqCst);
            }
            drop(busy_guard);
            self.pool
                .drain_session_if_needed(&auth_session_key, &session)
                .await;
            self.pool.trigger_background_reap();
            return Err(err);
        }
        let session_for_events = Arc::clone(&session);
        let pool_for_reap = Arc::clone(&self.pool);
        tokio::spawn(async move {
            let _busy_guard = busy_guard;
            let deadline = tokio::time::Instant::now() + CRP_AUTH_EVENT_FORWARD_TIMEOUT;
            let mut last_seq = 0u64;
            let mut tool_output_cache: HashMap<String, String> = HashMap::new();
            let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
            'auth_forward: loop {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                let timeout_remaining = deadline.saturating_duration_since(now);
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        break;
                    }
                    _ = tokio::time::sleep(timeout_remaining) => {
                        break;
                    }
                    recv = rx.recv() => {
                        match recv {
                            Ok(env) => {
                                if !event_matches_session(&env.event, &auth_session_key) {
                                    continue;
                                }
                                if env.seq <= last_seq {
                                    continue;
                                }
                                last_seq = env.seq;
                                if matches!(
                                    &env.event,
                                    CrpEvent::SessionNotice { code, .. }
                                        if code == "session_status" || code == "session_status_failed"
                                ) {
                                    continue;
                                }
                                if let CrpEvent::SessionOpened {
                                    supports_session_status,
                                    ..
                                } = &env.event
                                {
                                    session_for_events.opened.store(true, Ordering::SeqCst);
                                    session_for_events.opening.store(false, Ordering::SeqCst);
                                    let default_support = session_for_events
                                        .status_supported
                                        .load(Ordering::SeqCst);
                                    session_for_events.status_supported.store(
                                        supports_session_status.unwrap_or(default_support),
                                        Ordering::SeqCst,
                                    );
                                }
                                let auth_terminal_event = matches!(
                                    &env.event,
                                    CrpEvent::SessionNotice { code, .. }
                                    if code == "auth_complete"
                                        || code == "auth_completed"
                                        || code == "auth_success"
                                        || code == "authenticated"
                                        || code == "auth_failed"
                                        || code == "auth_error"
                                );
                                if auth_terminal_event {
                                    session_for_events.opening.store(false, Ordering::SeqCst);
                                }
                                let mapped = map_crp_event(
                                    env.event,
                                    env.channel,
                                    env.seq,
                                    &mut tool_output_cache,
                                    &mut tool_input_cache,
                                );
                                for event in mapped.events {
                                    if event_sink.send(event).await.is_err() {
                                        break 'auth_forward;
                                    }
                                }
                                if auth_terminal_event {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                let _ = event_sink
                                    .send(NormalizedEvent {
                                        event_type: SessionEventType::Notice,
                                        payload_json: json!({
                                            "kind": "session_gap",
                                            "reason": "crp_receiver_lagged",
                                        }),
                                    })
                                    .await;
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }
                    stderr = stderr_rx.recv() => {
                        match stderr {
                            Ok(line) => {
                                if let Some(auth_url) = extract_auth_url_from_stderr_line(&line) {
                                    if event_sink
                                        .send(NormalizedEvent {
                                            event_type: SessionEventType::Notice,
                                            payload_json: auth_required_notice_payload_from_stderr(
                                                &auth_url,
                                            ),
                                        })
                                        .await
                                        .is_err()
                                    {
                                        break 'auth_forward;
                                    }
                                }
                                if let Some(message) = extract_auth_error_from_stderr_line(&line) {
                                    let _ = event_sink
                                        .send(NormalizedEvent {
                                            event_type: SessionEventType::Error,
                                            payload_json: json!({
                                                "message": message,
                                                "source": "crp_stderr",
                                            }),
                                        })
                                        .await;
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => {
                                break;
                            }
                        }
                    }
                }
            }
            if !session_for_events.opened.load(Ordering::SeqCst) {
                session_for_events.opening.store(false, Ordering::SeqCst);
            }
            drop(_busy_guard);
            pool_for_reap
                .drain_session_if_needed(&auth_session_key, &session_for_events)
                .await;
            pool_for_reap.trigger_background_reap();
        });
        Ok(())
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
