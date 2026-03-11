use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    MessageDelivery, MessageRole, Session, SessionEventType, SessionTurnStatus, SessionTurnTool,
};
use ctx_providers::adapters::TurnInput;
use ctx_providers::events::NormalizedEvent;
use ctx_store::store::SessionTurnToolCountDeltas;

use crate::daemon::{ensure_provider_adapter_for_target_with_cfg, AppState};
use crate::execution_effective;
use crate::harness_runtime::HarnessRuntimeKind;
use crate::harness_sources::{self, HarnessSourceKind};
use crate::installer;
use crate::installs::InstallTarget;
use crate::ops_events::OpsEvent;
use crate::order_seq::{attach_order_seq, OrderSeqState};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::provider_accounts;
use crate::settings::{self, ProviderControlMode};
use crate::telemetry::TelemetryEvent;
use crate::workspace_config;

use super::lifecycle::RunningTurn;
use super::persistence::{append_session_event_with_retry, emit_event, persist_assistant_message};
use super::tools::{
    build_tool_ops_meta, build_turn_tool_update_from_payload, cwd_outside_worktree,
    maybe_spool_tool_output, merge_tool_update, sanitize_tool_event_payload, tool_count_deltas,
};
use super::QueuedMessage;

fn provider_mode_id_for(
    provider_id: &str,
    control_mode: &ProviderControlMode,
) -> Option<&'static str> {
    match control_mode {
        ProviderControlMode::Full => match provider_id {
            "codex" => Some("full-access"),
            "claude-crp" => Some("bypassPermissions"),
            _ => None,
        },
        ProviderControlMode::HarnessNative | ProviderControlMode::CtxEnforced => None,
    }
}

