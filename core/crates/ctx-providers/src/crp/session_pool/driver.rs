use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::Result;
use serde_json::json;
use tokio::sync::broadcast;

use ctx_core::models::SessionEventType;

use crate::container_exec::translate_thread_cwd_for_container;
use crate::events::NormalizedEvent;

use super::super::config::{
    build_crp_session_config, build_prompt_items, flatten_prompt_items_as_text,
    model_override_disabled, provider_requires_flattened_text_prompt, split_model_id_and_effort,
};
use super::super::normalize::{
    event_matches_session, event_turn_id, map_crp_event, CachedToolInput,
};
use super::super::policy::{
    extract_auth_error_from_stderr_line, extract_auth_url_from_stderr_line,
    extract_runtime_fatal_error_from_stderr_line, parse_native_crp_slash_command_for_provider,
    validate_provider_slash_command_support, CrpSlashCommand,
};
use super::super::protocol::{CrpCommand, CrpEvent, CrpSessionConfig, KnownCrpEvent};
use super::super::{auth_required_notice_payload_from_stderr, CRP_CANCEL_DRAIN_TIMEOUT};
use super::{registry::ActivePromptGuard, CrpPromptRequest, CrpSession, CrpSessionPool};
use crate::adapters::{ProviderTurnOutcome, ProviderTurnStatus};

const CRP_AUTH_EVENT_FORWARD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60 * 10);
const CRP_SESSION_MODEL_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn is_sweep_only_status_notice(event: &CrpEvent) -> bool {
    matches!(
        event,
        CrpEvent::Known(event)
            if matches!(
                event.as_ref(),
                KnownCrpEvent::SessionNotice { code, .. }
            if code == "session_status" || code == "session_status_failed"
            )
    )
}

fn apply_session_opened_state(session: &CrpSession, event: &CrpEvent) {
    if let CrpEvent::Known(event) = event {
        let KnownCrpEvent::SessionOpened {
            supports_session_status,
            ..
        } = event.as_ref()
        else {
            return;
        };
        session.opened.store(true, Ordering::SeqCst);
        session.opening.store(false, Ordering::SeqCst);
        let default_support = session.status_supported.load(Ordering::SeqCst);
        session.status_supported.store(
            (*supports_session_status).unwrap_or(default_support),
            Ordering::SeqCst,
        );
    }
}

