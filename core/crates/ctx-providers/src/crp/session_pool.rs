use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};

use ctx_core::models::SessionEventType;

use crate::adapters::{ProviderProcessInfo, ProviderSessionSweepConfig, ProviderSessionSweepStats};
use crate::container_exec::translate_thread_cwd_for_container;
use crate::events::NormalizedEvent;

use super::config::{
    build_crp_session_config, build_prompt_items, flatten_prompt_items_as_text,
    model_override_disabled, provider_requires_flattened_text_prompt, split_model_id_and_effort,
};
use super::normalize::{event_matches_session, event_turn_id, map_crp_event, CachedToolInput};
use super::policy::{
    extract_runtime_fatal_error_from_stderr_line, parse_native_crp_slash_command_for_provider,
    validate_provider_slash_command_support, CrpSlashCommand,
};
use super::protocol::{CrpCommand, CrpEvent};
use super::runtime::{CrpAgentConfig, CrpProcess};
use crate::crp::CRP_CANCEL_DRAIN_TIMEOUT;

const CRP_SESSION_STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
struct CrpSessionStatusDetails {
    quiescent: bool,
}

struct SessionSnapshot {
    session_key: String,
    session: Arc<CrpSession>,
    last_used: Instant,
    draining: bool,
    shutdown_reason: Option<String>,
}

pub(super) struct CrpSessionPool {
    agent: CrpAgentConfig,
    sessions: Mutex<HashMap<String, Arc<CrpSession>>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
    busy_sessions: Arc<StdMutex<HashMap<String, usize>>>,
    default_sweep_config: ProviderSessionSweepConfig,
    supports_session_status: bool,
    reap_in_flight: AtomicBool,
}

pub(super) fn session_shutdown_reason(session: &CrpSession) -> Option<String> {
    session.process.shutdown.borrow().clone()
}

fn session_is_live(session: &CrpSession) -> bool {
    !session.draining.load(Ordering::SeqCst) && session_shutdown_reason(session).is_none()
}

impl CrpSessionPool {
    pub(super) fn new(agent: CrpAgentConfig, supports_session_status: bool) -> Self {
        Self {
            agent,
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
            busy_sessions: Arc::new(StdMutex::new(HashMap::new())),
            default_sweep_config: ProviderSessionSweepConfig::from_env(),
            supports_session_status,
            reap_in_flight: AtomicBool::new(false),
        }
    }

    pub(super) async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
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

    async fn prune_dead_sessions(&self) -> usize {
        let mut guard = self.sessions.lock().await;
        let dead_session_keys = guard
            .iter()
            .filter_map(|(session_key, session)| {
                session_shutdown_reason(session).map(|_| session_key.clone())
            })
            .collect::<Vec<_>>();
        let mut removed = 0usize;
        for session_key in dead_session_keys {
            if guard.remove(&session_key).is_some() {
                removed += 1;
            }
        }
        removed
    }

    pub(super) async fn has_session(&self, session_key: &str) -> bool {
        let sessions = self.sessions.lock().await;
        match sessions.get(session_key) {
            Some(session) => session_is_live(session),
            None => false,
        }
    }

