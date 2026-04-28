use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time::Instant as TokioInstant;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{
    MessageDelivery, MessageRole, Session, SessionEventType, SessionTurnStatus, SessionTurnTool,
};
use ctx_providers::adapters::{ProviderRunHooks, ProviderSessionRefClaimHook, TurnInput};
use ctx_providers::events::NormalizedEvent;
use ctx_session_tools::{
    build_tool_ops_meta_from_normalized, build_turn_tool_update, merge_tool_update,
    normalize_tool_event, sanitize_normalized_tool_event_payload, tool_count_deltas,
};
use ctx_store::store::SessionTurnToolCountDeltas;

use crate::api::sessions::compose_model_id;
use crate::daemon::{ensure_provider_adapter_for_target_with_cfg, AppState};
use crate::execution_effective;
use crate::ops_events::OpsEvent;
use crate::order_seq::{attach_order_seq, read_order_seq, OrderSeqState};
use crate::perf_telemetry::{PerfMetric, PerfMetricKind};
use crate::settings;
use crate::storage_guard;
use crate::telemetry::TelemetryEvent;
use ctx_harness_sources::HarnessSourceKind;
use ctx_provider_install::install_state::InstallTarget;
use ctx_workspace_config as workspace_config;
use ctx_worktree_data_plane::apply_data_plane_to_execution_settings;

use crate::worktree_data_plane::resolve_worktree_data_plane;

mod event_loop;
mod helpers;
mod provider_env;
#[cfg(test)]
mod tests;
mod tool_runtime;
mod turn_start;

use self::event_loop::{spawn_turn_event_loop, TurnEventLoop};
use self::helpers::{
    apply_provider_launch_overrides, compute_context_window_metrics,
    load_system_prompt_append_for_relationship, normalize_session_model_id,
    provider_supports_system_prompt_append, runtime_provider_id_for_session_provider,
};
use self::provider_env::{emit_provider_run_env_ready_event, prepare_provider_runtime_environment};
use self::tool_runtime::{cwd_outside_worktree, maybe_spool_tool_output};
use self::turn_start::{provider_mode_id_for, turn_start_deadline};
use super::lifecycle::{RunningTurn, TurnStartProgress};
use super::persistence::append_session_event_with_retry;
use super::terminal::{finalize_failed_turn, FailedTurnTerminalization};
use super::QueuedMessage;