fn outcome_from_terminal_events(events: &[NormalizedEvent]) -> Option<ProviderTurnOutcome> {
    events.iter().find_map(|event| match event.event_type {
        SessionEventType::Done => Some(ProviderTurnOutcome::completed()),
        SessionEventType::Error => {
            let message = event
                .payload_json
                .get("message")
                .or_else(|| event.payload_json.get("error"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("crp_turn_error")
                .to_string();
            Some(ProviderTurnOutcome::failed_with_context(
                message,
                event
                    .payload_json
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                event.payload_json.get("details").cloned(),
                event.payload_json.get("kind").cloned(),
                true,
            ))
        }
        SessionEventType::TurnInterrupted => Some(ProviderTurnOutcome {
            status: ProviderTurnStatus::Interrupted,
            message: None,
            reason: event
                .payload_json
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            details: None,
            kind: None,
            provider_cancelled: event
                .payload_json
                .get("provider_cancelled")
                .and_then(serde_json::Value::as_bool),
            terminal_event_emitted: true,
        }),
        _ => None,
    })
}

fn update_terminal_outcome(
    outcome: &mut Option<ProviderTurnOutcome>,
    events: &[NormalizedEvent],
    done: bool,
) {
    if outcome.is_some() {
        return;
    }

    if let Some(terminal) = outcome_from_terminal_events(events) {
        *outcome = Some(terminal);
    } else if done {
        *outcome = Some(ProviderTurnOutcome::protocol_violation(
            "provider_protocol_violation_no_terminal_outcome",
            "CRP turn ended without a mapped terminal event",
        ));
    }
}

fn interrupted_outcome_without_event(
    reason: &str,
    provider_cancelled: bool,
) -> ProviderTurnOutcome {
    ProviderTurnOutcome {
        terminal_event_emitted: false,
        ..ProviderTurnOutcome::interrupted(reason, provider_cancelled)
    }
}

impl CrpSessionPool {
    async fn send_session_open(
        &self,
        session: &Arc<CrpSession>,
        session_key: &str,
        provider_session_id: Option<String>,
        config: CrpSessionConfig,
    ) -> Result<()> {
        session.opening.store(true, Ordering::SeqCst);
        if let Err(err) = session
            .process
            .send(CrpCommand::SessionOpen {
                session_id: Some(session_key.to_string()),
                provider_session_id,
                config: Some(config),
            })
            .await
        {
            session.opening.store(false, Ordering::SeqCst);
            return Err(err);
        }
        Ok(())
    }

    pub(in crate::crp) async fn prompt(
        self: &Arc<Self>,
        req: CrpPromptRequest,
    ) -> Result<ProviderTurnOutcome> {
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
            return Ok(ProviderTurnOutcome::interrupted(reason, true));
        }

        let result: Result<ProviderTurnOutcome> = async {
            if !session.opened.load(Ordering::SeqCst) && !session.opening.load(Ordering::SeqCst) {
                let config = build_crp_session_config(&req.env, &req.workdir)?;
                let provider_session_id = req
                    .env
                    .get("CTX_PROVIDER_SESSION_REF")
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty());
                self.send_session_open(&session, &req.session_key, provider_session_id, config)
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
            let mut outcome: Option<ProviderTurnOutcome> = None;
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
                        outcome = Some(interrupted_outcome_without_event("cancelled", true));
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
                        outcome = Some(ProviderTurnOutcome::interrupted(reason, true));
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
                                if is_sweep_only_status_notice(&env.event) {
                                    continue;
                                }
                                apply_session_opened_state(&session, &env.event);
                                let auth_required = matches!(
                                    &env.event,
                                    CrpEvent::Known(event)
                                        if matches!(
                                            event.as_ref(),
                                            KnownCrpEvent::SessionNotice { code, .. }
                                                if code == "auth_required"
                                        )
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
                                update_terminal_outcome(&mut outcome, &mapped.events, mapped.done);
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
                                    update_terminal_outcome(
                                        &mut outcome,
                                        &[NormalizedEvent {
                                            event_type: SessionEventType::TurnInterrupted,
                                            payload_json: json!({
                                                "reason": "auth_required",
                                            }),
                                        }],
                                        false,
                                    );
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
                            Err(broadcast::error::RecvError::Closed) => {
                                outcome = Some(ProviderTurnOutcome::protocol_violation(
                                    "provider_protocol_violation_event_stream_closed",
                                    "CRP event stream closed before the turn reported an outcome",
                                ));
                                break;
                            }
                        }
                    }
                }
            }
            Ok(outcome.unwrap_or_else(|| {
                ProviderTurnOutcome::protocol_violation(
                    "provider_protocol_violation_no_terminal_outcome",
                    "CRP prompt ended without a terminal outcome",
                )
            }))
        }
        .await;

        if !session.opened.load(Ordering::SeqCst) {
            session.opening.store(false, Ordering::SeqCst);
        }
        session.touch();
        self.drain_session_if_needed(&req.session_key, &session)
            .await;
        result
    }

    pub(in crate::crp) async fn set_session_model(
        self: &Arc<Self>,
        session_key: String,
        model_id: String,
    ) -> Result<()> {
        let busy_guard = self.session_busy_guard(session_key.clone());
        let session = self.require_open_session(&session_key).await?;
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
            self.drain_session_if_needed(&session_key, &session).await;
            self.trigger_background_reap();
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
                                if let CrpEvent::Known(event) = env.event {
                                    if let KnownCrpEvent::SessionNotice { code, message, details, .. } = *event {
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
        self.drain_session_if_needed(&session_key, &session).await;
        self.trigger_background_reap();
        result??;
        Ok(())
    }

    pub(in crate::crp) async fn authenticate_session(
        self: &Arc<Self>,
        session_key: String,
        workdir: std::path::PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: tokio::sync::mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let busy_guard = self.session_busy_guard(session_key.clone());
        let session = self
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
            if let Err(err) = self
                .send_session_open(&session, &session_key, provider_session_id, config)
                .await
            {
                session.opening.store(false, Ordering::SeqCst);
                drop(busy_guard);
                self.drain_session_if_needed(&auth_session_key, &session)
                    .await;
                self.trigger_background_reap();
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
            self.drain_session_if_needed(&auth_session_key, &session)
                .await;
            self.trigger_background_reap();
            return Err(err);
        }
        let session_for_events = Arc::clone(&session);
        let pool_for_reap = Arc::clone(self);
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
                                if is_sweep_only_status_notice(&env.event) {
                                    continue;
                                }
                                apply_session_opened_state(&session_for_events, &env.event);
                                let auth_terminal_event = matches!(
                                    &env.event,
                                    CrpEvent::Known(event)
                                        if matches!(
                                            event.as_ref(),
                                            KnownCrpEvent::SessionNotice { code, .. }
                                                if code == "auth_complete"
                                                    || code == "auth_completed"
                                                    || code == "auth_success"
                                                    || code == "authenticated"
                                                    || code == "auth_failed"
                                                    || code == "auth_error"
                                        )
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(event_type: SessionEventType, payload_json: serde_json::Value) -> NormalizedEvent {
        NormalizedEvent {
            event_type,
            payload_json,
        }
    }

    #[test]
    fn outcome_from_terminal_events_prefers_first_terminal_event() {
        let outcome = outcome_from_terminal_events(&[
            event(
                SessionEventType::TurnInterrupted,
                json!({"reason": "cancelled", "provider_cancelled": true}),
            ),
            event(SessionEventType::Done, json!({})),
        ])
        .expect("terminal outcome");

        assert!(matches!(outcome.status, ProviderTurnStatus::Interrupted));
        assert_eq!(outcome.reason.as_deref(), Some("cancelled"));
        assert_eq!(outcome.provider_cancelled, Some(true));
    }

    #[test]
    fn outcome_from_terminal_events_prefers_first_error_terminal_event() {
        let outcome = outcome_from_terminal_events(&[
            event(
                SessionEventType::Error,
                json!({
                    "message": "boom",
                    "reason": "crp_error",
                    "details": {"code": 42},
                    "kind": "provider_error",
                }),
            ),
            event(
                SessionEventType::TurnInterrupted,
                json!({"reason": "cancelled", "provider_cancelled": true}),
            ),
        ])
        .expect("terminal outcome");

        assert!(matches!(outcome.status, ProviderTurnStatus::Failed));
        assert_eq!(outcome.message.as_deref(), Some("boom"));
        assert_eq!(outcome.reason.as_deref(), Some("crp_error"));
        assert_eq!(outcome.details, Some(json!({"code": 42})));
        assert_eq!(outcome.kind, Some(json!("provider_error")));
    }

    #[test]
    fn update_terminal_outcome_does_not_override_existing_terminal_outcome() {
        let mut outcome = Some(ProviderTurnOutcome {
            status: ProviderTurnStatus::Interrupted,
            message: None,
            reason: Some("auth_required".to_string()),
            details: None,
            kind: None,
            provider_cancelled: None,
            terminal_event_emitted: true,
        });

        update_terminal_outcome(
            &mut outcome,
            &[event(SessionEventType::Done, json!({}))],
            true,
        );

        let outcome = outcome.expect("terminal outcome");
        assert!(matches!(outcome.status, ProviderTurnStatus::Interrupted));
        assert_eq!(outcome.reason.as_deref(), Some("auth_required"));
    }

    #[test]
    fn interrupted_outcome_without_event_requires_scheduler_terminal_event() {
        let outcome = interrupted_outcome_without_event("cancelled", true);

        assert!(matches!(outcome.status, ProviderTurnStatus::Interrupted));
        assert_eq!(outcome.reason.as_deref(), Some("cancelled"));
        assert_eq!(outcome.provider_cancelled, Some(true));
        assert!(!outcome.terminal_event_emitted);
    }
}
