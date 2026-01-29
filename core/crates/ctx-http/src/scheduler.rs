use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEvent, SessionEventType,
    SessionTurnStatus, SessionTurnTool,
};
use ctx_providers::adapters::{ProviderAdapter, RunHandle, TurnInput};
use ctx_providers::events::NormalizedEvent;
use ctx_store::store::SessionTurnToolCountDeltas;

use crate::daemon::AppState;
use crate::installer;
use crate::ops_events::OpsEvent;
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::provider_accounts;
use crate::provider_debug::apply_acp_heap_profile_env;
use crate::settings::{self, ProviderControlMode};
use crate::telemetry::TelemetryEvent;
use crate::workspace_config;
mod tools;

use tools::{
    build_tool_ops_meta, build_turn_tool_update_from_payload, cwd_outside_worktree,
    maybe_spool_tool_output, merge_tool_update, sanitize_tool_event_payload, tool_count_deltas,
};

#[derive(Debug)]
pub struct QueuedMessage {
    pub message: Message,
    pub enqueued_at: Instant,
    pub run_id: Option<String>,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum SchedulerCommand {
    Enqueue(QueuedMessage),
    RemoveQueued(MessageId),
    Cancel,
    Interrupt,
}

struct RunningTurn {
    adapter: Arc<dyn ProviderAdapter>,
    handle: RunHandle,
    run_id: RunId,
    turn_id: TurnId,
    event_tx: mpsc::Sender<NormalizedEvent>,
}

fn provider_mode_id_for(
    provider_id: &str,
    control_mode: &ProviderControlMode,
) -> Option<&'static str> {
    match control_mode {
        ProviderControlMode::Full => match provider_id {
            "codex" => Some("full-access"),
            "claude" => Some("bypassPermissions"),
            _ => None,
        },
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => None,
    }
}