pub(crate) async fn start_turn(
    state: &Arc<AppState>,
    session: &Session,
    workdir: &Path,
    session_root_kind: &str,
    queued: QueuedMessage,
    order_seq_state: Arc<Mutex<OrderSeqState>>,
) -> Result<RunningTurn> {
    state.wait_for_worktree_bootstrap(session.worktree_id).await;

    let store = state.store_for_session(session.id).await?;

    let workdir_root = workdir.to_path_buf();
    let workdir_canonical = tokio::fs::canonicalize(&workdir_root).await.ok();
    let workdir_str = workdir_root.to_string_lossy().to_string();
    let execution_environment = session.execution_environment;

    let mut message = queued.message;
    let message_id = message.id;
    let perf_run_id = queued.run_id.clone();
    let queue_wait_ms = queued.enqueued_at.elapsed().as_millis() as u64;
    let mut queue_labels = HashMap::new();
    queue_labels.insert("provider_id".to_string(), session.provider_id.clone());
    queue_labels.insert("model_id".to_string(), session.model_id.clone());
    queue_labels.insert(
        "execution_environment".to_string(),
        execution_environment.as_str().to_string(),
    );
    queue_labels.insert(
        "session_root_kind".to_string(),
        session_root_kind.to_string(),
    );
    queue_labels.insert("event".to_string(), "queue_wait".to_string());
    let queue_metric = PerfMetric {
        name: "scheduler.queue_wait_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: queue_wait_ms as f64,
        labels: queue_labels,
    };
    state
        .telemetry
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
        "execution_environment": execution_environment.as_str(),
        "session_root_kind": session_root_kind,
    }));
    state.telemetry.ops_events.emit(run_event);

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

    async fn emit_turn_start_failed(
        state: &Arc<AppState>,
        store: &ctx_store::Store,
        session: &Session,
        run_id: RunId,
        turn_id: TurnId,
        message_id: MessageId,
        err: &anyhow::Error,
    ) {
        let failed_at = Utc::now();
        let _ = store
            .update_session_turn_status(
                session.id,
                turn_id,
                SessionTurnStatus::Failed,
                None,
                None,
                failed_at,
            )
            .await;
        let _ = emit_event(
            state,
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::Error,
            json!({
                "message_id": message_id.0,
                "error": err.to_string(),
                "status": "failed",
            }),
        )
        .await;
        let _ = emit_event(
            state,
            session.id,
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

    let prompt = message.content.clone();
    let mut provider_session_ref = session.provider_session_ref.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &session.model_id, &prompt);

    let (ev_tx, mut ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let event_tx = ev_tx.clone();

    let mut provider_env = std::collections::HashMap::new();
    provider_env.insert("CTX_DAEMON_URL".to_string(), state.core.daemon_url.clone());
    provider_env.insert(
        "CTX_DATA_ROOT".to_string(),
        state.core.data_root.to_string_lossy().to_string(),
    );
    provider_env.insert("CTX_PROVIDER_ID".to_string(), session.provider_id.clone());
    provider_env.insert(
        "CLAUDE_CODE_ENABLE_ASK_USER_QUESTION_TOOL".to_string(),
        "1".to_string(),
    );
    if let Some(token) = state.core.auth_token.clone() {
        provider_env.insert("CTX_AUTH_TOKEN".to_string(), token);
    }
    if let Some(provider_ref) = session.provider_session_ref.clone() {
        provider_env.insert("CTX_PROVIDER_SESSION_REF".to_string(), provider_ref);
    }
    provider_env.insert("CTX_SESSION_ID".to_string(), session.id.0.to_string());
    provider_env.insert("CTX_MODEL_ID".to_string(), session.model_id.clone());
    let mcp_token = uuid::Uuid::new_v4().to_string();
    provider_env.insert("CTX_MCP_TOKEN".to_string(), mcp_token);
    let settings = settings::load_settings(state.global_store()).await?;
    let provider_control_mode = settings
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

    let workspace = match store.get_workspace(session.workspace_id).await {
        Ok(Some(workspace)) => workspace,
        Ok(None) => {
            let err = anyhow!("workspace not found: {}", session.workspace_id.0);
            emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
        Err(err) => {
            emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let worktree_for_runtime = match store.get_worktree(session.worktree_id).await {
        Ok(Some(worktree)) => worktree,
        Ok(None) => {
            let err = anyhow!("worktree not found: {}", session.worktree_id.0);
            emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
        Err(err) => {
            emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let execution_settings =
        match execution_effective::effective_execution_settings_for_environment(
            state.as_ref(),
            workspace.id,
            execution_environment,
        )
        .await
        {
            Ok(settings) => settings,
            Err(err) => {
                emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err)
                    .await;
                return Err(err);
            }
        };
    let runtime_plan = match state
        .execution
        .harness
        .prepare(
            &workspace,
            &worktree_for_runtime,
            &execution_settings,
            &state.core.daemon_url,
        )
        .await
    {
        Ok(plan) => plan,
        Err(err) => {
            emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let is_container = matches!(runtime_plan.runtime, HarnessRuntimeKind::Container { .. });
    for (key, value) in runtime_plan.env_overrides.iter() {
        provider_env.insert(key.clone(), value.clone());
    }
    let runtime_data_root = if is_container {
        runtime_plan
            .env_overrides
            .get("CTX_DATA_ROOT")
            .map(Path::new)
    } else {
        None
    };
    let resolved_source = match harness_sources::resolve_provider_source_for_run_with_runtime_root(
        &state.core.data_root,
        &session.provider_id,
        runtime_data_root,
    )
    .await
    {
        Ok(source) => source,
        Err(err) => {
            let err = anyhow!(
                "provider source resolution failed for {}: {}",
                session.provider_id,
                err
            );
            emit_turn_start_failed(state, &store, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let using_endpoint_source = resolved_source.source_kind == HarnessSourceKind::Endpoint;
    provider_env.insert(
        "CTX_PROVIDER_SOURCE_KIND".to_string(),
        match resolved_source.source_kind {
            HarnessSourceKind::Subscription => "subscription".to_string(),
            HarnessSourceKind::Endpoint => "endpoint".to_string(),
        },
    );
    if let Some(endpoint) = resolved_source.endpoint.as_ref() {
        provider_env.insert("CTX_PROVIDER_ENDPOINT_ID".to_string(), endpoint.id.clone());
        provider_env.insert(
            "CTX_PROVIDER_ENDPOINT_SHAPE".to_string(),
            endpoint.api_shape.as_str().to_string(),
        );
    }
    for (key, value) in resolved_source.env.iter() {
        provider_env.insert(key.clone(), value.clone());
    }

    let runtime_provider_id =
        runtime_provider_id_for_session_provider(&session.provider_id, &resolved_source);
    if runtime_provider_id != session.provider_id {
        provider_env.insert(
            "CTX_PROVIDER_RUNTIME_ID".to_string(),
            runtime_provider_id.to_string(),
        );
    }
    let install_target = if is_container {
        InstallTarget::Container
    } else {
        InstallTarget::Host
    };
    let adapter_cfg = installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let adapter = ensure_provider_adapter_for_target_with_cfg(
        state,
        &adapter_cfg,
        runtime_provider_id,
        install_target,
    )
    .await;

    if runtime_provider_id == "codex" && is_container && using_endpoint_source {
        if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
            provider_accounts::ensure_codex_endpoint_runtime_home_from_env(
                std::path::Path::new(root),
                &mut provider_env,
            )
            .await?;
        }
    }

    if runtime_provider_id == "codex"
        && !provider_env.contains_key("CODEX_HOME")
        && !using_endpoint_source
    {
        if is_container {
            if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
                let codex_home = provider_accounts::codex_runtime_home(std::path::Path::new(root));
                tokio::fs::create_dir_all(&codex_home).await.ok();
                provider_accounts::seed_codex_auth_from_host(&codex_home).await?;
                provider_env.insert(
                    "CODEX_HOME".to_string(),
                    codex_home.to_string_lossy().to_string(),
                );
            }
        } else {
            let env =
                provider_accounts::codex_env_for_active_account(&state.core.data_root).await?;
            for (key, value) in env {
                provider_env.insert(key, value);
            }
        }
    }
    if runtime_provider_id != "codex" && !using_endpoint_source {
        let env = if is_container {
            if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
                provider_accounts::subscription_env_for_active_account_with_runtime_root(
                    &state.core.data_root,
                    Path::new(root),
                    runtime_provider_id,
                )
                .await?
            } else {
                provider_accounts::subscription_env_for_active_account(
                    &state.core.data_root,
                    runtime_provider_id,
                )
                .await?
            }
        } else {
            provider_accounts::subscription_env_for_active_account(
                &state.core.data_root,
                runtime_provider_id,
            )
            .await?
        };
        for (key, value) in env {
            provider_env.insert(key, value);
        }
    }
    if runtime_provider_id == "codex" {
        let codex_home = provider_env
            .get("CODEX_HOME")
            .cloned()
            .ok_or_else(|| anyhow!("missing CODEX_HOME for {}", runtime_provider_id))?;
        provider_accounts::ensure_codex_auth_ready(Path::new(&codex_home))
            .await
            .map_err(|err| {
                if using_endpoint_source {
                    anyhow!(
                        "Codex endpoint credentials are not configured correctly. Open Settings -> Agent Harnesses and verify the selected endpoint. Details: {err}"
                    )
                } else {
                    anyhow!(
                        "Codex authentication is not configured. Open Settings -> Codex and add a subscription login or API key. Details: {err}"
                    )
                }
            })?;
        if is_container && using_endpoint_source {
            let openai_api_key_present = provider_env
                .get("OPENAI_API_KEY")
                .is_some_and(|value| !value.trim().is_empty());
            if !openai_api_key_present {
                anyhow::bail!(
                    "codex endpoint container runtime missing OPENAI_API_KEY after endpoint resolution"
                );
            }
            if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
                let expected_home = provider_accounts::codex_runtime_home(Path::new(root));
                if Path::new(&codex_home) != expected_home {
                    anyhow::bail!(
                        "codex endpoint container runtime must use CODEX_HOME={} but resolved {}",
                        expected_home.display(),
                        codex_home
                    );
                }
            }
        }
    }

    if is_container {
        if let Some(root) = runtime_plan.env_overrides.get("CTX_DATA_ROOT") {
            provider_accounts::ensure_provider_runtime_home_env(
                Path::new(root),
                runtime_provider_id,
                &mut provider_env,
            )
            .await?;
        }
    }

    installer::prepend_runtime_bin_dirs_to_provider_path_for_target(
        &mut provider_env,
        &adapter_cfg,
        runtime_provider_id,
        &state.core.data_root,
        Some(install_target),
    );

    let mut run_env_event = OpsEvent::new("info", "provider_run_env_ready");
    run_env_event.session_id = Some(session.id.0.to_string());
    run_env_event.worktree_id = Some(session.worktree_id.0.to_string());
    run_env_event.run_id = Some(run_id.0.to_string());
    run_env_event.turn_id = Some(turn_id.0.to_string());
    run_env_event.provider_id = Some(session.provider_id.clone());
    run_env_event.cwd = Some(workdir_str.clone());
    run_env_event.worktree_root = Some(workdir_str.clone());
    run_env_event.meta = Some(json!({
        "model_id": session.model_id.clone(),
        "execution_environment": execution_environment.as_str(),
        "session_root_kind": session_root_kind,
        "runtime_provider_id": runtime_provider_id,
        "source_kind": if using_endpoint_source { "endpoint" } else { "subscription" },
        "is_container": is_container,
        "has_openai_api_key": provider_env
            .get("OPENAI_API_KEY")
            .is_some_and(|value| !value.trim().is_empty()),
        "has_codex_home": provider_env
            .get("CODEX_HOME")
            .is_some_and(|value| !value.trim().is_empty()),
        "openai_base_url_host": provider_env
            .get("OPENAI_BASE_URL")
            .and_then(|value| url::Url::parse(value).ok())
            .and_then(|parsed| parsed.host_str().map(|host| host.to_string())),
    }));
    state.telemetry.ops_events.emit(run_env_event);

    let prompt_config = workspace_config::load_agent_system_prompt_append(&store)
        .await
        .unwrap_or_else(|_| workspace_config::AgentSystemPromptAppendConfig::new_default());
    let mut system_prompt_append = prompt_config.effective_append();
    if session.relationship.as_deref() == Some("sub_agent") {
        let subagent_config = workspace_config::load_subagent_system_prompt_append(&store)
            .await
            .unwrap_or_else(|_| workspace_config::SubagentSystemPromptAppendConfig::new_default());
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
            spawn_labels.insert(
                "execution_environment".to_string(),
                execution_environment.as_str().to_string(),
            );
            spawn_labels.insert(
                "session_root_kind".to_string(),
                session_root_kind.to_string(),
            );
            spawn_labels.insert("event".to_string(), "spawn".to_string());
            let spawn_metric = PerfMetric {
                name: "provider.spawn_ms".to_string(),
                kind: PerfMetricKind::Histogram,
                unit: "ms".to_string(),
                value: spawn_ms as f64,
                labels: spawn_labels,
            };
            state
                .telemetry
                .perf_telemetry
                .record_metric(spawn_metric, perf_run_id.clone(), None, None)
                .await;
            handle
        }
        Err(err) => {
            let duration_ms = run_started_at.elapsed().as_millis() as u64;
            state
                .telemetry
                .telemetry
                .emit(TelemetryEvent::provider_call(
                    session.provider_id.clone(),
                    session.model_id.clone(),
                    Some(execution_environment.as_str().to_string()),
                    Some(session_root_kind.to_string()),
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
                "execution_environment": execution_environment.as_str(),
                "session_root_kind": session_root_kind,
                "error": err.to_string(),
            }));
            state.telemetry.ops_events.emit(fail_event);
            let failed_at = Utc::now();
            let _ = store
                .update_session_turn_status(
                    session.id,
                    turn_id,
                    SessionTurnStatus::Failed,
                    None,
                    None,
                    failed_at,
                )
                .await;
            let _ = emit_event(
                state,
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::Error,
                json!({
                    "message_id": message_id.0,
                    "error": err.to_string(),
                    "status": "failed",
                }),
            )
            .await;
            let _ = emit_event(
                state,
                session.id,
                Some(run_id),
                Some(turn_id),
                SessionEventType::TurnFinished,
                json!({
                    "message_id": message_id.0,
                    "status": "failed",
                }),
            )
            .await;
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
    let session_root_kind = session_root_kind.to_string();
    let perf_run_id = perf_run_id.clone();
    let workdir_root = workdir_root.clone();
    let workdir_canonical = workdir_canonical.clone();
    let workdir_str = workdir_str.clone();
    let mut telemetry_emitted = false;

    let order_seq_state = Arc::clone(&order_seq_state);
    tokio::spawn(async move {
        let mut assistant_partial = String::new();
        let mut assistant_partial_message_id: Option<String> = None;
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
                first_labels.insert(
                    "execution_environment".to_string(),
                    execution_environment.as_str().to_string(),
                );
                first_labels.insert("session_root_kind".to_string(), session_root_kind.clone());
                first_labels.insert("event".to_string(), "first_event".to_string());
                let first_metric = PerfMetric {
                    name: "provider.first_event_ms".to_string(),
                    kind: PerfMetricKind::Histogram,
                    unit: "ms".to_string(),
                    value: first_ms as f64,
                    labels: first_labels,
                };
                state_for_events
                    .telemetry
                    .perf_telemetry
                    .record_metric(first_metric, perf_run_id.clone(), None, None)
                    .await;
            }
            if matches!(ev.event_type, SessionEventType::Init) {
                if payload.get("crp_session_id").is_some() {
                    state_for_events
                        .emit_compat_payload_reject_counter(
                            "scheduler.init_event",
                            "crp_session_id",
                            None,
                        )
                        .await;
                }
                let provider_session_id =
                    payload.get("provider_session_id").and_then(Value::as_str);
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
                        let metrics = if provider_id == "codex" {
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
                state_for_events.telemetry.ops_events.emit(event);

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
                        state_for_events.telemetry.ops_events.emit(warn_event);
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
            {
                let mut order_seq_state = order_seq_state.lock().await;
                attach_order_seq(
                    &mut order_seq_state,
                    &event_type,
                    &mut payload,
                    Some(&turn_id),
                    assistant_sequence,
                );
            }
            let event = match append_session_event_with_retry(
                &store,
                session_id,
                Some(run_id),
                Some(turn_id),
                event_type.clone(),
                payload,
            )
            .await
            {
                Ok(event) => event,
                Err(err) => {
                    tracing::warn!(
                        session_id = %session_id.0,
                        run_id = %run_id.0,
                        turn_id = %turn_id.0,
                        event_type = ?event_type,
                        "failed to append session event: {err:#}"
                    );
                    continue;
                }
            };
            state_for_events.publish_event(event.clone()).await;

            match event.event_type {
                SessionEventType::AssistantChunk => {
                    if let Some(fragment) =
                        raw_payload.get("content_fragment").and_then(Value::as_str)
                    {
                        assistant_partial.push_str(fragment);
                        if let Some(message_id) = raw_payload
                            .get("message_id")
                            .and_then(Value::as_str)
                            .map(|value| value.to_string())
                        {
                            assistant_partial_message_id = Some(message_id);
                        }
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
                            .workspaces
                            .workspace_active_snapshot
                            .publish_session_gap(workspace_id, session_id, event.seq, reason)
                            .await;
                    }
                }
                SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult => {
                    if !assistant_partial.is_empty() {
                        let message_id = ctx_core::ids::MessageId::new();
                        let order_seq = {
                            let mut order_seq_state = order_seq_state.lock().await;
                            order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
                        };
                        if let Ok(saved) = persist_assistant_message(
                            state_for_events.as_ref(),
                            &store,
                            workspace_id,
                            message_id,
                            order_seq,
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
                            let mut payload = json!({
                                "message_id": saved.id.0,
                                "content": saved.content,
                                "delivery": saved.delivery,
                                "attachments": saved.attachments,
                                "turn_sequence": saved.turn_sequence,
                                "order_seq": saved.order_seq,
                            });
                            if let Some(provider_message_id) = assistant_partial_message_id.take() {
                                if let Some(obj) = payload.as_object_mut() {
                                    obj.insert(
                                        "provider_message_id".to_string(),
                                        json!(provider_message_id),
                                    );
                                }
                            }
                            {
                                let mut order_seq_state = order_seq_state.lock().await;
                                attach_order_seq(
                                    &mut order_seq_state,
                                    &SessionEventType::AssistantMessageInserted,
                                    &mut payload,
                                    Some(&turn_id),
                                    assistant_sequence,
                                );
                            }
                            let _ = emit_event(
                                &state_for_events,
                                session_id,
                                Some(run_id),
                                Some(turn_id),
                                SessionEventType::AssistantMessageInserted,
                                payload,
                            )
                            .await;
                        }
                    }
                    if let Some(update) =
                        build_turn_tool_update_from_payload(&event_type, &raw_payload)
                    {
                        let prev = if matches!(event.event_type, SessionEventType::ToolCallUpdate) {
                            tool_cache.get(&update.tool_call_id).cloned()
                        } else if let Some(cached) = tool_cache.get(&update.tool_call_id).cloned() {
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
                    let provider_message_id = event
                        .payload_json
                        .get("message_id")
                        .or_else(|| event.payload_json.get("messageId"))
                        .and_then(Value::as_str)
                        .map(|s: &str| s.to_string())
                        .or_else(|| assistant_partial_message_id.clone());
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
                        if let Some(content) = strip_emitted_prefix(&content, &assistant_emitted) {
                            let message_id = ctx_core::ids::MessageId::new();
                            let order_seq = {
                                let mut order_seq_state = order_seq_state.lock().await;
                                order_seq_state
                                    .get_or_assign(format!("message:{}", message_id.0), None)
                            };
                            if let Ok(saved) = persist_assistant_message(
                                state_for_events.as_ref(),
                                &store,
                                workspace_id,
                                message_id,
                                order_seq,
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
                                let mut payload = json!({
                                    "message_id": saved.id.0,
                                    "content": saved.content,
                                    "delivery": saved.delivery,
                                    "attachments": saved.attachments,
                                    "turn_sequence": saved.turn_sequence,
                                    "order_seq": saved.order_seq,
                                });
                                if let Some(provider_message_id) = provider_message_id {
                                    if let Some(obj) = payload.as_object_mut() {
                                        obj.insert(
                                            "provider_message_id".to_string(),
                                            json!(provider_message_id),
                                        );
                                    }
                                }
                                let _ = emit_event(
                                    &state_for_events,
                                    session_id,
                                    Some(run_id),
                                    Some(turn_id),
                                    SessionEventType::AssistantMessageInserted,
                                    payload,
                                )
                                .await;
                                assistant_partial_message_id = None;
                            }
                        } else {
                            assistant_partial.clear();
                            assistant_partial_message_id = None;
                        }
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
                        run_labels.insert(
                            "execution_environment".to_string(),
                            execution_environment.as_str().to_string(),
                        );
                        run_labels
                            .insert("session_root_kind".to_string(), session_root_kind.clone());
                        run_labels.insert("event".to_string(), "run_complete".to_string());
                        let run_metric = PerfMetric {
                            name: "scheduler.run_total_ms".to_string(),
                            kind: PerfMetricKind::Histogram,
                            unit: "ms".to_string(),
                            value: duration_ms as f64,
                            labels: run_labels,
                        };
                        state_for_events
                            .telemetry
                            .perf_telemetry
                            .record_metric(run_metric, perf_run_id.clone(), None, None)
                            .await;
                        state_for_events
                            .telemetry
                            .telemetry
                            .emit(TelemetryEvent::provider_call(
                                provider_id.clone(),
                                model_id.clone(),
                                Some(execution_environment.as_str().to_string()),
                                Some(session_root_kind.clone()),
                                true,
                                duration_ms,
                            ))
                            .await;
                        state_for_events
                            .telemetry
                            .telemetry
                            .emit(TelemetryEvent::session_completed(
                                provider_id.clone(),
                                model_id.clone(),
                                Some(execution_environment.as_str().to_string()),
                                Some(session_root_kind.clone()),
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
                        run_labels.insert(
                            "execution_environment".to_string(),
                            execution_environment.as_str().to_string(),
                        );
                        run_labels
                            .insert("session_root_kind".to_string(), session_root_kind.clone());
                        run_labels.insert("event".to_string(), "run_interrupt".to_string());
                        let run_metric = PerfMetric {
                            name: "scheduler.run_total_ms".to_string(),
                            kind: PerfMetricKind::Histogram,
                            unit: "ms".to_string(),
                            value: duration_ms as f64,
                            labels: run_labels,
                        };
                        state_for_events
                            .telemetry
                            .perf_telemetry
                            .record_metric(run_metric, perf_run_id.clone(), None, None)
                            .await;
                        state_for_events
                            .telemetry
                            .telemetry
                            .emit(TelemetryEvent::provider_call(
                                provider_id.clone(),
                                model_id.clone(),
                                Some(execution_environment.as_str().to_string()),
                                Some(session_root_kind.clone()),
                                false,
                                duration_ms,
                            ))
                            .await;
                        state_for_events
                            .telemetry
                            .telemetry
                            .emit(TelemetryEvent::session_completed(
                                provider_id.clone(),
                                model_id.clone(),
                                Some(execution_environment.as_str().to_string()),
                                Some(session_root_kind.clone()),
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
                    let error_message = event
                        .payload_json
                        .get("message")
                        .and_then(|value| value.as_str())
                        .unwrap_or("provider runtime error")
                        .to_string();
                    if !telemetry_emitted {
                        telemetry_emitted = true;
                        let duration_ms = run_started_at.elapsed().as_millis() as u64;
                        let mut run_labels = HashMap::new();
                        run_labels.insert("provider_id".to_string(), provider_id.clone());
                        run_labels.insert("model_id".to_string(), model_id.clone());
                        run_labels.insert(
                            "execution_environment".to_string(),
                            execution_environment.as_str().to_string(),
                        );
                        run_labels
                            .insert("session_root_kind".to_string(), session_root_kind.clone());
                        run_labels.insert("event".to_string(), "run_failed".to_string());
                        let run_metric = PerfMetric {
                            name: "scheduler.run_total_ms".to_string(),
                            kind: PerfMetricKind::Histogram,
                            unit: "ms".to_string(),
                            value: duration_ms as f64,
                            labels: run_labels,
                        };
                        state_for_events
                            .telemetry
                            .perf_telemetry
                            .record_metric(run_metric, perf_run_id.clone(), None, None)
                            .await;
                        state_for_events
                            .telemetry
                            .telemetry
                            .emit(TelemetryEvent::provider_call(
                                provider_id.clone(),
                                model_id.clone(),
                                Some(execution_environment.as_str().to_string()),
                                Some(session_root_kind.clone()),
                                false,
                                duration_ms,
                            ))
                            .await;
                        state_for_events
                            .telemetry
                            .telemetry
                            .emit(TelemetryEvent::session_completed(
                                provider_id.clone(),
                                model_id.clone(),
                                Some(execution_environment.as_str().to_string()),
                                Some(session_root_kind.clone()),
                                "failed".to_string(),
                                duration_ms,
                            ))
                            .await;
                    }
                    let mut fail_event = OpsEvent::new("error", "provider_run_failed");
                    fail_event.session_id = Some(session_id.0.to_string());
                    fail_event.worktree_id = Some(worktree_id.0.to_string());
                    fail_event.run_id = Some(run_id.0.to_string());
                    fail_event.turn_id = Some(turn_id.0.to_string());
                    fail_event.provider_id = Some(provider_id.clone());
                    fail_event.cwd = Some(workdir_str.clone());
                    fail_event.worktree_root = Some(workdir_str.clone());
                    fail_event.meta = Some(json!({
                        "model_id": model_id.clone(),
                        "execution_environment": execution_environment.as_str(),
                        "session_root_kind": session_root_kind.clone(),
                        "error": error_message,
                        "details": event.payload_json.get("details").cloned(),
                        "kind": event.payload_json.get("kind").cloned(),
                    }));
                    state_for_events.telemetry.ops_events.emit(fail_event);
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
        let _ = events_done_tx.send(());
    });

    Ok(RunningTurn {
        adapter,
        handle,
        run_id,
        turn_id,
        event_tx,
        events_done: Some(events_done_rx),
    })
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
    matches!(provider_id, "claude-crp" | "codex")
}

fn runtime_provider_id_for_session_provider<'a>(
    session_provider_id: &'a str,
    _resolved_source: &harness_sources::ResolvedHarnessSource,
) -> &'a str {
    session_provider_id
}

#[cfg(test)]
mod runtime_tests {
    use super::{runtime_provider_id_for_session_provider, strip_emitted_prefix};
    use crate::harness_sources::{
        HarnessApiShape, HarnessEndpointRecord, HarnessEndpointVerificationStatus,
        HarnessSourceKind, ResolvedHarnessSource,
    };
    use crate::installer::{
        prepend_runtime_bin_dirs_to_provider_path, AgentServerCommand, AgentServerConfigFile,
        ManagedInstallMetadata,
    };
    use chrono::Utc;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::tempdir;

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

    #[test]
    fn gemini_bearer_endpoint_keeps_gemini_runtime_provider() {
        let source = ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Endpoint,
            endpoint: Some(HarnessEndpointRecord {
                id: "ep".to_string(),
                provider_id: "gemini".to_string(),
                name: "Gemini Legacy Bearer".to_string(),
                base_url: Some("https://openrouter.ai/api/v1".to_string()),
                api_shape: HarnessApiShape::OpenaiResponses,
                auth_type: "bearer".to_string(),
                model_override: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                last_verification_status: HarnessEndpointVerificationStatus::Unknown,
                last_verification_at: None,
                last_error: None,
                has_api_key: true,
                model_catalog_status: crate::harness_sources::EndpointModelCatalogStatus::Unknown,
                model_catalog_fetched_at: None,
                model_catalog_error: None,
                model_catalog_models: Vec::new(),
                manual_model_ids: Vec::new(),
                model_catalog_source: None,
            }),
            env: std::collections::HashMap::new(),
        };
        assert_eq!(
            runtime_provider_id_for_session_provider("gemini", &source),
            "gemini"
        );
    }

    #[test]
    fn gemini_subscription_keeps_gemini_runtime_provider() {
        let source = ResolvedHarnessSource {
            source_kind: HarnessSourceKind::Subscription,
            endpoint: None,
            env: std::collections::HashMap::new(),
        };
        assert_eq!(
            runtime_provider_id_for_session_provider("gemini", &source),
            "gemini"
        );
    }

    #[test]
    fn runtime_path_includes_command_parent_before_existing_path() {
        let tmp = tempdir().expect("tempdir");
        let data_root = tmp.path().join("data");
        let provider_bin_dir = tmp.path().join("provider-bin");
        std::fs::create_dir_all(&data_root).expect("data_root");
        std::fs::create_dir_all(&provider_bin_dir).expect("provider_bin_dir");
        let provider_cmd = provider_bin_dir.join("provider-cmd");
        std::fs::write(&provider_cmd, b"#!/bin/sh\n").expect("provider_cmd");

        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "test-provider".to_string(),
            AgentServerCommand {
                command: provider_cmd.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: Vec::new(),
                managed: None,
            },
        );

        let mut provider_env = HashMap::new();
        provider_env.insert("PATH".to_string(), "/usr/bin".to_string());
        prepend_runtime_bin_dirs_to_provider_path(
            &mut provider_env,
            &cfg,
            "test-provider",
            &data_root,
        );

        let path_value = provider_env.get("PATH").expect("path");
        let split: Vec<PathBuf> = std::env::split_paths(std::ffi::OsStr::new(path_value)).collect();
        let expected_first =
            std::fs::canonicalize(&provider_bin_dir).expect("canonical provider_bin_dir");
        assert_eq!(split.first().expect("first path"), &expected_first);
    }

    #[test]
    fn runtime_path_includes_dependency_bin_dirs() {
        let tmp = tempdir().expect("tempdir");
        let data_root = tmp.path().join("data");
        let provider_bin_dir = tmp.path().join("provider-bin");
        let managed_bin_rel = "managed/dep/bin";
        let managed_bin_dir = data_root.join(managed_bin_rel);
        std::fs::create_dir_all(&managed_bin_dir).expect("managed_bin_dir");
        std::fs::create_dir_all(&provider_bin_dir).expect("provider_bin_dir");
        let provider_cmd = provider_bin_dir.join("provider-cmd");
        std::fs::write(&provider_cmd, b"#!/bin/sh\n").expect("provider_cmd");

        let mut cfg = AgentServerConfigFile::default();
        cfg.providers.insert(
            "test-provider".to_string(),
            AgentServerCommand {
                command: provider_cmd.to_string_lossy().to_string(),
                args: Vec::new(),
                dependencies: vec!["dep-node".to_string()],
                managed: None,
            },
        );
        cfg.managed_installs.insert(
            "dep-node".to_string(),
            ManagedInstallMetadata {
                package: None,
                version: None,
                target: None,
                install_dir_rel: None,
                bin_dir_rel: Some(managed_bin_rel.to_string()),
                last_success_at: None,
                last_error: None,
            },
        );

        let mut provider_env = HashMap::new();
        provider_env.insert("PATH".to_string(), "/usr/bin".to_string());
        prepend_runtime_bin_dirs_to_provider_path(
            &mut provider_env,
            &cfg,
            "test-provider",
            &data_root,
        );

        let path_value = provider_env.get("PATH").expect("path");
        let split: Vec<PathBuf> = std::env::split_paths(std::ffi::OsStr::new(path_value)).collect();
        let expected_first =
            std::fs::canonicalize(&provider_bin_dir).expect("canonical provider_bin_dir");
        assert_eq!(split.first().expect("first path"), &expected_first);
        assert_eq!(split.get(1).expect("second path"), &managed_bin_dir);
    }
}
