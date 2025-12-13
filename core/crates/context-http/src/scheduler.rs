use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use context_core::ids::{MessageId, RunId, TurnId};
use context_core::models::{Message, MessageDelivery, MessageRole, Session, SessionEventType};
use context_providers::adapters::{ProviderAdapter, RunHandle, TurnInput};
use context_providers::events::NormalizedEvent;

use crate::daemon::AppState;

#[derive(Debug)]
pub enum SchedulerCommand {
    Enqueue(Message),
    RemoveQueued(MessageId),
    Cancel,
    Interrupt,
}

struct RunningTurn {
    adapter: Arc<dyn ProviderAdapter>,
    handle: RunHandle,
    run_id: RunId,
    turn_id: TurnId,
}

pub async fn session_worker(
    state: Arc<AppState>,
    session: Session,
    mut rx: mpsc::Receiver<SchedulerCommand>,
) {
    let mut queue: VecDeque<Message> = VecDeque::new();
    if let Ok(mut queued) = state
        .store
        .list_queued_messages_for_session(session.id)
        .await
    {
        for m in queued.drain(..) {
            queue.push_back(m);
        }
    }
    let mut running: Option<RunningTurn> = None;
    let mut suspend_queue = false;

    let worktree = match state.store.get_worktree(session.worktree_id).await {
        Ok(Some(wt)) => wt,
        _ => return,
    };
    let workdir = PathBuf::from(worktree.root_path.clone());

    loop {
        if running.is_none() && !suspend_queue {
            if let Some(msg) = queue.pop_front() {
                match start_turn(&state, &session, &workdir, msg).await {
                    Ok(turn) => {
                        state.set_running(session.id, true).await;
                        running = Some(turn);
                    }
                    Err(_) => {
                        state.set_running(session.id, false).await;
                        running = None;
                    }
                }
                continue;
            }
        }

        tokio::select! {
            cmd = rx.recv() => {
                match cmd {
                    Some(SchedulerCommand::Enqueue(msg)) => {
                        if running.is_some() {
                            queue.push_back(msg);
                        } else {
                            if matches!(msg.delivery, MessageDelivery::Immediate) {
                                suspend_queue = false;
                            }
                            queue.push_front(msg);
                        }
                    }
                    Some(SchedulerCommand::RemoveQueued(id)) => {
                        let mut next = VecDeque::new();
                        while let Some(m) = queue.pop_front() {
                            if m.id != id {
                                next.push_back(m);
                            }
                        }
                        queue = next;
                    }
                    Some(SchedulerCommand::Cancel) => {
                        if let Some(turn) = running.take() {
                            let _ = turn.adapter.cancel(turn.handle).await;
                            state.set_running(session.id, false).await;
                        }
                    }
                    Some(SchedulerCommand::Interrupt) => {
                        if let Some(turn) = running.take() {
                            let _ = emit_event(
                                &state,
                                session.id,
                                Some(turn.run_id),
                                Some(turn.turn_id),
                                SessionEventType::InterruptRequested,
                                json!({"by":"user"}),
                            ).await;
                            let _ = turn.adapter.cancel(turn.handle).await;
                            let _ = emit_event(
                                &state,
                                session.id,
                                Some(turn.run_id),
                                Some(turn.turn_id),
                                SessionEventType::TurnInterrupted,
                                json!({"reason":"user_interrupt","provider_cancelled":true}),
                            ).await;
                            state.set_running(session.id, false).await;
                            suspend_queue = true;
                        }
                    }
                    None => break,
                }
            }
            _ = async {
                if let Some(turn) = running.as_mut() {
                    let _ = (&mut turn.handle.join).await;
                }
            }, if running.is_some() => {
                running = None;
                state.set_running(session.id, false).await;
            }
        }
    }
}