pub async fn session_worker(
    state: Arc<AppState>,
    session: Session,
    mut rx: mpsc::Receiver<SchedulerCommand>,
) {
    let mut session = session;
    let mut queue: VecDeque<QueuedMessage> = VecDeque::new();
    let store = match state.store_for_session(session.id).await {
        Ok(store) => store,
        Err(_) => return,
    };
    if let Ok(mut queued) = store.list_queued_messages_for_session(session.id).await {
        for m in queued.drain(..) {
            queue.push_back(QueuedMessage {
                message: m,
                enqueued_at: Instant::now(),
                run_id: None,
            });
        }
    }
    let mut running: Option<RunningTurn> = None;
    let mut suspend_queue = false;

    let worktree = match store.get_worktree(session.worktree_id).await {
        Ok(Some(wt)) => wt,
        _ => return,
    };
    let workdir = PathBuf::from(worktree.root_path.clone());
    let is_worktree = worktree.vcs_ref.is_some() || worktree.git_branch.is_some();
    let env_target = if is_worktree {
        "worktree".to_string()
    } else {
        "local".to_string()
    };
    let mut worktree_event = OpsEvent::new("info", "worktree_resolved");
    worktree_event.session_id = Some(session.id.0.to_string());
    worktree_event.worktree_id = Some(session.worktree_id.0.to_string());
    worktree_event.worktree_root = Some(workdir.to_string_lossy().to_string());
    worktree_event.meta = Some(json!({
        "env_target": env_target.clone(),
        "vcs_kind": worktree.vcs_kind,
        "vcs_ref": worktree.vcs_ref,
        "git_branch": worktree.git_branch,
    }));
    state.ops_events.emit(worktree_event);

    loop {
        if running.is_none() && !suspend_queue {
            if let Some(msg) = queue.pop_front() {
                let session_for_turn = match store.get_session(session.id).await {
                    Ok(Some(fresh)) => {
                        session = fresh.clone();
                        fresh
                    }
                    _ => session.clone(),
                };
                if matches!(msg.message.delivery, MessageDelivery::Queued) {
                    let _ = emit_event(
                        &state,
                        session.id,
                        msg.message.run_id,
                        msg.message.turn_id,
                        SessionEventType::MessageQueuePromoted,
                        json!({
                            "message_id": msg.message.id.0,
                            "previous_position": 0,
                        }),
                    )
                    .await;
                }
                match start_turn(&state, &session_for_turn, &workdir, &env_target, msg).await {
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
                            if matches!(msg.message.delivery, MessageDelivery::Immediate) {
                                suspend_queue = false;
                            }
                            queue.push_front(msg);
                        }
                    }
                    Some(SchedulerCommand::RemoveQueued(id)) => {
                        let mut next = VecDeque::new();
                        while let Some(m) = queue.pop_front() {
                            if m.message.id != id {
                                next.push_back(m);
                            }
                        }
                        queue = next;
                    }
                    Some(SchedulerCommand::Cancel) => {
                        if let Some(turn) = running.take() {
                            let sent = send_turn_interrupted(
                                &turn.event_tx,
                                "user_cancel",
                                true,
                            )
                            .await;
                            let _ = turn.adapter.cancel(turn.handle).await;
                            if !sent {
                                let _ = reconcile_turn_terminal_state(
                                    &state,
                                    session.id,
                                    Some(turn.run_id),
                                    turn.turn_id,
                                    "user_cancel",
                                )
                                .await;
                            }
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
                            let sent = send_turn_interrupted(
                                &turn.event_tx,
                                "user_interrupt",
                                true,
                            )
                            .await;
                            let _ = turn.adapter.cancel(turn.handle).await;
                            if !sent {
                                let _ = reconcile_turn_terminal_state(
                                    &state,
                                    session.id,
                                    Some(turn.run_id),
                                    turn.turn_id,
                                    "user_interrupt",
                                )
                                .await;
                            }
                            state.set_running(session.id, false).await;
                            suspend_queue = true;
                        }
                    }
                    None => break,
                }
            }
            _ = async {
                if let Some(turn) = running.as_mut() {
                    let _ = (&mut turn.handle.done).await;
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
    workdir: &Path,
    env_target: &str,
    queued: QueuedMessage,
) -> Result<RunningTurn> {
    state.wait_for_worktree_bootstrap(session.worktree_id).await;

    let store = state.store_for_session(session.id).await?;

    let workdir_root = workdir.to_path_buf();
    let workdir_canonical = tokio::fs::canonicalize(&workdir_root).await.ok();
    let workdir_str = workdir_root.to_string_lossy().to_string();

    let adapter = {
        let map = state.providers.lock().await;
        map.get(&session.provider_id)
            .cloned()
            .ok_or_else(|| anyhow!("provider not available: {}", session.provider_id))?
    };

    let mut message = queued.message;
    let message_id = message.id;
    let perf_run_id = queued.run_id.clone();
    let queue_wait_ms = queued.enqueued_at.elapsed().as_millis() as u64;
    let mut queue_labels = HashMap::new();
    queue_labels.insert("provider_id".to_string(), session.provider_id.clone());
    queue_labels.insert("model_id".to_string(), session.model_id.clone());
    queue_labels.insert("env_target".to_string(), env_target.to_string());
    queue_labels.insert("event".to_string(), "queue_wait".to_string());
    let queue_metric = PerfMetric {
        name: "scheduler.queue_wait_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: queue_wait_ms as f64,
        labels: queue_labels,
    };
    state
        .perf_telemetry
        .record_metric(queue_metric, perf_run_id.clone(), None, None)
        .await;
    let run_id = message.run_id.get_or_insert_with(RunId::new).to_owned();
    let turn_id = message.turn_id.get_or_insert_with(TurnId::new).to_owned();

    let mut run_event = OpsEvent::new("info", "provider_run_started");
    run_event.session_id = Some(session.id.0.to_string());
    run_event.worktree_id = Some(session.worktree_id.0.to_string());
    run_event.run_id = Some(run_id.0.to_string());
    run_event.turn_id = Some(turn_id.0.to_string());
    run_event.provider_id = Some(session.provider_id.clone());
    run_event.cwd = Some(workdir_str.clone());
    run_event.worktree_root = Some(workdir_str.clone());
    run_event.meta = Some(json!({
        "model_id": session.model_id.clone(),
        "env_target": env_target,
    }));
    state.ops_events.emit(run_event);

    if message.delivered_at.is_none() {
        store.mark_message_delivered(message.id).await?;
        message.delivery = MessageDelivery::Immediate;
        message.delivered_at = Some(Utc::now());
    }
    let _ = store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Running,
            None,
            None,
            Utc::now(),
        )
        .await;
    let _ = emit_event(
        state,
        session.id,
        Some(run_id),
        Some(turn_id),
        SessionEventType::TurnStarted,
        json!({
            "message_id": message.id.0,
        }),
    )
    .await;

    let prompt = message.content.clone();
    let mut provider_session_ref = session.provider_session_ref.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &session.model_id, &prompt);

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let event_tx = ev_tx.clone();

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.daemon_url.clone());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert(
        "CLAUDE_CODE_ENABLE_ASK_USER_QUESTION_TOOL".to_string(),
        "1".to_string(),
    );
    if let Some(token) = state.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    provider_env.insert("CTX_MODEL_ID".to_string(), session.model_id.clone());
    let mcp_token = uuid::Uuid::new_v4().to_string();
    provider_env.insert("CTX_MCP_TOKEN".to_string(), mcp_token);
    let provider_control_mode = settings::load_settings(&state.data_root)
        .await
        .sandboxing
        .as_ref()
        .map(|s| s.provider_control_mode.clone())
        .unwrap_or_default();
    if let Some(mode_id) = provider_mode_id_for(&session.provider_id, &provider_control_mode) {
        provider_env.insert("CTX_PROVIDER_MODE".to_string(), mode_id.to_string());
    }
    if let Ok(v) = std::env::var("CTX_MCP_COMMAND") {
        provider_env.insert("CTX_MCP_COMMAND".to_string(), v);
    }
    if let Ok(v) = std::env::var("CTX_MCP_DISABLED") {
        provider_env.insert("CTX_MCP_DISABLED".to_string(), v);
    }
    if session.provider_id == "codex" || session.provider_id == "codex-crp" {
        if let Ok(env) = provider_accounts::codex_env_for_active_account(&state.data_root).await {
            for (key, value) in env {
                provider_env.insert(key, value);
            }
        }
    }

    if let Ok(cfg) = installer::load_agent_server_config(&state.data_root).await {
        if let Some(cmd) = cfg.providers.get(&session.provider_id) {
            let mut bin_dirs: Vec<std::path::PathBuf> = Vec::new();
            for dep in &cmd.dependencies {
                if let Some(meta) = cfg.managed_installs.get(dep) {
                    if let Some(rel) = meta.bin_dir_rel.as_ref() {
                        bin_dirs.push(state.data_root.join(rel));
                    }
                }
            }
            if !bin_dirs.is_empty() {
                let mut path_parts: Vec<std::path::PathBuf> = bin_dirs;
                if let Some(current) = std::env::var_os("PATH") {
                    path_parts.extend(std::env::split_paths(&current));
                }
                if let Ok(joined) = std::env::join_paths(path_parts) {
                    provider_env.insert("PATH".to_string(), joined.to_string_lossy().to_string());
                }
            }
        }
    }
    if !provider_env.contains_key("CTX_WORKER_GATEWAY_URL") {
        apply_acp_heap_profile_env(&session.provider_id, &mut provider_env, &state.data_root);
    }

    let prompt_config = workspace_config::load_agent_system_prompt_append(workdir)
        .await
        .unwrap_or_else(|_| workspace_config::AgentSystemPromptAppendConfig::new_default(workdir));
    let mut system_prompt_append = prompt_config.effective_append();
    if session.relationship.as_deref() == Some("sub_agent") {
        let subagent_config = workspace_config::load_subagent_system_prompt_append(workdir)
            .await
            .unwrap_or_else(|_| {
                workspace_config::SubagentSystemPromptAppendConfig::new_default(workdir)
            });
        if let Some(subagent_append) = subagent_config.effective_append() {
            system_prompt_append = Some(match system_prompt_append {
                Some(mut append) => {
                    append.push_str("\n\n");
                    append.push_str(&subagent_append);
                    append
                }
                None => subagent_append,
            });
        }
    }
    let mut context_blocks = Vec::new();
    if let Some(append) = system_prompt_append.as_deref() {
        if !provider_supports_system_prompt_append(&session.provider_id) {
            context_blocks.push(json!({"type":"text","text": append}));
        }
        provider_env.insert("CTX_SYSTEM_PROMPT_APPEND".to_string(), append.to_string());
    }

    let run_started_at = Instant::now();
    let spawn_started_at = Instant::now();
    let handle = match adapter
        .run(
            TurnInput {
                content: prompt,
                attachments: message.attachments.clone(),
                context_blocks,
                model_id: normalize_session_model_id(&session.model_id),
            },
            workdir.to_path_buf(),
            provider_env,
            ev_tx,
        )
        .await
    {
        Ok(handle) => {
            let spawn_ms = spawn_started_at.elapsed().as_millis() as u64;
            let mut spawn_labels = HashMap::new();
            spawn_labels.insert("provider_id".to_string(), session.provider_id.clone());
            spawn_labels.insert("model_id".to_string(), session.model_id.clone());
            spawn_labels.insert("env_target".to_string(), env_target.to_string());
            spawn_labels.insert("event".to_string(), "spawn".to_string());
            let spawn_metric = PerfMetric {
                name: "provider.spawn_ms".to_string(),
                kind: PerfMetricKind::Histogram,
                unit: "ms".to_string(),
                value: spawn_ms as f64,
                labels: spawn_labels,
            };
            state
                .perf_telemetry
                .record_metric(spawn_metric, perf_run_id.clone(), None, None)
                .await;
            handle
        }
        Err(err) => {
            let duration_ms = run_started_at.elapsed().as_millis() as u64;
            state
                .telemetry
                .emit(TelemetryEvent::provider_call(
                    session.provider_id.clone(),
                    session.model_id.clone(),
                    Some(env_target.to_string()),
                    false,
                    duration_ms,
                ))
                .await;
            let mut fail_event = OpsEvent::new("error", "provider_run_failed");
            fail_event.session_id = Some(session.id.0.to_string());
            fail_event.worktree_id = Some(session.worktree_id.0.to_string());
            fail_event.run_id = Some(run_id.0.to_string());
            fail_event.turn_id = Some(turn_id.0.to_string());
            fail_event.provider_id = Some(session.provider_id.clone());
            fail_event.cwd = Some(workdir_str.clone());
            fail_event.worktree_root = Some(workdir_str.clone());
            fail_event.meta = Some(json!({
                "model_id": session.model_id.clone(),
                "env_target": env_target,
                "error": err.to_string(),
            }));
            state.ops_events.emit(fail_event);
            return Err(err);
        }
    };

    let state_for_events = Arc::clone(state);
    let store = store.clone();
    let session_id = session.id;
    let task_id = session.task_id;
    let workspace_id = session.workspace_id;
    let worktree_id = session.worktree_id;
    let provider_id = session.provider_id.clone();
    let model_id = session.model_id.clone();
    let env_target = env_target.to_string();
    let perf_run_id = perf_run_id.clone();
    let workdir_root = workdir_root.clone();
    let workdir_canonical = workdir_canonical.clone();
    let workdir_str = workdir_str.clone();
    let mut telemetry_emitted = false;

    tokio::spawn(async move {
        let mut assistant_partial = String::new();
        let mut assistant_sequence: i64 = 0;
        let mut assistant_emitted = String::new();
        let mut thought_partial = String::new();
        let mut tool_cache: HashMap<String, SessionTurnTool> = HashMap::new();
        let mut terminal_status: Option<SessionTurnStatus> = None;
        let mut first_event_at: Option<Instant> = None;

        while let Some(ev) = ev_rx.recv().await {
            let event_type = ev.event_type.clone();
            let raw_payload = ev.payload_json.clone();
            let mut payload = raw_payload.clone();
            if first_event_at.is_none() {
                first_event_at = Some(Instant::now());
                let first_ms = run_started_at.elapsed().as_millis() as u64;
                let mut first_labels = HashMap::new();
                first_labels.insert("provider_id".to_string(), provider_id.clone());
                first_labels.insert("model_id".to_string(), model_id.clone());
                first_labels.insert("env_target".to_string(), env_target.clone());
                first_labels.insert("event".to_string(), "first_event".to_string());
                let first_metric = PerfMetric {
                    name: "provider.first_event_ms".to_string(),
                    kind: PerfMetricKind::Histogram,
                    unit: "ms".to_string(),
                    value: first_ms as f64,
                    labels: first_labels,
                };
                state_for_events
                    .perf_telemetry
                    .record_metric(first_metric, perf_run_id.clone(), None, None)
                    .await;
            }
            if matches!(ev.event_type, SessionEventType::Init) {
                let provider_session_id = payload
                    .get("provider_session_id")
                    .or_else(|| payload.get("crp_session_id"))
                    .or_else(|| payload.get("acp_session_id"))
                    .and_then(Value::as_str);
                if let Some(ps) = provider_session_id {
                    provider_session_ref = Some(ps.to_string());
                    let _ = store
                        .update_session_provider_session_ref(session_id, Some(ps.to_string()))
                        .await;
                }
                if model_id.trim().is_empty() || model_id.eq_ignore_ascii_case("default") {
                    if let Some(current) = payload
                        .get("models")
                        .and_then(|m| {
                            m.get("currentModelId")
                                .or_else(|| m.get("current_model_id"))
                        })
                        .and_then(Value::as_str)
                    {
                        let _ = store
                            .update_session_model(session_id, current.to_string())
                            .await;
                    }
                }
            }
            if matches!(ev.event_type, SessionEventType::Done) {
                if let Some(obj) = payload.as_object_mut() {
                    if obj.get("context_window").is_none() {
                        let metrics = if provider_id == "codex" || provider_id == "codex-crp" {
                            provider_session_ref
                                .as_deref()
                                .and_then(read_codex_context_window_metrics)
                                .or_else(|| context_window_metrics.clone())
                        } else {
                            context_window_metrics.clone()
                        };
                        if let Some(metrics) = metrics {
                            obj.entry("context_window").or_insert(metrics);
                        }
                    }
                    obj.entry("status").or_insert(json!("completed"));
                }
            }
            if matches!(event_type, SessionEventType::ToolCall) {
                let tool_meta = build_tool_ops_meta(&event_type, &raw_payload);
                let mut meta = serde_json::Map::new();
                if let Some(tool_call_id) = tool_meta.tool_call_id.clone() {
                    meta.insert("tool_call_id".to_string(), json!(tool_call_id));
                }
                if let Some(title) = tool_meta.title.clone() {
                    meta.insert("title".to_string(), json!(title));
                }
                if let Some(status) = tool_meta.status.clone() {
                    meta.insert("status".to_string(), json!(status));
                }
                if let Some(input_preview) = tool_meta.input_preview.clone() {
                    meta.insert("input".to_string(), input_preview);
                }
                let mut event = OpsEvent::new("info", "tool_exec");
                event.session_id = Some(session_id.0.to_string());
                event.worktree_id = Some(worktree_id.0.to_string());
                event.run_id = Some(run_id.0.to_string());
                event.turn_id = Some(turn_id.0.to_string());
                event.provider_id = Some(provider_id.clone());
                event.tool_kind = tool_meta.tool_kind.clone();
                event.cwd = tool_meta.cwd.clone();
                event.worktree_root = Some(workdir_str.clone());
                event.meta = if meta.is_empty() {
                    None
                } else {
                    Some(Value::Object(meta))
                };
                state_for_events.ops_events.emit(event);

                if let Some(cwd) = tool_meta.cwd.as_deref() {
                    if cwd_outside_worktree(cwd, &workdir_root, workdir_canonical.as_ref()) {
                        let mut warn_event = OpsEvent::new("warn", "tool_exec_anomaly");
                        warn_event.session_id = Some(session_id.0.to_string());
                        warn_event.worktree_id = Some(worktree_id.0.to_string());
                        warn_event.run_id = Some(run_id.0.to_string());
                        warn_event.turn_id = Some(turn_id.0.to_string());
                        warn_event.provider_id = Some(provider_id.clone());
                        warn_event.tool_kind = tool_meta.tool_kind.clone();
                        warn_event.cwd = Some(cwd.to_string());
                        warn_event.worktree_root = Some(workdir_str.clone());
                        warn_event.meta = Some(json!({
                            "reason": "cwd_outside_worktree",
                            "tool_call_id": tool_meta.tool_call_id,
                        }));
                        state_for_events.ops_events.emit(warn_event);
                    }
                }
            }
            if matches!(
                event_type,
                SessionEventType::ToolCall
                    | SessionEventType::ToolCallUpdate
                    | SessionEventType::ToolResult
            ) {
                let output_spool_path = if matches!(event_type, SessionEventType::ToolResult) {
                    maybe_spool_tool_output(
                        state_for_events.as_ref(),
                        &raw_payload,
                        session_id,
                        turn_id,
                    )
                    .await
                } else {
                    None
                };
                payload = sanitize_tool_event_payload(
                    &event_type,
                    &raw_payload,
                    output_spool_path.as_deref(),
                );
            }
            let appended = store
                .append_session_event(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    event_type.clone(),
                    payload,
                )
                .await;
            if let Ok(event) = appended {
                state_for_events.publish_event(event.clone()).await;

                match event.event_type {
                    SessionEventType::AssistantChunk => {
                        if let Some(fragment) =
                            raw_payload.get("content_fragment").and_then(Value::as_str)
                        {
                            assistant_partial.push_str(fragment);
                        }
                    }
                    SessionEventType::ThoughtChunk => {
                        if should_track_thought_chunk(&raw_payload) {
                            if let Some(fragment) =
                                raw_payload.get("content_fragment").and_then(Value::as_str)
                            {
                                thought_partial.push_str(fragment);
                                let _ = store
                                    .update_session_turn_partial(
                                        session_id,
                                        turn_id,
                                        None,
                                        Some(&thought_partial),
                                        event.created_at,
                                    )
                                    .await;
                            }
                        }
                    }
                    SessionEventType::Notice => {
                        if event
                            .payload_json
                            .get("kind")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind == "session_gap")
                        {
                            let reason = event
                                .payload_json
                                .get("reason")
                                .and_then(Value::as_str)
                                .map(|value| value.to_string());
                            state_for_events
                                .workspace_active_snapshot
                                .publish_session_gap(workspace_id, session_id, event.seq, reason)
                                .await;
                        }
                    }
                    SessionEventType::ToolCall
                    | SessionEventType::ToolCallUpdate
                    | SessionEventType::ToolResult => {
                        if !assistant_partial.is_empty() {
                            if let Ok(saved) = persist_assistant_message(
                                &store,
                                session_id,
                                task_id,
                                run_id,
                                turn_id,
                                assistant_partial.clone(),
                                assistant_sequence + 1,
                                event.created_at,
                            )
                            .await
                            {
                                assistant_sequence += 1;
                                assistant_emitted.push_str(&saved.content);
                                assistant_partial.clear();
                                let _ = emit_event(
                                    &state_for_events,
                                    session_id,
                                    Some(run_id),
                                    Some(turn_id),
                                    SessionEventType::AssistantMessageInserted,
                                    json!({
                                        "message_id": saved.id.0,
                                        "content": saved.content,
                                        "delivery": saved.delivery,
                                        "attachments": saved.attachments,
                                        "turn_sequence": saved.turn_sequence,
                                    }),
                                )
                                .await;
                            }
                        }
                        if let Some(update) =
                            build_turn_tool_update_from_payload(&event_type, &raw_payload)
                        {
                            let prev =
                                if matches!(event.event_type, SessionEventType::ToolCallUpdate) {
                                    tool_cache.get(&update.tool_call_id).cloned()
                                } else if let Some(cached) =
                                    tool_cache.get(&update.tool_call_id).cloned()
                                {
                                    Some(cached)
                                } else {
                                    store
                                        .get_session_turn_tool(session_id, &update.tool_call_id)
                                        .await
                                        .ok()
                                        .flatten()
                                };
                            let merged = merge_tool_update(
                                prev.as_ref(),
                                update,
                                session_id,
                                turn_id,
                                event.seq,
                                event.created_at,
                            );
                            if matches!(event.event_type, SessionEventType::ToolCallUpdate) {
                                tool_cache.insert(merged.tool_call_id.clone(), merged);
                            } else {
                                let (
                                    delta_total,
                                    delta_pending,
                                    delta_running,
                                    delta_completed,
                                    delta_failed,
                                ) = tool_count_deltas(prev.as_ref(), &merged);
                                let _ = store.upsert_session_turn_tool(merged.clone()).await;
                                if delta_total != 0
                                    || delta_pending != 0
                                    || delta_running != 0
                                    || delta_completed != 0
                                    || delta_failed != 0
                                {
                                    let _ = store
                                        .update_session_turn_tool_counts(
                                            session_id,
                                            turn_id,
                                            SessionTurnToolCountDeltas {
                                                total: delta_total,
                                                pending: delta_pending,
                                                running: delta_running,
                                                completed: delta_completed,
                                                failed: delta_failed,
                                            },
                                            event.created_at,
                                        )
                                        .await;
                                }
                                tool_cache.insert(merged.tool_call_id.clone(), merged);
                            }
                        }
                    }
                    SessionEventType::AssistantComplete => {
                        let content: Option<String> = event
                            .payload_json
                            .get("full_content")
                            .or_else(|| event.payload_json.get("content"))
                            .and_then(Value::as_str)
                            .map(|s: &str| s.to_string())
                            .or_else(|| {
                                if assistant_partial.is_empty() {
                                    None
                                } else {
                                    Some(assistant_partial.clone())
                                }
                            });
                        if let Some(content) = content {
                            if let Some(content) =
                                strip_emitted_prefix(&content, &assistant_emitted)
                            {
                                if let Ok(saved) = persist_assistant_message(
                                    &store,
                                    session_id,
                                    task_id,
                                    run_id,
                                    turn_id,
                                    content,
                                    assistant_sequence + 1,
                                    event.created_at,
                                )
                                .await
                                {
                                    assistant_sequence += 1;
                                    assistant_emitted.push_str(&saved.content);
                                    assistant_partial.clear();
                                    let _ = emit_event(
                                        &state_for_events,
                                        session_id,
                                        Some(run_id),
                                        Some(turn_id),
                                        SessionEventType::AssistantMessageInserted,
                                        json!({
                                            "message_id": saved.id.0,
                                            "content": saved.content,
                                            "delivery": saved.delivery,
                                            "attachments": saved.attachments,
                                            "turn_sequence": saved.turn_sequence,
                                        }),
                                    )
                                    .await;
                                }
                            } else {
                                assistant_partial.clear();
                            }
                        }
                        if terminal_status.is_none() {
                            let _ = store
                                .update_session_turn_status(
                                    session_id,
                                    turn_id,
                                    SessionTurnStatus::Completed,
                                    None,
                                    None,
                                    event.created_at,
                                )
                                .await;
                        }
                        let _ = store
                            .delete_session_events_for_turn_types(
                                session_id,
                                turn_id,
                                &[
                                    SessionEventType::AssistantChunk,
                                    SessionEventType::ThoughtChunk,
                                ],
                            )
                            .await;
                    }
                    SessionEventType::Done => {
                        if !telemetry_emitted {
                            telemetry_emitted = true;
                            let duration_ms = run_started_at.elapsed().as_millis() as u64;
                            let mut run_labels = HashMap::new();
                            run_labels.insert("provider_id".to_string(), provider_id.clone());
                            run_labels.insert("model_id".to_string(), model_id.clone());
                            run_labels.insert("env_target".to_string(), env_target.clone());
                            run_labels.insert("event".to_string(), "run_complete".to_string());
                            let run_metric = PerfMetric {
                                name: "scheduler.run_total_ms".to_string(),
                                kind: PerfMetricKind::Histogram,
                                unit: "ms".to_string(),
                                value: duration_ms as f64,
                                labels: run_labels,
                            };
                            state_for_events
                                .perf_telemetry
                                .record_metric(run_metric, perf_run_id.clone(), None, None)
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::provider_call(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    true,
                                    duration_ms,
                                ))
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::session_completed(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    "completed".to_string(),
                                    duration_ms,
                                ))
                                .await;
                        }
                        let metrics = event.payload_json.get("context_window");
                        if terminal_status.is_none() {
                            let _ = store
                                .update_session_turn_status(
                                    session_id,
                                    turn_id,
                                    SessionTurnStatus::Completed,
                                    Some(event.seq),
                                    metrics,
                                    event.created_at,
                                )
                                .await;
                        }
                        let _ = store
                            .delete_session_events_for_turn_types(
                                session_id,
                                turn_id,
                                &[SessionEventType::ThoughtChunk],
                            )
                            .await;
                        let _ = emit_event(
                            &state_for_events,
                            session_id,
                            Some(run_id),
                            Some(turn_id),
                            SessionEventType::TurnFinished,
                            json!({
                                "message_id": message_id.0,
                                "status": "completed",
                            }),
                        )
                        .await;
                    }
                    SessionEventType::TurnInterrupted => {
                        if !telemetry_emitted {
                            telemetry_emitted = true;
                            let duration_ms = run_started_at.elapsed().as_millis() as u64;
                            let mut run_labels = HashMap::new();
                            run_labels.insert("provider_id".to_string(), provider_id.clone());
                            run_labels.insert("model_id".to_string(), model_id.clone());
                            run_labels.insert("env_target".to_string(), env_target.clone());
                            run_labels.insert("event".to_string(), "run_interrupt".to_string());
                            let run_metric = PerfMetric {
                                name: "scheduler.run_total_ms".to_string(),
                                kind: PerfMetricKind::Histogram,
                                unit: "ms".to_string(),
                                value: duration_ms as f64,
                                labels: run_labels,
                            };
                            state_for_events
                                .perf_telemetry
                                .record_metric(run_metric, perf_run_id.clone(), None, None)
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::provider_call(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    false,
                                    duration_ms,
                                ))
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::session_completed(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    "interrupted".to_string(),
                                    duration_ms,
                                ))
                                .await;
                        }
                        terminal_status = Some(SessionTurnStatus::Interrupted);
                        let _ = store
                            .update_session_turn_status(
                                session_id,
                                turn_id,
                                SessionTurnStatus::Interrupted,
                                Some(event.seq),
                                None,
                                event.created_at,
                            )
                            .await;
                        let _ = store
                            .delete_session_events_for_turn_types(
                                session_id,
                                turn_id,
                                &[
                                    SessionEventType::AssistantChunk,
                                    SessionEventType::ThoughtChunk,
                                ],
                            )
                            .await;
                        let _ = emit_event(
                            &state_for_events,
                            session_id,
                            Some(run_id),
                            Some(turn_id),
                            SessionEventType::TurnFinished,
                            json!({
                                "message_id": message_id.0,
                                "status": "interrupted",
                                "reason": event.payload_json.get("reason").cloned(),
                                "provider_cancelled": event.payload_json.get("provider_cancelled").cloned(),
                            }),
                        )
                        .await;
                    }
                    SessionEventType::Error => {
                        if matches!(terminal_status, Some(SessionTurnStatus::Interrupted)) {
                            continue;
                        }
                        if !telemetry_emitted {
                            telemetry_emitted = true;
                            let duration_ms = run_started_at.elapsed().as_millis() as u64;
                            let mut run_labels = HashMap::new();
                            run_labels.insert("provider_id".to_string(), provider_id.clone());
                            run_labels.insert("model_id".to_string(), model_id.clone());
                            run_labels.insert("env_target".to_string(), env_target.clone());
                            run_labels.insert("event".to_string(), "run_failed".to_string());
                            let run_metric = PerfMetric {
                                name: "scheduler.run_total_ms".to_string(),
                                kind: PerfMetricKind::Histogram,
                                unit: "ms".to_string(),
                                value: duration_ms as f64,
                                labels: run_labels,
                            };
                            state_for_events
                                .perf_telemetry
                                .record_metric(run_metric, perf_run_id.clone(), None, None)
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::provider_call(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    false,
                                    duration_ms,
                                ))
                                .await;
                            state_for_events
                                .telemetry
                                .emit(TelemetryEvent::session_completed(
                                    provider_id.clone(),
                                    model_id.clone(),
                                    Some(env_target.clone()),
                                    "failed".to_string(),
                                    duration_ms,
                                ))
                                .await;
                        }
                        terminal_status = Some(SessionTurnStatus::Failed);
                        let _ = store
                            .update_session_turn_status(
                                session_id,
                                turn_id,
                                SessionTurnStatus::Failed,
                                None,
                                None,
                                event.created_at,
                            )
                            .await;
                        let _ = store
                            .delete_session_events_for_turn_types(
                                session_id,
                                turn_id,
                                &[
                                    SessionEventType::AssistantChunk,
                                    SessionEventType::ThoughtChunk,
                                ],
                            )
                            .await;
                        let _ = emit_event(
                            &state_for_events,
                            session_id,
                            Some(run_id),
                            Some(turn_id),
                            SessionEventType::TurnFinished,
                            json!({
                                "message_id": message_id.0,
                                "status": "failed",
                            }),
                        )
                        .await;
                    }
                    _ => {}
                }
            }
        }
    });

    Ok(RunningTurn {
        adapter,
        handle,
        run_id,
        turn_id,
        event_tx,
    })
}

