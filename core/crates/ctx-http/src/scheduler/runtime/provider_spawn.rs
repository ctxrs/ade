use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Error, Result};
use serde_json::json;
use tokio::sync::mpsc;

use ctx_core::ids::{MessageId, RunId, TurnId};
use ctx_core::models::{ExecutionEnvironment, Session};
use ctx_provider_install::install_state::InstallTarget;
use ctx_providers::adapters::{
    ProviderAdapter, ProviderRunHooks, ProviderSessionRefClaimHook, RunHandle, TurnInput,
};
use ctx_providers::events::NormalizedEvent;
use ctx_store::Store;

use crate::daemon::{ensure_provider_adapter_for_target_with_cfg, AppState};
use crate::installer;
use ctx_observability::ops_events::OpsEvent;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_observability::telemetry::TelemetryEvent;

use super::super::terminal::{finalize_failed_turn, FailedTurnTerminalization};

pub(super) struct PreparedProviderAdapter {
    pub(super) adapter: Arc<dyn ProviderAdapter>,
    pub(super) adapter_cfg: installer::AgentServerConfigFile,
    pub(super) install_target: InstallTarget,
}

pub(super) async fn prepare_provider_adapter_for_turn(
    state: &Arc<AppState>,
    runtime_provider_id: &str,
    is_linux_sandbox: bool,
) -> Result<PreparedProviderAdapter> {
    let install_target = provider_install_target_for_runtime(is_linux_sandbox);
    let adapter_cfg = crate::daemon::load_managed_agent_server_config_or_err(&state.core.data_root)
        .await
        .map_err(|err| anyhow!(err.to_string()))?;
    let adapter = ensure_provider_adapter_for_target_with_cfg(
        state.as_ref(),
        &adapter_cfg,
        runtime_provider_id,
        install_target,
    )
    .await;

    Ok(PreparedProviderAdapter {
        adapter,
        adapter_cfg,
        install_target,
    })
}

fn provider_install_target_for_runtime(is_linux_sandbox: bool) -> InstallTarget {
    if is_linux_sandbox {
        InstallTarget::Container
    } else {
        InstallTarget::Host
    }
}

pub(super) struct ProviderTurnSpawnRequest<'a> {
    pub(super) state: &'a Arc<AppState>,
    pub(super) store: &'a Store,
    pub(super) session: &'a Session,
    pub(super) adapter: Arc<dyn ProviderAdapter>,
    pub(super) turn_input: TurnInput,
    pub(super) workdir: &'a Path,
    pub(super) provider_env: HashMap<String, String>,
    pub(super) event_tx: mpsc::Sender<NormalizedEvent>,
    pub(super) perf_run_id: Option<String>,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) message_id: MessageId,
    pub(super) mcp_token: Option<&'a str>,
    pub(super) run_started_at: Instant,
    pub(super) workdir_str: &'a str,
    pub(super) full_model_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) session_root_kind: &'a str,
}

pub(super) async fn spawn_provider_turn(
    request: ProviderTurnSpawnRequest<'_>,
) -> Result<RunHandle> {
    let ProviderTurnSpawnRequest {
        state,
        store,
        session,
        adapter,
        turn_input,
        workdir,
        provider_env,
        event_tx,
        perf_run_id,
        run_id,
        turn_id,
        message_id,
        mcp_token,
        run_started_at,
        workdir_str,
        full_model_id,
        execution_environment,
        session_root_kind,
    } = request;
    let spawn_started_at = Instant::now();
    let provider_run_hooks = build_provider_run_hooks(
        state,
        store,
        session,
        execution_environment,
        session_root_kind,
    );
    let handle = match adapter
        .run(
            turn_input,
            workdir.to_path_buf(),
            provider_env,
            event_tx,
            provider_run_hooks,
        )
        .await
    {
        Ok(handle) => {
            record_provider_spawn_metric(
                state,
                perf_run_id,
                session,
                full_model_id,
                execution_environment,
                session_root_kind,
                spawn_started_at,
            )
            .await;
            handle
        }
        Err(err) => {
            handle_provider_start_failure(
                state,
                ProviderStartFailure {
                    session,
                    run_id,
                    turn_id,
                    message_id,
                    mcp_token,
                    run_started_at,
                    workdir_str,
                    full_model_id,
                    execution_environment,
                    session_root_kind,
                    err: &err,
                },
            )
            .await;
            return Err(err);
        }
    };

    Ok(handle)
}