pub(crate) async fn start_turn(
    state: &Arc<AppState>,
    session: &Session,
    workdir: &Path,
    session_root_kind: &str,
    queued: QueuedMessage,
    order_seq_state: Arc<Mutex<OrderSeqState>>,
) -> Result<RunningTurn> {
    state.wait_for_worktree_bootstrap(session.worktree_id).await;
    state.reject_if_update_draining().await?;
    storage_guard::preflight_turn_start(state, workdir).await?;

    let store = state.store_for_session(session.id).await?;

    let workdir_root = workdir.to_path_buf();
    let workdir_canonical = tokio::fs::canonicalize(&workdir_root).await.ok();
    let workdir_str = workdir_root.to_string_lossy().to_string();
    let execution_environment = session.execution_environment;
    let full_model_id = compose_model_id(&session.model_id, session.reasoning_effort.as_deref());

    let mut message = queued.message;
    let message_id = message.id;
    let perf_run_id = queued.run_id.clone();
    let queue_wait_ms = queued.enqueued_at.elapsed().as_millis() as u64;
    let mut queue_labels = HashMap::new();
    queue_labels.insert("provider_id".to_string(), session.provider_id.clone());
    queue_labels.insert("model_id".to_string(), full_model_id.clone());
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
        "model_id": full_model_id.clone(),
        "reasoning_effort": session.reasoning_effort.clone(),
        "execution_environment": execution_environment.as_str(),
        "session_root_kind": session_root_kind,
    }));
    state.telemetry.ops_events.emit(run_event);

    if message.delivered_at.is_none() {
        store.mark_message_delivered(message.id).await?;
        message.delivery = MessageDelivery::Immediate;
        message.delivered_at = Some(Utc::now());
    }
    store
        .update_session_turn_status(
            session.id,
            turn_id,
            SessionTurnStatus::Starting,
            None,
            None,
            Utc::now(),
        )
        .await?;

    async fn emit_turn_start_failed(
        state: &Arc<AppState>,
        session: &Session,
        run_id: RunId,
        turn_id: TurnId,
        message_id: MessageId,
        err: &anyhow::Error,
    ) {
        let error_message = err.to_string();
        let _ = finalize_failed_turn(
            state,
            session.id,
            Some(run_id),
            turn_id,
            message_id,
            FailedTurnTerminalization {
                message: &error_message,
                reason: Some("start_failed"),
                details: None,
                kind: Some(json!("start_failed")),
                emit_error_event: true,
            },
        )
        .await;
    }

    let prompt = message.content.clone();
    let provider_session_ref = session.provider_session_ref.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &full_model_id, &prompt);

    let (ev_tx, ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, start_progress_rx) = watch::channel(TurnStartProgress::Pending);
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
    provider_env.insert("CTX_MODEL_ID".to_string(), full_model_id.clone());
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
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let worktree_for_runtime = match store.get_worktree(session.worktree_id).await {
        Ok(Some(worktree)) => worktree,
        Ok(None) => {
            let err = anyhow!("worktree not found: {}", session.worktree_id.0);
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
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
                emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
                return Err(err);
            }
        };
    let execution_settings = match resolve_worktree_data_plane(state, &worktree_for_runtime).await {
        Ok(data_plane) => {
            match apply_data_plane_to_execution_settings(&execution_settings, &data_plane) {
                Ok(settings) => settings,
                Err(err) => {
                    emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
                    return Err(err);
                }
            }
        }
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
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
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let is_linux_sandbox = runtime_plan.is_linux_sandbox();
    for (key, value) in runtime_plan.env_overrides.iter() {
        provider_env.insert(key.clone(), value.clone());
    }
    if let Err(err) =
        crate::mcp_command::configure_runtime_mcp_command(&mut provider_env, &state.core.data_root)
    {
        emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
        return Err(err);
    }
    let runtime_data_root = runtime_plan.runtime_data_root();
    let resolved_source =
        match ctx_harness_sources::resolve_provider_source_for_run_with_runtime_root(
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
                emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
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
    let install_target = if is_linux_sandbox {
        InstallTarget::Container
    } else {
        InstallTarget::Host
    };
    let adapter_cfg =
        match crate::daemon::load_managed_agent_server_config_or_err(&state.core.data_root).await {
            Ok(cfg) => cfg,
            Err(err) => {
                let err = anyhow!(err.to_string());
                emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
                return Err(err);
            }
        };
    let adapter = ensure_provider_adapter_for_target_with_cfg(
        state,
        &adapter_cfg,
        runtime_provider_id,
        install_target,
    )
    .await;

    prepare_provider_runtime_environment(
        state,
        &mut provider_env,
        runtime_provider_id,
        &runtime_plan,
        is_linux_sandbox,
        using_endpoint_source,
        &adapter_cfg,
        install_target,
    )
    .await?;

    emit_provider_run_env_ready_event(
        state,
        session,
        run_id,
        turn_id,
        &workdir_str,
        &full_model_id,
        execution_environment.as_str(),
        session_root_kind,
        runtime_provider_id,
        using_endpoint_source,
        is_linux_sandbox,
        &runtime_plan,
        &provider_env,
    );

    let system_prompt_append =
        load_system_prompt_append_for_relationship(&store, session.relationship.as_deref()).await?;
    let mut context_blocks = Vec::new();
    if let Some(append) = system_prompt_append.as_deref() {
        if !provider_supports_system_prompt_append(&session.provider_id) {
            context_blocks.push(json!({"type":"text","text": append}));
        }
        provider_env.insert("CTX_SYSTEM_PROMPT_APPEND".to_string(), append.to_string());
    }
    apply_provider_launch_overrides(runtime_provider_id, workdir, &mut provider_env).await?;
    let codex_home = provider_env
        .get("CODEX_HOME")
        .map(|value| PathBuf::from(value.as_str()));

    let run_started_at = Instant::now();
    let spawn_started_at = Instant::now();
    let claim_store = store.clone();
    let claim_session_id = session.id;
    let provider_session_ref_claim: ProviderSessionRefClaimHook = Arc::new(move |claim| {
        let claim_store = claim_store.clone();
        Box::pin(async move {
            if let Some(returned_ref) = claim.returned_provider_session_ref {
                claim_store
                    .claim_session_provider_session_ref(
                        claim_session_id,
                        returned_ref,
                        "provider.session_opened",
                    )
                    .await?;
            }
            Ok(())
        })
    });
    let handle = match adapter
        .run(
            TurnInput {
                content: prompt,
                attachments: message.attachments.clone(),
                context_blocks,
                model_id: normalize_session_model_id(&full_model_id),
            },
            workdir.to_path_buf(),
            provider_env,
            ev_tx,
            ProviderRunHooks {
                provider_session_ref_claim: Some(provider_session_ref_claim),
            },
        )
        .await
    {
        Ok(handle) => {
            let spawn_ms = spawn_started_at.elapsed().as_millis() as u64;
            let mut spawn_labels = HashMap::new();
            spawn_labels.insert("provider_id".to_string(), session.provider_id.clone());
            spawn_labels.insert("model_id".to_string(), full_model_id.clone());
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
                    full_model_id.clone(),
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
                "model_id": full_model_id.clone(),
                "reasoning_effort": session.reasoning_effort.clone(),
                "execution_environment": execution_environment.as_str(),
                "session_root_kind": session_root_kind,
                "error": err.to_string(),
            }));
            state.telemetry.ops_events.emit(fail_event);
            let error_message = err.to_string();
            let _ = finalize_failed_turn(
                state,
                session.id,
                Some(run_id),
                turn_id,
                message_id,
                FailedTurnTerminalization {
                    message: &error_message,
                    reason: Some("provider_start_failed"),
                    details: None,
                    kind: Some(json!("provider_start_failed")),
                    emit_error_event: true,
                },
            )
            .await;
            return Err(err);
        }
    };

    spawn_turn_event_loop(TurnEventLoop {
        state_weak: Arc::downgrade(state),
        store: store.clone(),
        session_id: session.id,
        task_id: session.task_id,
        workspace_id: session.workspace_id,
        worktree_id: session.worktree_id,
        provider_id: session.provider_id.clone(),
        model_id: full_model_id.clone(),
        session_root_kind: session_root_kind.to_string(),
        execution_environment_label: execution_environment.as_str().to_string(),
        perf_run_id: perf_run_id.clone(),
        workdir_root: workdir_root.clone(),
        workdir_canonical: workdir_canonical.clone(),
        workdir_str: workdir_str.clone(),
        run_started_at,
        run_id,
        turn_id,
        message_id,
        provider_session_ref,
        codex_home,
        context_window_metrics,
        ev_rx,
        events_done_tx,
        start_progress_tx,
        order_seq_state: Arc::clone(&order_seq_state),
    });

    Ok(RunningTurn {
        adapter,
        handle,
        run_id,
        turn_id,
        message_id,
        provider_id: session.provider_id.clone(),
        model_id: full_model_id.clone(),
        execution_environment_label: execution_environment.as_str().to_string(),
        session_root_kind: session_root_kind.to_string(),
        event_tx,
        events_done: Some(events_done_rx),
        start_progress: start_progress_rx,
        start_deadline: TokioInstant::now() + turn_start_deadline(),
    })
}