    pub(super) async fn require_open_session(&self, session_key: &str) -> Result<Arc<CrpSession>> {
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

    pub(super) async fn restart_immediate(&self, reason: &str) {
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

    pub(super) async fn restart_drain(&self, reason: &str) {
        let active = self.busy_session_snapshot();
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

    fn busy_session_snapshot(&self) -> HashSet<String> {
        let Ok(guard) = self.busy_sessions.lock() else {
            return HashSet::new();
        };
        guard.keys().cloned().collect()
    }

    pub(super) fn session_busy_guard(&self, session_key: String) -> BusySessionGuard {
        BusySessionGuard::new(Arc::clone(&self.busy_sessions), session_key)
    }

    pub(super) fn trigger_background_reap(self: &Arc<Self>) {
        if self.reap_in_flight.swap(true, Ordering::SeqCst) {
            return;
        }

        let pool = Arc::clone(self);
        tokio::spawn(async move {
            let _ = pool
                .reap_idle_sessions(pool.default_sweep_config.clone())
                .await;
            pool.reap_in_flight.store(false, Ordering::SeqCst);
        });
    }

    pub(super) async fn drain_session_if_needed(
        &self,
        session_key: &str,
        session: &Arc<CrpSession>,
    ) {
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

    pub(super) async fn prompt(&self, req: CrpPromptRequest) -> Result<()> {
        let _guard = ActivePromptGuard::new(
            Arc::clone(&self.active_prompts),
            Arc::clone(&self.busy_sessions),
            req.session_key.clone(),
        )?;
        let session = self
            .get_or_create_session(&req.session_key, &req.workdir, &req.env)
            .await?;

        let turn_id = format!("crp-{}", uuid::Uuid::new_v4());
        let mut rx = session.process.events.subscribe();
        let mut stderr_rx = session.process.stderr_lines.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        let shutdown_reason = shutdown_rx.borrow().clone();
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
            let config = build_crp_session_config(&req.env, &req.workdir)?;
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
                let prompt_cwd = translate_thread_cwd_for_container(&req.env, &req.workdir)?;
                session
                    .process
                    .send(CrpCommand::SessionPrompt {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                        items: prompt_items,
                        prompt,
                        model,
                        reasoning_effort,
                        cwd: Some(prompt_cwd),
                    })
                    .await?;
            }
        }

        let mut last_seq = 0u64;
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut tool_input_cache: HashMap<String, CachedToolInput> = HashMap::new();
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
                        Ok(()) => shutdown_rx
                            .borrow()
                            .clone()
                            .unwrap_or_else(|| "crp_shutdown".to_string()),
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
                stderr = stderr_rx.recv() => {
                    match stderr {
                        Ok(line) => {
                            if last_seq == 0 {
                                if let Some(message) = extract_runtime_fatal_error_from_stderr_line(&line) {
                                    session.process.shutdown("crp_runtime_fatal_stderr").await;
                                    anyhow::bail!("{message}");
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {}
                    }
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
                                session.opened.store(true, Ordering::SeqCst);
                                session.opening.store(false, Ordering::SeqCst);
                                session.status_supported.store(
                                    supports_session_status
                                        .unwrap_or(self.supports_session_status),
                                    Ordering::SeqCst,
                                );
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

        session.touch();
        self.drain_session_if_needed(&req.session_key, &session)
            .await;
        Ok(())
    }

    pub(super) async fn get_or_create_session(
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
                    existing.touch();
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

        if self.sessions.lock().await.len() >= self.default_sweep_config.max_idle_sessions.max(1) {
            let _ = self.prune_dead_sessions().await;
        }

        let process = CrpProcess::spawn(&self.agent, workdir, env)
            .await
            .with_context(|| format!("spawning CRP runtime {}", self.agent.command))?;
        let session = Arc::new(CrpSession {
            process,
            opened: AtomicBool::new(false),
            opening: AtomicBool::new(false),
            status_supported: AtomicBool::new(self.supports_session_status),
            draining: AtomicBool::new(false),
            last_used: StdMutex::new(Instant::now()),
        });
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_key.to_string(), Arc::clone(&session));
        Ok(session)
    }

    pub(super) async fn reap_idle_sessions(
        &self,
        config: ProviderSessionSweepConfig,
    ) -> ProviderSessionSweepStats {
        let active = self.busy_session_snapshot();
        let now = Instant::now();
        let sessions = {
            let guard = self.sessions.lock().await;
            guard
                .iter()
                .map(|(session_key, session)| SessionSnapshot {
                    session_key: session_key.clone(),
                    session: Arc::clone(session),
                    last_used: session.last_used(),
                    draining: session.draining.load(Ordering::SeqCst),
                    shutdown_reason: session_shutdown_reason(session),
                })
                .collect::<Vec<_>>()
        };

        let mut stats = ProviderSessionSweepStats::default();
        let mut dead_candidates = Vec::new();
        let mut idle_candidates = Vec::new();
        for snapshot in sessions {
            if snapshot.shutdown_reason.is_some() {
                dead_candidates.push(snapshot);
                continue;
            }
            if snapshot.draining {
                continue;
            }
            if active.contains(&snapshot.session_key) {
                continue;
            }
            idle_candidates.push(snapshot);
        }

        if !dead_candidates.is_empty() {
            let mut guard = self.sessions.lock().await;
            for candidate in dead_candidates {
                let should_remove = matches!(
                    guard.get(&candidate.session_key),
                    Some(current)
                        if Arc::ptr_eq(current, &candidate.session)
                            && session_shutdown_reason(current).is_some()
                );
                if should_remove && guard.remove(&candidate.session_key).is_some() {
                    stats.dead_removed += 1;
                }
            }
        }

        idle_candidates.sort_by_key(|candidate| candidate.last_used);
        let mut to_reap = Vec::new();
        let mut remaining_idle = idle_candidates.len();
        for candidate in idle_candidates {
            let ttl_expired = now.duration_since(candidate.last_used) >= config.idle_ttl;
            let cap_expired = remaining_idle > config.max_idle_sessions;
            if !ttl_expired && !cap_expired {
                break;
            }
            if !candidate.session.opened.load(Ordering::SeqCst)
                && !candidate.session.opening.load(Ordering::SeqCst)
            {
                to_reap.push(candidate);
                remaining_idle = remaining_idle.saturating_sub(1);
                continue;
            }
            if !candidate.session.status_supported.load(Ordering::SeqCst) {
                continue;
            }

            match self
                .query_session_status(&candidate.session_key, &candidate.session)
                .await
            {
                Ok(status) if status.quiescent => {
                    to_reap.push(candidate);
                    remaining_idle = remaining_idle.saturating_sub(1);
                }
                Ok(_) => stats.skipped_busy += 1,
                Err(_) => stats.status_errors += 1,
            }
        }

        for candidate in to_reap {
            if self
                .busy_session_snapshot()
                .contains(&candidate.session_key)
            {
                continue;
            }
            let removed = {
                let mut guard = self.sessions.lock().await;
                match guard.get(&candidate.session_key) {
                    Some(current)
                        if Arc::ptr_eq(current, &candidate.session)
                            && !current.draining.load(Ordering::SeqCst)
                            && current.last_used() == candidate.last_used =>
                    {
                        guard.remove(&candidate.session_key);
                        true
                    }
                    _ => false,
                }
            };
            if removed {
                candidate
                    .session
                    .process
                    .shutdown(&format!("idle session reap ({})", candidate.session_key))
                    .await;
                stats.reaped += 1;
            }
        }

        stats
    }

    async fn query_session_status(
        &self,
        session_key: &str,
        session: &Arc<CrpSession>,
    ) -> Result<CrpSessionStatusDetails> {
        let mut rx = session.process.events.subscribe();
        let mut shutdown_rx = session.process.shutdown.subscribe();
        session
            .process
            .send(CrpCommand::SessionStatus {
                session_id: Some(session_key.to_string()),
            })
            .await?;

        tokio::time::timeout(CRP_SESSION_STATUS_TIMEOUT, async {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        let reason = shutdown_rx.borrow().clone().unwrap_or_else(|| "crp_shutdown".to_string());
                        anyhow::bail!("CRP runtime shut down while querying session status: {reason}");
                    }
                    recv = rx.recv() => {
                        match recv {
                            Ok(env) => {
                                if !event_matches_session(&env.event, session_key) {
                                    continue;
                                }
                                match env.event {
                                    CrpEvent::SessionNotice { code, details, .. } if code == "session_status" => {
                                        let details = details.ok_or_else(|| anyhow::anyhow!("session_status notice missing details"))?;
                                        return serde_json::from_value::<CrpSessionStatusDetails>(details)
                                            .context("parsing session status details");
                                    }
                                    CrpEvent::SessionNotice { code, message, .. } if code == "session_status_failed" => {
                                        anyhow::bail!(message.unwrap_or_else(|| "session status query failed".to_string()));
                                    }
                                    _ => {}
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => {
                                anyhow::bail!("CRP runtime closed while waiting for session status");
                            }
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for session status"))?
    }
}

struct ActivePromptGuard {
    _busy_guard: BusySessionGuard,
    session_key: String,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl ActivePromptGuard {
    fn new(
        active_prompts: Arc<StdMutex<HashSet<String>>>,
        busy_sessions: Arc<StdMutex<HashMap<String, usize>>>,
        session_key: String,
    ) -> Result<Self> {
        if let Ok(mut active) = active_prompts.lock() {
            if active.contains(&session_key) {
                anyhow::bail!("session {session_key} already has an active prompt");
            }
            active.insert(session_key.clone());
        }
        Ok(Self {
            _busy_guard: BusySessionGuard::new(busy_sessions, session_key.clone()),
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

pub(super) struct BusySessionGuard {
    session_key: String,
    busy_sessions: Arc<StdMutex<HashMap<String, usize>>>,
}

impl BusySessionGuard {
    fn new(busy_sessions: Arc<StdMutex<HashMap<String, usize>>>, session_key: String) -> Self {
        if let Ok(mut guard) = busy_sessions.lock() {
            *guard.entry(session_key.clone()).or_default() += 1;
        }
        Self {
            session_key,
            busy_sessions,
        }
    }
}

impl Drop for BusySessionGuard {
    fn drop(&mut self) {
        let Ok(mut guard) = self.busy_sessions.lock() else {
            return;
        };
        let Some(count) = guard.get_mut(&self.session_key) else {
            return;
        };
        if *count > 1 {
            *count -= 1;
            return;
        }
        guard.remove(&self.session_key);
    }
}

pub(super) struct CrpSession {
    pub(super) process: Arc<CrpProcess>,
    pub(super) opened: AtomicBool,
    pub(super) opening: AtomicBool,
    pub(super) status_supported: AtomicBool,
    pub(super) draining: AtomicBool,
    last_used: StdMutex<Instant>,
}

impl CrpSession {
    pub(super) fn touch(&self) {
        if let Ok(mut last_used) = self.last_used.lock() {
            *last_used = Instant::now();
        }
    }

    pub(super) fn last_used(&self) -> Instant {
        self.last_used
            .lock()
            .map(|instant| *instant)
            .unwrap_or_else(|_| Instant::now())
    }
}

pub(super) struct CrpPromptRequest {
    pub(super) session_key: String,
    pub(super) input: crate::adapters::TurnInput,
    pub(super) workdir: PathBuf,
    pub(super) env: HashMap<String, String>,
    pub(super) event_sink: mpsc::Sender<NormalizedEvent>,
    pub(super) cancel_rx: oneshot::Receiver<()>,
}