async fn send_turn_interrupted(
    event_tx: &mpsc::Sender<NormalizedEvent>,
    reason: &str,
    provider_cancelled: bool,
) -> bool {
    event_tx
        .send(NormalizedEvent {
            event_type: SessionEventType::TurnInterrupted,
            payload_json: json!({
                "reason": reason,
                "provider_cancelled": provider_cancelled,
                "status": "interrupted",
            }),
        })
        .await
        .is_ok()
}

pub async fn reconcile_turn_terminal_state(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: TurnId,
    fallback_reason: &str,
) -> Result<()> {
    let store = state.store_for_session(session_id).await?;
    let turn = store.get_session_turn(session_id, turn_id).await?;
    let Some(turn) = turn else {
        return Ok(());
    };
    if matches!(
        turn.status,
        SessionTurnStatus::Completed | SessionTurnStatus::Failed | SessionTurnStatus::Interrupted
    ) {
        return Ok(());
    }

    let events = store
        .list_session_events_for_turn(session_id, turn_id, false)
        .await?;
    if let Some(event) = events.iter().rev().find(|ev| {
        matches!(
            ev.event_type,
            SessionEventType::Done
                | SessionEventType::Error
                | SessionEventType::TurnInterrupted
                | SessionEventType::TurnFinished
        )
    }) {
        match event.event_type {
            SessionEventType::Done => {
                let metrics = event.payload_json.get("context_window");
                let _ = store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Completed,
                        Some(event.seq),
                        metrics,
                        event.created_at,
                    )
                    .await;
            }
            SessionEventType::TurnFinished => {
                let _ = store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Completed,
                        Some(event.seq),
                        None,
                        event.created_at,
                    )
                    .await;
            }
            SessionEventType::TurnInterrupted => {
                let _ = store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Interrupted,
                        Some(event.seq),
                        None,
                        event.created_at,
                    )
                    .await;
            }
            SessionEventType::Error => {
                let _ = store
                    .update_session_turn_status(
                        session_id,
                        turn_id,
                        SessionTurnStatus::Failed,
                        None,
                        None,
                        event.created_at,
                    )
                    .await;
            }
            _ => {}
        }
        return Ok(());
    }

    let event = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnInterrupted,
        json!({"reason": fallback_reason, "provider_cancelled": false}),
    )
    .await?;
    let _ = store
        .update_session_turn_status(
            session_id,
            turn_id,
            SessionTurnStatus::Interrupted,
            Some(event.seq),
            None,
            event.created_at,
        )
        .await;
    let _ = emit_event(
        state,
        session_id,
        run_id,
        Some(turn_id),
        SessionEventType::TurnFinished,
        json!({
            "message_id": turn.user_message_id.map(|id| id.0),
            "status": "interrupted",
            "reason": fallback_reason,
        }),
    )
    .await;
    Ok(())
}

