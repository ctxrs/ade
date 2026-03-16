use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::time::Duration;
use uuid::Uuid;

use ctx_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo,
    ProviderRestartMode, ProviderStatus, RunHandle, TurnInput,
};
use crate::events::NormalizedEvent;

mod config;
mod normalize;
mod policy;
mod probe;
mod protocol;
mod runtime;
#[cfg(test)]
mod tests;

use self::config::{
    build_crp_session_config, build_prompt_items, flatten_prompt_items_as_text,
    model_override_disabled, provider_requires_flattened_text_prompt, split_model_id_and_effort,
};
use self::normalize::{event_matches_session, event_turn_id, map_crp_event, CachedToolInput};
use self::policy::{
    extract_auth_error_from_stderr_line, extract_auth_url_from_stderr_line,
    parse_native_crp_slash_command_for_provider, validate_provider_slash_command_support,
    CrpSlashCommand,
};
use self::protocol::{CrpCommand, CrpEvent};
use self::runtime::{resolve_explicit_command_path, CrpAgentConfig, CrpProcess};

pub use self::protocol::{CrpModelInfo, CrpModelsProbe};
pub(crate) use self::runtime::rewrite_bundled_path_for_linux;

const CRP_VERSION: u32 = 1;
const CRP_MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const CRP_MODEL_PROBE_TIMEOUT_CONTAINER: Duration = Duration::from_secs(45);
const CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const CRP_RUNTIME_LAUNCH_PROBE_TIMEOUT_CONTAINER: Duration = Duration::from_secs(5);
const CRP_AUTH_EVENT_FORWARD_TIMEOUT: Duration = Duration::from_secs(60 * 10);
const CRP_SESSION_MODEL_UPDATE_TIMEOUT: Duration = Duration::from_secs(5);
const CRP_CANCEL_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const CODEX_CRP_DUMP_CODEX_EVENTS_ENV: &str = "CODEX_CRP_DUMP_CODEX_EVENTS_PATH";
const CODEX_CRP_DUMP_CRP_EVENTS_ENV: &str = "CODEX_CRP_DUMP_CRP_EVENTS_PATH";

#[derive(Clone)]
pub struct Tier1CrpAdapter {
    id: String,
    command: String,
    pool: Arc<CrpSessionPool>,
}

impl Tier1CrpAdapter {
    fn new(id: &str, command: &str, args: Vec<String>) -> Self {
        let agent = CrpAgentConfig {
            provider_id: id.to_string(),
            command: command.to_string(),
            args: args.clone(),
        };
        Self {
            id: id.to_string(),
            command: command.to_string(),
            pool: Arc::new(CrpSessionPool::new(agent)),
        }
    }

    pub fn from_raw(id: &str, command: String, args: Vec<String>) -> Self {
        Self::new(id, &command, args)
    }

    pub fn codex() -> Self {
        Self::new("codex", "codex", vec![])
    }

    pub fn claude() -> Self {
        Self::new("claude-crp", "claude-crp", vec![])
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
        self.pool.restart(reason, mode).await
    }

    async fn has_live_session(&self, session_key: &str) -> bool {
        self.pool.has_session(session_key).await
    }

