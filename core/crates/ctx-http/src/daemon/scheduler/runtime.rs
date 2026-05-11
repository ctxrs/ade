use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::sync::{mpsc, oneshot, watch, Mutex};
use tokio::time::Instant as TokioInstant;

use ctx_core::models::Session;
use ctx_providers::events::NormalizedEvent;
use ctx_session_tools::order_seq::OrderSeqState;

use crate::daemon::AppState;

mod event_loop;
mod execution_plan;
mod helpers;
mod provider_env;
mod provider_launch;
mod provider_setup;
mod provider_spawn;
#[cfg(test)]
mod tests;
mod tool_runtime;
mod turn_context;
mod turn_failure;
mod turn_input;
mod turn_start;

use self::event_loop::{spawn_turn_event_loop_for_session, TurnEventLoopSpawnRequest};
use self::provider_launch::prepare_provider_launch_environment;
use self::provider_setup::{prepare_provider_turn_runtime, ProviderTurnRuntimeSetupRequest};
use self::provider_spawn::{spawn_provider_turn, ProviderTurnSpawnRequest};
use self::turn_context::{prepare_turn_runtime_context, TurnRuntimeContext};
use self::turn_input::prepare_turn_input;
use self::turn_start::{prepare_turn_start, PrepareTurnStartRequest};
use super::lifecycle::{RunningTurn, TurnStartProgress};
use super::QueuedMessage;

pub(crate) async fn start_turn(
    state: &Arc<AppState>,
    session: &Session,
    workdir: &Path,
    session_root_kind: &str,
    queued: QueuedMessage,
    order_seq_state: Arc<Mutex<OrderSeqState>>,
) -> Result<RunningTurn> {
    let TurnRuntimeContext {
        store,
        workdir_root,
        workdir_canonical,
        workdir_str,
        execution_environment,
        full_model_id,
    } = prepare_turn_runtime_context(state, session, workdir).await?;

    let turn_start = prepare_turn_start(PrepareTurnStartRequest {
        state,
        store: &store,
        session,
        workdir_str: &workdir_str,
        full_model_id: &full_model_id,
        execution_environment,
        session_root_kind,
        queued,
    })
    .await?;
    let message = turn_start.message;
    let message_id = turn_start.message_id;
    let perf_run_id = turn_start.perf_run_id;
    let run_id = turn_start.run_id;
    let turn_id = turn_start.turn_id;
    let provider_session_ref = turn_start.provider_session_ref;
    let context_window_metrics = turn_start.context_window_metrics;

    let (ev_tx, ev_rx) = mpsc::channel::<NormalizedEvent>(128);
    let (events_done_tx, events_done_rx) = oneshot::channel();
    let (start_progress_tx, start_progress_rx) = watch::channel(TurnStartProgress::Pending);
    let event_tx = ev_tx.clone();

    let provider_runtime = prepare_provider_turn_runtime(ProviderTurnRuntimeSetupRequest {
        state,
        store: &store,
        session,
        run_id,
        turn_id,
        message_id,
        workdir_str: &workdir_str,
        full_model_id: &full_model_id,
        execution_environment,
        session_root_kind,
    })
    .await?;
    let mut provider_env = provider_runtime.provider_env;
    let runtime_provider_id = provider_runtime.runtime_provider_id;
    let adapter = provider_runtime.adapter;

    let turn_input =
        prepare_turn_input(&store, session, &message, &full_model_id, &mut provider_env).await?;
    let launch_environment = prepare_provider_launch_environment(
        state,
        session,
        &runtime_provider_id,
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
        adapter: Arc::clone(&adapter),
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
        start_deadline: TokioInstant::now() + start_deadline_duration,
        mcp_token,
    })
}