#[allow(dead_code)]
async fn build_rehydrate_transcript_block(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
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
    text.push_str("Session transcript (for continuity after ctx daemon restart):\n\n");
    for m in tail {
        let role = match m.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        let mut content = m.content;
        if content.chars().count() > MAX_CHARS_PER_MESSAGE {
            content = content
                .chars()
                .take(MAX_CHARS_PER_MESSAGE)
                .collect::<String>();
            content.push_str("\n…(truncated)");
        }
        text.push_str(&format!(
            "[{}] {role}:\n{content}\n\n",
            m.created_at.to_rfc3339()
        ));
    }

    Ok(json!({
        "type": "resource",
        "resource": {
            "uri": format!("ctx://session/{}/transcript", session_id.0),
            "mimeType": "text/plain",
            "text": text
        }
    }))
}
async fn emit_event(
    state: &Arc<AppState>,
    session_id: ctx_core::ids::SessionId,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    event_type: SessionEventType,
    payload_json: serde_json::Value,
) -> Result<SessionEvent> {
    let store = state.store_for_session(session_id).await?;
    let event = store
        .append_session_event(session_id, run_id, turn_id, event_type, payload_json)
        .await?;
    state.publish_event(event.clone()).await;
    Ok(event)
}

fn should_track_thought_chunk(payload: &serde_json::Value) -> bool {
    let meta = payload
        .get("acp_update")
        .and_then(|v| v.get("_meta"))
        .or_else(|| payload.get("acp_update").and_then(|v| v.get("meta")))
        .or_else(|| payload.get("_meta"))
        .or_else(|| payload.get("meta"));
    if meta
        .and_then(|v| v.get("heartbeat"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return false;
    }
    let has_status_text = |meta: &Value| {
        for key in ["status_text", "statusText", "status_string", "statusString"] {
            if meta.get(key).and_then(Value::as_str).is_some() {
                return true;
            }
        }
        if let Some(status) = meta.get("status").and_then(Value::as_str) {
            let s = status.trim().to_lowercase();
            let is_tool_status = matches!(
                s.as_str(),
                "pending"
                    | "queued"
                    | "running"
                    | "in_progress"
                    | "completed"
                    | "failed"
                    | "error"
                    | "ok"
                    | "success"
                    | "succeeded"
            );
            if !is_tool_status {
                return true;
            }
        }
        false
    };

    let reasoning_kind = meta
        .and_then(|v| v.get("codex"))
        .and_then(|v| v.get("reasoning_kind").or_else(|| v.get("reasoningKind")))
        .and_then(Value::as_str);
    if matches!(reasoning_kind, Some("summary" | "status")) {
        return false;
    }
    if let Some(meta) = meta {
        if has_status_text(meta) {
            return false;
        }
        if let Some(codex_meta) = meta.get("codex") {
            if has_status_text(codex_meta) {
                return false;
            }
        }
    }
    true
}

fn strip_emitted_prefix(full_content: &str, emitted: &str) -> Option<String> {
    let full = full_content.trim_end_matches(['\r', '\n']);
    if full.is_empty() {
        return None;
    }
    let emitted_trimmed = emitted.trim_end_matches(|c: char| c.is_whitespace());
    if emitted_trimmed.is_empty() {
        return Some(full.to_string());
    }
    if full == emitted_trimmed {
        return None;
    }
    if full.starts_with(emitted_trimmed) {
        let suffix = full.get(emitted_trimmed.len()..).unwrap_or("").to_string();
        if suffix.trim().is_empty() {
            None
        } else {
            Some(suffix)
        }
    } else {
        Some(full.to_string())
    }
}

#[allow(clippy::too_many_arguments)]
async fn persist_assistant_message(
    store: &ctx_store::Store,
    session_id: ctx_core::ids::SessionId,
    task_id: ctx_core::ids::TaskId,
    run_id: RunId,
    turn_id: TurnId,
    content: String,
    turn_sequence: i64,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Result<Message> {
    if content.is_empty() {
        return Err(anyhow!("assistant message content empty"));
    }
    let msg = Message {
        id: ctx_core::ids::MessageId::new(),
        session_id,
        task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(turn_sequence),
        role: MessageRole::Assistant,
        content,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: Some(created_at),
        created_at,
    };
    store.insert_message(msg).await
}

fn compute_context_window_metrics(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<serde_json::Value> {
    let context_window_tokens = model_context_window(provider_id, model_id)?;
    let context_tokens_estimate = estimate_tokens(prompt);
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(context_tokens_estimate);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;
    Some(json!({
        "context_tokens_estimate": context_tokens_estimate,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
    }))
}

fn read_codex_context_window_metrics(session_ref: &str) -> Option<serde_json::Value> {
    let path = find_codex_session_log(session_ref)?;
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    let mut latest_info: Option<Value> = None;

    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let payload: Value = serde_json::from_str(trimmed).ok()?;
        if payload.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let event_payload = payload.get("payload")?;
        if event_payload.get("type").and_then(Value::as_str) != Some("token_count") {
            continue;
        }
        if let Some(info) = event_payload.get("info") {
            latest_info = Some(info.clone());
        }
    }

    let info = latest_info?;
    let context_window_tokens = info.get("model_context_window").and_then(Value::as_u64)?;
    let last_usage = info.get("last_token_usage").and_then(Value::as_object);
    let input_tokens = last_usage
        .and_then(|m| m.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let output_tokens = last_usage
        .and_then(|m| m.get("output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let reasoning_tokens = last_usage
        .and_then(|m| m.get("reasoning_output_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total_tokens = last_usage
        .and_then(|m| m.get("total_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(
            input_tokens
                .saturating_add(output_tokens)
                .saturating_add(reasoning_tokens),
        );

    if context_window_tokens == 0 {
        return None;
    }
    let remaining_tokens_estimate = context_window_tokens.saturating_sub(total_tokens);
    let remaining_fraction = remaining_tokens_estimate as f64 / context_window_tokens as f64;

    Some(json!({
        "context_tokens_estimate": total_tokens,
        "context_window_tokens": context_window_tokens,
        "remaining_tokens_estimate": remaining_tokens_estimate,
        "remaining_fraction": remaining_fraction,
        "total_input_tokens": input_tokens,
        "total_output_tokens": output_tokens.saturating_add(reasoning_tokens),
    }))
}

fn find_codex_session_log(session_ref: &str) -> Option<PathBuf> {
    let base = directories::BaseDirs::new()?;
    let root = base.home_dir().join(".codex").join("sessions");
    if !root.exists() {
        return None;
    }
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".jsonl") && name.contains(session_ref) {
                return Some(path);
            }
        }
    }
    None
}

pub(crate) fn model_context_window(provider_id: &str, model_id: &str) -> Option<usize> {
    match (provider_id, model_id) {
        ("fake", "fake-model") => Some(8192),
        _ => None,
    }
}

fn estimate_tokens(text: &str) -> usize {
    let chars = text.chars().count();
    chars.div_ceil(4)
}

fn normalize_session_model_id(model_id: &str) -> Option<String> {
    let trimmed = model_id.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn provider_supports_system_prompt_append(provider_id: &str) -> bool {
    matches!(provider_id, "claude" | "codex")
}

#[cfg(test)]
mod strip_emitted_prefix_tests {
    use super::strip_emitted_prefix;

    #[test]
    fn returns_full_when_no_emitted() {
        assert_eq!(strip_emitted_prefix("Hello", ""), Some("Hello".to_string()));
    }

    #[test]
    fn returns_suffix_when_full_contains_emitted_prefix() {
        let full = "Planning:Done.";
        let emitted = "Planning:";
        assert_eq!(
            strip_emitted_prefix(full, emitted),
            Some("Done.".to_string())
        );
    }

    #[test]
    fn returns_none_when_full_equals_emitted() {
        assert_eq!(strip_emitted_prefix("Same", "Same"), None);
    }

    #[test]
    fn returns_full_when_prefix_does_not_match() {
        assert_eq!(
            strip_emitted_prefix("Hello", "Nope"),
            Some("Hello".to_string())
        );
    }
}
