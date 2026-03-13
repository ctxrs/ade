use std::collections::HashMap;
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

use crate::api::sessions::compose_model_id;
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

mod event_loop;
mod helpers;

use self::event_loop::{spawn_turn_event_loop, TurnEventLoop};
pub(crate) use self::helpers::model_context_window;
use self::helpers::{
    apply_provider_launch_overrides, compute_context_window_metrics, normalize_session_model_id,
    provider_supports_system_prompt_append, runtime_provider_id_for_session_provider,
};
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
            "droid" => Some("auto_high"),
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
    let provider_session_ref = session.provider_session_ref.clone();
    let context_window_metrics =
        compute_context_window_metrics(&session.provider_id, &full_model_id, &prompt);

    let (ev_tx, ev_rx) = mpsc::channel::<NormalizedEvent>(128);
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
        "model_id": full_model_id.clone(),
        "reasoning_effort": session.reasoning_effort.clone(),
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
    apply_provider_launch_overrides(runtime_provider_id, workdir, &mut provider_env).await?;

    let run_started_at = Instant::now();
    let spawn_started_at = Instant::now();
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

    spawn_turn_event_loop(TurnEventLoop {
        state: Arc::clone(state),
        store: store.clone(),
        session_id: session.id,
        task_id: session.task_id,
        workspace_id: session.workspace_id,
        worktree_id: session.worktree_id,
        provider_id: session.provider_id.clone(),
        model_id: full_model_id,
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
        context_window_metrics,
        ev_rx,
        events_done_tx,
        order_seq_state: Arc::clone(&order_seq_state),
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

#[cfg(test)]
mod runtime_tests {
    use super::helpers::strip_emitted_prefix;
    use super::provider_mode_id_for;
    use super::runtime_provider_id_for_session_provider;
    use crate::harness_sources::{
        HarnessApiShape, HarnessEndpointRecord, HarnessEndpointVerificationStatus,
        HarnessSourceKind, ResolvedHarnessSource,
    };
    use crate::installer::{
        prepend_runtime_bin_dirs_to_provider_path, AgentServerCommand, AgentServerConfigFile,
        ManagedInstallMetadata,
    };
    use crate::settings::ProviderControlMode;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn returns_full_when_no_emitted() {
        assert_eq!(strip_emitted_prefix("Hello", ""), Some("Hello".to_string()));
    }

    #[test]
    fn full_provider_control_maps_known_full_access_modes() {
        assert_eq!(
            provider_mode_id_for("codex", &ProviderControlMode::Full),
            Some("full-access")
        );
        assert_eq!(
            provider_mode_id_for("claude-crp", &ProviderControlMode::Full),
            Some("bypassPermissions")
        );
        assert_eq!(
            provider_mode_id_for("droid", &ProviderControlMode::Full),
            Some("auto_high")
        );
    }

    #[test]
    fn non_full_provider_control_does_not_force_provider_modes() {
        assert_eq!(
            provider_mode_id_for("droid", &ProviderControlMode::HarnessNative),
            None
        );
        assert_eq!(
            provider_mode_id_for("droid", &ProviderControlMode::CtxEnforced),
            None
        );
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
