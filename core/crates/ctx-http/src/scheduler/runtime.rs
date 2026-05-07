use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time::Instant as TokioInstant;

use ctx_core::ids::{RunId, TurnId};
use ctx_core::models::{
    MessageDelivery, MessageRole, Session, SessionEventType, SessionTurnStatus,
};
use ctx_providers::events::NormalizedEvent;
use ctx_session_tools::model_resolution::compose_model_id;
use ctx_session_tools::order_seq::OrderSeqState;

use crate::daemon::AppState;
use crate::settings;
use crate::storage_guard;
use ctx_workspace_config as workspace_config;

mod event_loop;
mod execution_plan;
mod helpers;
mod provider_env;
mod provider_launch;
mod provider_spawn;
#[cfg(test)]
mod tests;
mod tool_runtime;
mod turn_failure;
mod turn_input;
mod turn_start;

use self::event_loop::{spawn_turn_event_loop_for_session, TurnEventLoopSpawnRequest};
use self::execution_plan::prepare_turn_execution_plan;
use self::helpers::{compute_context_window_metrics, runtime_provider_id_for_session_provider};
use self::provider_env::{
    apply_runtime_source_env, build_base_provider_env, emit_provider_run_env_ready_event,
    prepare_provider_runtime_environment, BaseProviderEnvRequest, ProviderRunEnvReadyEvent,
    ProviderRuntimeEnvironmentRequest,
};
use self::provider_launch::prepare_provider_launch_environment;
use self::provider_spawn::{
    prepare_provider_adapter_for_turn, spawn_provider_turn, ProviderTurnSpawnRequest,
};
use self::turn_failure::emit_turn_start_failed;
use self::turn_input::prepare_turn_input;
use self::turn_start::{
    apply_crp_launch_policy_env_for_control_mode, emit_provider_run_started_event,
    record_queue_wait_metric, ProviderRunStartedEvent,
};
use super::lifecycle::{RunningTurn, TurnStartProgress};
use super::persistence::append_session_event_with_retry;
use super::policy_admission::{
    admit_runtime_turn, apply_turn_admission_env, RuntimeTurnAdmissionRequest,
};
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
    state.core.update_drain.reject_if_draining().await?;
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
    record_queue_wait_metric(
        state,
        session,
        &full_model_id,
        execution_environment.as_str(),
        session_root_kind,
        perf_run_id.clone(),
        queue_wait_ms,
    )
    .await;
    let run_id = message.run_id.get_or_insert_with(RunId::new).to_owned();
    let turn_id = message.turn_id.get_or_insert_with(TurnId::new).to_owned();

    emit_provider_run_started_event(ProviderRunStartedEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str: &workdir_str,
        full_model_id: &full_model_id,
        execution_environment: execution_environment.as_str(),
        session_root_kind,
    });

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

    let prompt = message.content.clone();
    let provider_session_ref = session.provider_session_ref.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &full_model_id, &prompt);

    let (ev_tx, ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, start_progress_rx) = watch::channel(TurnStartProgress::Pending);
    let event_tx = ev_tx.clone();

    let settings = settings::load_settings(state.global_store()).await?;
    let provider_control_mode = settings
        .sandboxing
        .as_ref()
        .map(|s| s.provider_control_mode.clone())
        .unwrap_or_default();
    let mut provider_env = build_base_provider_env(BaseProviderEnvRequest {
        daemon_url: &state.core.daemon_url,
        data_root: &state.core.data_root,
        session,
        full_model_id: &full_model_id,
        provider_control_mode: &provider_control_mode,
    });

    let execution_plan =
        match prepare_turn_execution_plan(state, &store, session, execution_environment).await {
            Ok(execution_plan) => execution_plan,
            Err(err) => {
                emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
                return Err(err);
            }
        };
    let execution_settings = execution_plan.execution_settings;
    let runtime_plan = execution_plan.runtime_plan;
    let is_linux_sandbox = runtime_plan.is_linux_sandbox();
    let source_env = match apply_runtime_source_env(
        &state.core.data_root,
        &session.provider_id,
        &runtime_plan,
        &mut provider_env,
    )
    .await
    {
        Ok(source_env) => source_env,
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    let resolved_source = source_env.resolved_source;
    let runtime_source_mode = source_env.runtime_source_mode;
    let using_endpoint_source = source_env.using_endpoint_source;

    let admission = match admit_runtime_turn(
        state,
        &store,
        RuntimeTurnAdmissionRequest {
            session,
            run_id,
            provider_id: &session.provider_id,
            model_id: &full_model_id,
            execution_environment,
            container_network_mode: execution_settings.container.network_mode.clone(),
            source_kind: resolved_source.source_kind,
        },
    )
    .await
    {
        Ok(admission) => admission,
        Err(err) => {
            emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
            return Err(err);
        }
    };
    apply_turn_admission_env(&mut provider_env, &admission);

    let runtime_provider_id =
        runtime_provider_id_for_session_provider(&session.provider_id, &resolved_source);
    if runtime_provider_id != session.provider_id {
        provider_env.insert(
            "CTX_PROVIDER_RUNTIME_ID".to_string(),
            runtime_provider_id.to_string(),
        );
    }
    let prepared_adapter =
        match prepare_provider_adapter_for_turn(state, runtime_provider_id, is_linux_sandbox).await
        {
            Ok(prepared_adapter) => prepared_adapter,
            Err(err) => {
                emit_turn_start_failed(state, session, run_id, turn_id, message_id, &err).await;
                return Err(err);
            }
        };

    prepare_provider_runtime_environment(ProviderRuntimeEnvironmentRequest {
        state,
        provider_env: &mut provider_env,
        runtime_provider_id,
        runtime_plan: &runtime_plan,
        is_linux_sandbox,
        runtime_source_mode,
        adapter_cfg: &prepared_adapter.adapter_cfg,
        install_target: prepared_adapter.install_target,
    })
    .await?;
    apply_crp_launch_policy_env_for_control_mode(&mut provider_env, &provider_control_mode);

    emit_provider_run_env_ready_event(ProviderRunEnvReadyEvent {
        state,
        session,
        run_id,
        turn_id,
        workdir_str: &workdir_str,
        full_model_id: &full_model_id,
        execution_environment: execution_environment.as_str(),
        session_root_kind,
        runtime_provider_id,
        using_endpoint_source,
        is_linux_sandbox,
        runtime_plan: &runtime_plan,
        provider_env: &provider_env,
    });

    let turn_input =
        prepare_turn_input(&store, session, &message, &full_model_id, &mut provider_env).await?;
    let launch_environment = prepare_provider_launch_environment(
        state,
        session,
        runtime_provider_id,
        workdir,
        &mut provider_env,
    )
    .await?;
    let mcp_token = launch_environment.mcp_token;
    let codex_home = launch_environment.codex_home;
    let start_deadline_duration = launch_environment.start_deadline_duration;

    let run_started_at = Instant::now();
    let handle = spawn_provider_turn(ProviderTurnSpawnRequest {
        state,
        store: &store,
        session,
        adapter: prepared_adapter.adapter.clone(),
        turn_input,
        workdir,
        provider_env,
        event_tx: ev_tx,
        perf_run_id: perf_run_id.clone(),
        run_id,
        turn_id,
        message_id,
        mcp_token: mcp_token.as_deref(),
        run_started_at,
        workdir_str: &workdir_str,
        full_model_id: &full_model_id,
        execution_environment,
        session_root_kind,
    })
    .await?;

    spawn_turn_event_loop_for_session(TurnEventLoopSpawnRequest {
        state,
        store: store.clone(),
        session,
        full_model_id: &full_model_id,
        session_root_kind,
        execution_environment_label: execution_environment.as_str(),
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
        adapter: prepared_adapter.adapter,
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
        start_deadline: TokioInstant::now() + start_deadline_duration,
        mcp_token,
    })
}