pub(super) fn build_provider_run_hooks(
    state: &Arc<AppState>,
    store: &Store,
    session: &Session,
    execution_environment: ExecutionEnvironment,
    session_root_kind: &str,
) -> ProviderRunHooks {
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
    let provider_unknown_event =
        ctx_observability::provider_unknown_events::provider_unknown_event_hook(
            state.telemetry.provider_unknown_events.clone(),
            ctx_observability::provider_unknown_events::ProviderUnknownEventContext {
                provider_id: session.provider_id.clone(),
                execution_environment: Some(execution_environment.as_str().to_string()),
                session_root_kind: Some(session_root_kind.to_string()),
                operation: "turn".to_string(),
            },
        );
    ProviderRunHooks {
        provider_session_ref_claim: Some(provider_session_ref_claim),
        provider_unknown_event: Some(provider_unknown_event),
    }
}

pub(super) async fn record_provider_spawn_metric(
    state: &Arc<AppState>,
    perf_run_id: Option<String>,
    session: &Session,
    full_model_id: &str,
    execution_environment: ExecutionEnvironment,
    session_root_kind: &str,
    spawn_started_at: Instant,
) {
    let spawn_ms = spawn_started_at.elapsed().as_millis() as u64;
    let mut spawn_labels = HashMap::new();
    spawn_labels.insert("provider_id".to_string(), session.provider_id.clone());
    spawn_labels.insert("model_id".to_string(), full_model_id.to_string());
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
        .record_metric(spawn_metric, perf_run_id, None, None)
        .await;
}

pub(super) struct ProviderStartFailure<'a> {
    pub(super) session: &'a Session,
    pub(super) run_id: RunId,
    pub(super) turn_id: TurnId,
    pub(super) message_id: MessageId,
    pub(super) mcp_token: Option<&'a str>,
    pub(super) run_started_at: Instant,
    pub(super) workdir_str: &'a str,
    pub(super) full_model_id: &'a str,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) session_root_kind: &'a str,
    pub(super) err: &'a Error,
}

pub(super) async fn handle_provider_start_failure(
    state: &Arc<AppState>,
    failure: ProviderStartFailure<'_>,
) {
    let ProviderStartFailure {
        session,
        run_id,
        turn_id,
        message_id,
        mcp_token,
        run_started_at,
        workdir_str,
        full_model_id,
        execution_environment,
        session_root_kind,
        err,
    } = failure;
    if let Some(token) = mcp_token {
        crate::daemon::revoke_provider_session_mcp_token(state.as_ref(), token).await;
    }
    let duration_ms = run_started_at.elapsed().as_millis() as u64;
    state
        .telemetry
        .telemetry
        .emit(TelemetryEvent::provider_call(
            session.provider_id.clone(),
            full_model_id.to_string(),
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
    fail_event.cwd = Some(workdir_str.to_string());
    fail_event.worktree_root = Some(workdir_str.to_string());
    fail_event.meta = Some(json!({
        "model_id": full_model_id,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_sandbox_runtime_uses_container_install_target() {
        assert_eq!(
            provider_install_target_for_runtime(true),
            InstallTarget::Container
        );
    }

    #[test]
    fn host_runtime_uses_host_install_target() {
        assert_eq!(
            provider_install_target_for_runtime(false),
            InstallTarget::Host
        );
    }
}