    async fn set_session_model(&self, session_key: String, model_id: String) -> Result<()> {
        let session = self.pool.require_open_session(&session_key).await?;
        let mut rx = session.process.events.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        session
            .process
            .send(CrpCommand::SessionSetModel {
                session_id: Some(session_key.clone()),
                model_id: Some(model_id.clone()),
            })
            .await?;

        tokio::time::timeout(CRP_SESSION_MODEL_UPDATE_TIMEOUT, async {
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
        .map_err(|_| anyhow::anyhow!("timed out waiting for session model update"))??;
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
        let session = self
            .pool
            .get_or_create_session(&session_key, &workdir, &env)
            .await?;
        let mut rx = session.process.events.subscribe();
        let mut stderr_rx = session.process.stderr_lines.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        let auth_session_key = session_key.clone();
        if !session.opened.load(Ordering::SeqCst) && !session.opening.load(Ordering::SeqCst) {
            let config = build_crp_session_config(&env, &workdir);
            let provider_session_id = env
                .get("CTX_PROVIDER_SESSION_REF")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            session.opening.store(true, Ordering::SeqCst);
            session
                .process
                .send(CrpCommand::SessionOpen {
                    session_id: Some(session_key.clone()),
                    provider_session_id,
                    config: Some(config),
                })
                .await?;
        }
        session
            .process
            .send(CrpCommand::SessionAuthenticate {
                session_id: Some(session_key),
                method_id,
            })
            .await?;
        let session_for_events = Arc::clone(&session);
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + CRP_AUTH_EVENT_FORWARD_TIMEOUT;
            let mut last_seq = 0u64;
            let mut tool_output_cache: HashMap<String, String> = HashMap::new();
            let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
            loop {
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
                                if matches!(&env.event, CrpEvent::SessionOpened { .. }) {
                                    session_for_events.opened.store(true, Ordering::SeqCst);
                                    session_for_events.opening.store(false, Ordering::SeqCst);
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
                                        return;
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
                                            payload_json: json!({
                                                "kind": "auth_url_stderr",
                                                "auth_url": auth_url,
                                                "source": "crp_stderr",
                                            }),
                                        })
                                        .await
                                        .is_err()
                                    {
                                        return;
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
        });
        Ok(())
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
        supports_resume: matches!(id, "codex"),
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

struct CrpSessionPool {
    agent: CrpAgentConfig,
    sessions: Mutex<HashMap<String, Arc<CrpSession>>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

fn session_shutdown_reason(session: &CrpSession) -> Option<String> {
    session.process.shutdown.borrow().clone()
}

fn session_is_live(session: &CrpSession) -> bool {
    !session.draining.load(Ordering::SeqCst) && session_shutdown_reason(session).is_none()
}

impl CrpSessionPool {
    fn new(agent: CrpAgentConfig) -> Self {
        Self {
            agent,
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        let sessions = self.sessions.lock().await;
        let mut out = Vec::new();
        for (session_id, session) in sessions.iter() {
            if let Some(pid) = session.process.pid().await {
                out.push(ProviderProcessInfo {
                    provider_id: self.agent.provider_id.clone(),
                    pid,
                    label: Some(session_id.clone()),
                });
            }
        }
        out
    }

    async fn has_session(&self, session_key: &str) -> bool {
        let sessions = self.sessions.lock().await;
        match sessions.get(session_key) {
            Some(session) => session_is_live(session),
            None => false,
        }
    }

    async fn require_open_session(&self, session_key: &str) -> Result<Arc<CrpSession>> {
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(session_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("provider session {session_key} is not live"))?;
        drop(sessions);
        if !session_is_live(&session) {
            anyhow::bail!("provider session {session_key} is not live");
        }
        if !session.opened.load(Ordering::SeqCst) {
            anyhow::bail!("provider session {session_key} is not open");
        }
        Ok(session)
    }

    async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
        match mode {
            ProviderRestartMode::Immediate => {
                self.restart_immediate(reason).await;
                Ok(())
            }
            ProviderRestartMode::Drain => {
                self.restart_drain(reason).await;
                Ok(())
            }
        }
    }

    async fn restart_immediate(&self, reason: &str) {
        let sessions = {
            let mut guard = self.sessions.lock().await;
            guard.drain().collect::<Vec<_>>()
        };
        for (session_key, session) in sessions {
            session
                .process
                .shutdown(&format!("{reason}: immediate restart ({session_key})"))
                .await;
        }
    }

    async fn restart_drain(&self, reason: &str) {
        let active = self.active_prompt_snapshot();
        let sessions_to_kill = {
            let mut guard = self.sessions.lock().await;
            let mut to_kill = Vec::new();
            for (session_key, session) in guard.iter() {
                session.draining.store(true, Ordering::SeqCst);
                if !active.contains(session_key) {
                    to_kill.push((session_key.clone(), Arc::clone(session)));
                }
            }
            for (session_key, _) in &to_kill {
                guard.remove(session_key);
            }
            to_kill
        };

        for (session_key, session) in sessions_to_kill {
            session
                .process
                .shutdown(&format!("{reason}: drain idle ({session_key})"))
                .await;
        }
    }

    fn active_prompt_snapshot(&self) -> HashSet<String> {
        let Ok(guard) = self.active_prompts.lock() else {
            return HashSet::new();
        };
        guard.iter().cloned().collect()
    }

    async fn drain_session_if_needed(&self, session_key: &str, session: &Arc<CrpSession>) {
        if !session.draining.load(Ordering::SeqCst) {
            return;
        }
        let should_remove = {
            let mut guard = self.sessions.lock().await;
            let Some(current) = guard.get(session_key) else {
                return;
            };
            if !Arc::ptr_eq(current, session) {
                return;
            }
            guard.remove(session_key);
            true
        };
        if should_remove {
            session
                .process
                .shutdown(&format!("drain completed ({session_key})"))
                .await;
        }
    }

    async fn prompt(&self, req: CrpPromptRequest) -> Result<()> {
        let _guard =
            ActivePromptGuard::new(Arc::clone(&self.active_prompts), req.session_key.clone())?;
        let session = self
            .get_or_create_session(&req.session_key, &req.workdir, &req.env)
            .await?;

        let turn_id = format!("crp-{}", Uuid::new_v4());
        let mut rx = session.process.events.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        let shutdown_reason = {
            let reason = shutdown_rx.borrow().clone();
            reason
        };
        if let Some(reason) = shutdown_reason {
            let _ = req
                .event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::TurnInterrupted,
                    payload_json: json!({
                        "reason": reason,
                        "provider_cancelled": true,
                    }),
                })
                .await;
            self.drain_session_if_needed(&req.session_key, &session)
                .await;
            return Ok(());
        }

        if !session.opened.load(Ordering::SeqCst) && !session.opening.load(Ordering::SeqCst) {
            let config = build_crp_session_config(&req.env, &req.workdir);
            let provider_session_id = req
                .env
                .get("CTX_PROVIDER_SESSION_REF")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            session.opening.store(true, Ordering::SeqCst);
            session
                .process
                .send(CrpCommand::SessionOpen {
                    session_id: Some(req.session_key.clone()),
                    provider_session_id,
                    config: Some(config),
                })
                .await?;
        }
        validate_provider_slash_command_support(&self.agent.provider_id, &req.input.content)?;
        match parse_native_crp_slash_command_for_provider(
            &self.agent.provider_id,
            &req.input.content,
        ) {
            Some(CrpSlashCommand::Compact) => {
                session
                    .process
                    .send(CrpCommand::SessionCompact {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    })
                    .await?;
            }
            Some(CrpSlashCommand::Undo) => {
                session
                    .process
                    .send(CrpCommand::SessionUndo {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    })
                    .await?;
            }
            Some(CrpSlashCommand::Review { instructions }) => {
                session
                    .process
                    .send(CrpCommand::SessionReview {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                        instructions,
                    })
                    .await?;
            }
            None => {
                let items = build_prompt_items(&req.input, &req.workdir, &req.env).await?;
                let (prompt_items, prompt) =
                    if provider_requires_flattened_text_prompt(&self.agent.provider_id) {
                        (None, Some(flatten_prompt_items_as_text(&items)?))
                    } else {
                        (Some(items), Some(req.input.content.clone()))
                    };
                let (model, reasoning_effort) = if model_override_disabled(&req.env) {
                    (None, None)
                } else {
                    req.input
                        .model_id
                        .as_deref()
                        .map(split_model_id_and_effort)
                        .unwrap_or((None, None))
                };
                session
                    .process
                    .send(CrpCommand::SessionPrompt {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                        items: prompt_items,
                        prompt,
                        model,
                        reasoning_effort,
                        cwd: Some(req.workdir.clone()),
                    })
                    .await?;
            }
        }
        let mut last_seq = 0u64;
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
        // Debugging aid: when set, dump normalized session events (post-CRP mapping) to this file.
        // This lets us diff: raw Codex dump -> CRP stdout -> ctx-normalized events -> web UI.
        let dump_norm_path = std::env::var("CTX_CRP_DUMP_NORMALIZED_EVENTS_PATH")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut dump_norm_file = dump_norm_path.as_deref().and_then(|path| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()
        });
        let mut cancel_rx = req.cancel_rx;
        let mut cancel_requested = false;
        let mut cancel_deadline: Option<tokio::time::Instant> = None;
        loop {
            tokio::select! {
                _ = &mut cancel_rx, if !cancel_requested => {
                    let _ = session.process.send(CrpCommand::SessionCancel {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    }).await;
                    cancel_requested = true;
                    cancel_deadline = Some(tokio::time::Instant::now() + CRP_CANCEL_DRAIN_TIMEOUT);
                }
                _ = async {
                    if let Some(deadline) = cancel_deadline {
                        tokio::time::sleep_until(deadline).await;
                    }
                }, if cancel_requested && cancel_deadline.is_some() => {
                    break;
                }
                shutdown = shutdown_rx.changed() => {
                    let reason = match shutdown {
                        Ok(()) => {
                            let current = shutdown_rx.borrow().clone();
                            current.unwrap_or_else(|| "crp_shutdown".to_string())
                        }
                        Err(_) => "crp_shutdown".to_string(),
                    };
                    let _ = req
                        .event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::TurnInterrupted,
                            payload_json: json!({
                                "reason": reason,
                                "provider_cancelled": true,
                            }),
                        })
                        .await;
                    break;
                }
                recv = rx.recv() => {
                    match recv {
                        Ok(env) => {
                            if !event_matches_session(&env.event, &req.session_key) {
                                continue;
                            }
                            if let Some(event_turn_id) = event_turn_id(&env.event) {
                                if event_turn_id != turn_id {
                                    continue;
                                }
                            }
                            if env.seq <= last_seq {
                                continue;
                            }
                            last_seq = env.seq;
                            if matches!(&env.event, CrpEvent::SessionOpened { .. }) {
                                session.opened.store(true, Ordering::SeqCst);
                                session.opening.store(false, Ordering::SeqCst);
                            }
                            let auth_required = matches!(
                                &env.event,
                                CrpEvent::SessionNotice { code, .. } if code == "auth_required"
                            );
                            if auth_required {
                                session.opening.store(false, Ordering::SeqCst);
                            }
                            let mapped = map_crp_event(
                                env.event,
                                env.channel,
                                env.seq,
                                &mut tool_output_cache,
                                &mut tool_input_cache,
                            );
                            for event in mapped.events {
                                if let Some(f) = dump_norm_file.as_mut() {
                                    // Best-effort only; never fail the turn because debug dumping failed.
                                    let _ = writeln!(
                                        f,
                                        "{}",
                                        json!({
                                            "session_key": req.session_key,
                                            "turn_id": turn_id,
                                            "crp_seq": env.seq,
                                            "event_type": format!("{:?}", event.event_type),
                                            "payload_json": event.payload_json,
                                        })
                                    );
                                }
                                let _ = req.event_sink.send(event).await;
                            }
                            if auth_required {
                                let _ = req
                                    .event_sink
                                    .send(NormalizedEvent {
                                        event_type: SessionEventType::TurnInterrupted,
                                        payload_json: json!({
                                            "reason": "auth_required",
                                        }),
                                    })
                                    .await;
                                break;
                            }
                            if mapped.done {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let payload = json!({
                                "kind": "session_gap",
                                "reason": "crp_receiver_lagged",
                            });
                            let _ = req
                                .event_sink
                                .send(NormalizedEvent {
                                    event_type: SessionEventType::Notice,
                                    payload_json: payload,
                                })
                                .await;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        self.drain_session_if_needed(&req.session_key, &session)
            .await;
        Ok(())
    }

    async fn get_or_create_session(
        &self,
        session_key: &str,
        workdir: &PathBuf,
        env: &HashMap<String, String>,
    ) -> Result<Arc<CrpSession>> {
        let replaced = {
            let mut sessions = self.sessions.lock().await;
            if let Some(existing) = sessions.get(session_key) {
                let shutdown_reason = session_shutdown_reason(existing);
                if !existing.draining.load(Ordering::SeqCst) && shutdown_reason.is_none() {
                    return Ok(Arc::clone(existing));
                }
                sessions
                    .remove(session_key)
                    .map(|session| (session, shutdown_reason))
            } else {
                None
            }
        };
        if let Some((existing, shutdown_reason)) = replaced {
            if shutdown_reason.is_none() {
                existing
                    .process
                    .shutdown(&format!("drain replace ({session_key})"))
                    .await;
            }
        }

        let process = CrpProcess::spawn(&self.agent, workdir, env)
            .await
            .with_context(|| format!("spawning CRP runtime {}", self.agent.command))?;
        let session = Arc::new(CrpSession {
            process,
            opened: AtomicBool::new(false),
            opening: AtomicBool::new(false),
            draining: AtomicBool::new(false),
        });
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_key.to_string(), Arc::clone(&session));
        Ok(session)
    }
}

struct ActivePromptGuard {
    session_key: String,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl ActivePromptGuard {
    fn new(active_prompts: Arc<StdMutex<HashSet<String>>>, session_key: String) -> Result<Self> {
        if let Ok(mut active) = active_prompts.lock() {
            if active.contains(&session_key) {
                anyhow::bail!("session {session_key} already has an active prompt");
            }
            active.insert(session_key.clone());
        }
        Ok(Self {
            session_key,
            active_prompts,
        })
    }
}

impl Drop for ActivePromptGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_prompts.lock() {
            active.remove(&self.session_key);
        }
    }
}

struct CrpSession {
    process: Arc<CrpProcess>,
    opened: AtomicBool,
    opening: AtomicBool,
    draining: AtomicBool,
}

struct CrpPromptRequest {
    session_key: String,
    input: TurnInput,
    workdir: PathBuf,
    env: HashMap<String, String>,
    event_sink: mpsc::Sender<NormalizedEvent>,
    cancel_rx: oneshot::Receiver<()>,
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