async fn start_turn(
    state: &Arc<AppState>,
    session: &Session,
    workdir: &PathBuf,
    message: Message,
) -> Result<RunningTurn> {
    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id)
            .cloned()
            .or_else(|| map.get("fake").cloned())
            .expect("fake provider")
    };

    let mut message = message;
    let run_id = message.run_id.get_or_insert_with(RunId::new).to_owned();
    let turn_id = message.turn_id.get_or_insert_with(TurnId::new).to_owned();

    if message.delivered_at.is_none() {
        state.store.mark_message_delivered(message.id).await?;
        message.delivery = MessageDelivery::Immediate;
        message.delivered_at = Some(Utc::now());
    }

    let prompt = message.content.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &session.model_id, &prompt);

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CONTEXT_DAEMON_URL".to_string(), state.daemon_url.clone());
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CONTEXT_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CONTEXT_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert(
        "CONTEXT_SESSION_ID".to_string(),
        session.id.0.to_string(),
    );
    provider_env.insert(
        "CONTEXT_MCP_TOKEN".to_string(),
        uuid::Uuid::new_v4().to_string(),
    );
    if let Ok(v) = std::env::var("CONTEXT_MCP_COMMAND") {
        provider_env.insert("CONTEXT_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CONTEXT_MCP_DISABLED") {
        provider_env.insert("CONTEXT_MCP_DISABLED".to_string(), v);
    }

    let session_key = session.id.0.to_string();
    let needs_rehydrate = session.provider_session_ref.is_some()
        && !adapter.has_live_session(&session_key).await;
    let mut context_blocks: Vec<serde_json::Value> = Vec::new();
    if needs_rehydrate {
        if let Ok(block) = build_rehydrate_transcript_block(&state.store, session.id).await {
            context_blocks.push(block);
            let _ = emit_event(
                state,
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::Notice,
                json!({
                    "kind": "rehydrate",
                    "message": "Daemon restarted; providing transcript to rehydrate provider context.",
                }),
            )
            .await;
        }
    }

    let handle = adapter
        .run(
            TurnInput {
                content: prompt,
                attachments: message.attachments.clone(),
                context_blocks,
            },
            workdir.clone(),
            provider_env,
            ev_tx,
        )
        .await?;

    let state_for_events = state.clone();
    let store = state.store.clone();
    let session_id = session.id;
    let task_id = session.task_id;
    let track_id = session.track_id;

    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            let mut payload = ev.payload_json.clone();
            if matches!(ev.event_type, SessionEventType::Init) {
                if let Some(ps) = payload.get("acp_session_id").and_then(Value::as_str) {
                    let _ = store
                        .update_session_provider_session_ref(
                            session_id,
                            Some(ps.to_string()),
                        )
                        .await;
                }
            }
            if matches!(ev.event_type, SessionEventType::Done) {
                if let (Some(metrics), Some(obj)) =
                    (context_window_metrics.clone(), payload.as_object_mut())
                {
                    obj.entry("context_window")
                        .or_insert(metrics);
                    obj.entry("status").or_insert(json!("completed"));
                }
            }
            let appended = store
                .append_session_event(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    ev.event_type.clone(),
                    payload,
                )
                .await;
            if let Ok(event) = appended {
                state_for_events.publish_event(event.clone()).await;
                if matches!(event.event_type, SessionEventType::AssistantComplete) {
                    let content = event
                        .payload_json
                        .get("full_content")
                        .or_else(|| event.payload_json.get("content"))
                        .and_then(Value::as_str);
                    if let Some(content) = content
                    {
                        let msg = Message {
                            id: context_core::ids::MessageId::new(),
                            session_id,
                            task_id,
                            track_id,
                            run_id: Some(run_id),
                            turn_id: Some(turn_id),
                            role: MessageRole::Assistant,
                            content: content.to_string(),
                            attachments: vec![],
                            delivery: MessageDelivery::Immediate,
                            delivered_at: Some(event.created_at),
                            created_at: event.created_at,
                        };
                        let _ = store.insert_message(msg).await;
                    }
                }
            }
        }
    });

    Ok(RunningTurn {
        adapter,
        handle,
        run_id,
        turn_id,
    })
}

async fn build_rehydrate_transcript_block(
    store: &context_store::Store,
    session_id: context_core::ids::SessionId,
) -> Result<serde_json::Value> {
    let msgs = store.list_messages_for_session(session_id).await?;
    if msgs.is_empty() {
        anyhow::bail!("no messages to rehydrate");
    }

    const MAX_MESSAGES: usize = 24;
    const MAX_CHARS_PER_MESSAGE: usize = 4000;
    let tail: Vec<_> = msgs
        .into_iter()
        .rev()
        .take(MAX_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let mut text = String::new();
    text.push_str("Session transcript (for continuity after Context daemon restart):\n\n");
    for m in tail {
        let role = match m.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        let mut content = m.content;
        if content.chars().count() > MAX_CHARS_PER_MESSAGE {
            content = content.chars().take(MAX_CHARS_PER_MESSAGE).collect::<String>();
            content.push_str("\n…(truncated)");
        }
        text.push_str(&format!("[{}] {role}:\n{content}\n\n", m.created_at.to_rfc3339()));
    }

    Ok(json!({
        "type": "resource",
        "resource": {
            "uri": format!("context://session/{}/transcript", session_id.0),
            "mimeType": "text/plain",
            "text": text
        }
    }))
}

async fn emit_event(
    state: &Arc<AppState>,
    session_id: context_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    event_type: SessionEventType,
    payload_json: serde_json::Value,
) -> Result<()> {
    let event = state
        .store
        .append_session_event(session_id, run_id, turn_id, event_type, payload_json)
        .await?;
    state.publish_event(event).await;
    Ok(())
}

fn compute_context_window_metrics(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<serde_json::Value> {
    let context_window_tokens = model_context_window(provider_id, model_id)?;
    let context_tokens_estimate = estimate_tokens(prompt);
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(context_tokens_estimate);
    let remaining_fraction =
        remaining_tokens_estimate as f64 / context_window_tokens as f64;
    Some(json!({
        "context_tokens_estimate": context_tokens_estimate,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
    }))
}

fn model_context_window(provider_id: &str, model_id: &str) -> Option<usize> {
    match (provider_id, model_id) {
        ("fake", "fake-model") => Some(8192),
        _ => None,
    }
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    (chars + 3) / 4
}
