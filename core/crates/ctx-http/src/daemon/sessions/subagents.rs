use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use axum::http::StatusCode;
use axum::Json;

use crate::api::errors::ApiErrorResp;
use crate::api::providers::provider_status_for_target;
use crate::api::sessions::{
    build_subagent_result, build_subagent_result_for_session, context_window_for_run,
    context_window_for_session, diff_worktree_summary_for_session,
    estimate_context_window_for_prompt, estimate_context_window_for_prompt_len,
    load_provider_model_catalog_for_execution_environment, resolve_model_id,
    worktree_path_for_child, AgentInitReq, AgentInitResp, AgentInitResult, AgentReplyReq,
    AgentReplyResp, ModelCatalog, SubagentInterruptReq, SubagentInterruptResp, SubagentListItem,
    SubagentWaitReq, SubagentWaitResp,
};
use crate::daemon::AppState;
use crate::execution_effective;
use crate::logs;
use crate::scheduler::{InterruptTelemetryContext, QueuedMessage, SchedulerCommand};
use crate::settings as user_settings;
use crate::vcs_hooks;
use ctx_core::ids::{MessageId, RunId, SessionId, TaskId, TurnId, WorktreeId};
#[cfg(test)]
use ctx_core::models::SessionEvent;
use ctx_core::models::{
    Message, MessageDelivery, MessageRole, Session, SessionEventType, SessionStatus, SessionTurn,
    SessionTurnStatus, SubagentInvocation, SubagentInvocationChild, VcsKind, Workspace, Worktree,
};
#[cfg(test)]
use ctx_core::session_projection::turn_status_from_finished_payload;
use ctx_fs::vcs;

const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;

type ApiResult<T> = Result<T, (StatusCode, Json<ApiErrorResp>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubagentWorktreeSelection {
    Inherit,
    New,
}

fn api_error(status: StatusCode, error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn internal_api_error(error: impl ToString) -> (StatusCode, Json<ApiErrorResp>) {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        logs::redact_sensitive(&error.to_string()),
    )
}

async fn store_for_session(state: &AppState, session_id: SessionId) -> ApiResult<ctx_store::Store> {
    state
        .store_for_session(session_id)
        .await
        .map_err(internal_api_error)
}

async fn load_parent_session(
    state: &AppState,
    parent_id: SessionId,
) -> ApiResult<(ctx_store::Store, Session)> {
    let store = store_for_session(state, parent_id).await?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "parent session not found"))?;
    Ok((store, parent))
}

fn resolve_max_subagents_per_call(settings: &user_settings::Settings) -> usize {
    let configured = settings
        .subagents
        .as_ref()
        .and_then(|s| s.max_per_call)
        .filter(|value| *value > 0);
    configured
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_SUBAGENTS_PER_CALL)
}

fn parse_subagent_worktree(value: Option<&str>) -> Result<SubagentWorktreeSelection, String> {
    let trimmed = value.map(|raw| raw.trim()).filter(|raw| !raw.is_empty());
    match trimmed {
        Some("inherit") => Ok(SubagentWorktreeSelection::Inherit),
        Some("new") => Ok(SubagentWorktreeSelection::New),
        Some(_) => Err("worktree must be 'inherit' or 'new'".to_string()),
        None => Err("worktree is required".to_string()),
    }
}

fn normalize_reasoning_effort(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::api::sessions::normalize_effort_id)
        .filter(|value| !value.is_empty())
}

fn build_subagent_request_json(
    agents: &[crate::api::sessions::AgentInitItem],
) -> serde_json::Value {
    let mut items = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.iter().enumerate() {
        let prompt = agent.prompt.trim();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "position".to_string(),
            serde_json::Value::Number(serde_json::Number::from(idx as u64)),
        );
        obj.insert(
            "prompt_length".to_string(),
            serde_json::Value::Number(serde_json::Number::from(prompt.chars().count() as u64)),
        );
        if let Some(label) = agent
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
        if let Some(harness) = agent
            .harness
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "harness".to_string(),
                serde_json::Value::String(harness.to_string()),
            );
        }
        if let Some(model) = agent
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(reasoning_effort) =
            normalize_reasoning_effort(agent.reasoning_effort.as_deref())
        {
            obj.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(reasoning_effort),
            );
        }
        items.push(serde_json::Value::Object(obj));
    }

    serde_json::json!({
        "agents_total": agents.len(),
        "agents": items,
    })
}

fn default_catalog_model_id(catalog: Option<&ModelCatalog>) -> Option<&str> {
    catalog.and_then(ModelCatalog::default_model_id)
}

fn subagent_status_from_turn_status(status: SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Interrupted => "interrupted",
        SessionTurnStatus::Failed => "failed",
        SessionTurnStatus::Running | SessionTurnStatus::Queued => "running",
    }
}

fn subagent_terminal_status_from_turn_status(status: SessionTurnStatus) -> Option<&'static str> {
    match status {
        SessionTurnStatus::Completed => Some("completed"),
        SessionTurnStatus::Interrupted => Some("interrupted"),
        SessionTurnStatus::Failed => Some("failed"),
        SessionTurnStatus::Running | SessionTurnStatus::Queued => None,
    }
}

#[cfg(test)]
fn subagent_terminal_status_from_event(event: &SessionEvent) -> Option<&'static str> {
    match event.event_type {
        SessionEventType::Done => Some("completed"),
        SessionEventType::Error => Some("failed"),
        SessionEventType::TurnInterrupted => Some("interrupted"),
        SessionEventType::TurnFinished => turn_status_from_finished_payload(&event.payload_json)
            .and_then(subagent_terminal_status_from_turn_status),
        _ => None,
    }
}

async fn latest_terminal_turn_for_run(
    store: &ctx_store::Store,
    session_id: SessionId,
    run_id: RunId,
) -> Result<Option<SessionTurn>, String> {
    let turn = store
        .get_latest_turn_for_run(session_id, run_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    Ok(turn.and_then(|turn| {
        subagent_terminal_status_from_turn_status(turn.status.clone()).map(|_| turn)
    }))
}

async fn wait_for_run_terminal_turn(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Result<SessionTurn, String> {
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let mut rx = state.subscribe_session_event_head(session_id).await;
    if let Some(turn) = latest_terminal_turn_for_run(&store, session_id, run_id).await? {
        return Ok(turn);
    }

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    rx = state.subscribe_session_event_head(session_id).await;
                }
                if let Some(turn) = latest_terminal_turn_for_run(&store, session_id, run_id).await?
                {
                    return Ok(turn);
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if let Some(turn) = latest_terminal_turn_for_run(&store, session_id, run_id).await?
                {
                    return Ok(turn);
                }
            }
        }
    }
}

async fn emit_subagent_invocation_notice(
    state: &Arc<AppState>,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
    payload: serde_json::Value,
) -> ApiResult<()> {
    let store = store_for_session(state.as_ref(), parent_session_id).await?;
    let event = store
        .append_session_event(
            parent_session_id,
            None,
            parent_turn_id,
            SessionEventType::Notice,
            payload,
        )
        .await
        .map_err(internal_api_error)?;
    state.publish_event(event).await;
    Ok(())
}

async fn run_subagent_child(
    state: &Arc<AppState>,
    child: SubagentInvocationChild,
    parent_worktree_id: WorktreeId,
) -> Result<AgentInitResult, String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let store = state
        .store_for_session(child.child_session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let status = match wait_for_run_terminal_turn(state, child.child_session_id, run_id).await {
        Ok(turn) => subagent_status_from_turn_status(turn.status).to_string(),
        Err(_) => "unknown".to_string(),
    };

    let child_updated_at = chrono::Utc::now();
    let mut updated_child = child.clone();
    updated_child.status = status.clone();
    updated_child.updated_at = child_updated_at;
    store
        .upsert_subagent_invocation_child(updated_child)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;

    let content = store
        .get_last_assistant_message_for_run(child.child_session_id, run_id)
        .await
        .ok()
        .flatten()
        .map(|m| m.content);

    let context_window = context_window_for_run(state, child.child_session_id, run_id).await;
    build_subagent_result(
        state,
        parent_worktree_id,
        &child,
        status,
        content,
        context_window,
    )
    .await
}

async fn finalize_subagent_invocation(
    state: &Arc<AppState>,
    invocation_id: &str,
    tool_call_id: &str,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
) -> Result<(), String> {
    let store = state
        .store_for_session(parent_session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let Some(invocation) = store
        .get_subagent_invocation(invocation_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?
    else {
        return Ok(());
    };

    if invocation.children.is_empty() {
        return Ok(());
    }
    if invocation
        .children
        .iter()
        .any(|child| child.status == "running")
    {
        return Ok(());
    }

    let final_status = if invocation
        .children
        .iter()
        .all(|child| child.status == "completed")
    {
        "completed"
    } else {
        "failed"
    };
    if invocation.status == final_status {
        return Ok(());
    }

    let updated_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(invocation_id, final_status, updated_at)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    let child_session_ids = invocation
        .children
        .iter()
        .map(|child| child.child_session_id.0.to_string())
        .collect::<Vec<_>>();
    let child_statuses = invocation
        .children
        .iter()
        .map(|child| {
            serde_json::json!({
                "session_id": child.child_session_id.0.to_string(),
                "status": child.status,
            })
        })
        .collect::<Vec<_>>();
    emit_subagent_invocation_notice(
        state,
        parent_session_id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id,
            "tool_call_id": tool_call_id,
            "status": final_status,
            "child_session_ids": child_session_ids,
            "child_statuses": child_statuses,
        }),
    )
    .await
    .map_err(|(_, err)| err.0.error)?;

    Ok(())
}

async fn create_subagent_worktree(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    workspace: &Workspace,
    task_id: TaskId,
    base_commit_sha: &str,
    vcs_kind: VcsKind,
    effective: &crate::settings::ExecutionSettings,
) -> ApiResult<Worktree> {
    let worktree_id = WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task_id.0, worktree_id.0);
    let (wt_path, sandbox_binding) = crate::api::tasks::provision_worktree_for_execution(
        state,
        workspace,
        worktree_id,
        base_commit_sha,
        &branch_name,
        effective,
    )
    .await
    .map_err(|e| crate::api::shared::map_internal_api_error(&e))?;

    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: wt_path.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.to_string(),
        git_branch: (vcs_kind == VcsKind::Git).then(|| branch_name.clone()),
        vcs_kind: Some(vcs_kind),
        base_revision: Some(base_commit_sha.to_string()),
        vcs_ref: Some(branch_name),
        created_at: chrono::Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    };

    let worktree = crate::api::tasks::persist_provisioned_worktree(
        state,
        store,
        workspace,
        worktree,
        sandbox_binding,
    )
    .await
    .map_err(internal_api_error)?;
    if let Err(err) = vcs_hooks::ensure_task_commit_hook(state, workspace, &worktree, task_id).await
    {
        tracing::warn!(
            task_id = %task_id.0,
            worktree_id = %worktree.id.0,
            "failed to configure vcs hooks for subagent worktree: {err:#}"
        );
    }
    Ok(worktree)
}

async fn enqueue_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> ApiResult<(RunId, Message)> {
    let store = state
        .store_for_session(session.id)
        .await
        .map_err(internal_api_error)?;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    let order_seq_state = state.sessions.get_order_seq_state(&store, session.id).await;
    let order_seq = {
        let mut order_seq_state = order_seq_state.lock().await;
        order_seq_state.get_or_assign(format!("message:{}", message_id.0), None)
    };
    let msg = Message {
        id: message_id,
        session_id: session.id,
        task_id: session.task_id,
        run_id: Some(run_id),
        turn_id: Some(turn_id),
        turn_sequence: Some(0),
        order_seq: Some(order_seq),
        role: MessageRole::User,
        content: prompt,
        attachments: vec![],
        delivery: MessageDelivery::Immediate,
        delivered_at: None,
        created_at: chrono::Utc::now(),
    };
    let saved = store
        .insert_message(msg)
        .await
        .map_err(internal_api_error)?;
    let event = store
        .append_session_event(
            session.id,
            Some(run_id),
            Some(turn_id),
            SessionEventType::UserMessage,
            serde_json::json!({
                "message_id": saved.id.0,
                "content": saved.content.clone(),
                "delivery": saved.delivery.clone(),
                "attachments": saved.attachments,
                "order_seq": order_seq,
            }),
        )
        .await
        .map_err(internal_api_error)?;
    let start_seq = event.seq;

    let turn = SessionTurn {
        turn_id,
        session_id: session.id,
        run_id: Some(run_id),
        user_message_id: Some(saved.id),
        status: SessionTurnStatus::Running,
        start_seq: Some(start_seq),
        end_seq: None,
        started_at: saved.created_at,
        updated_at: saved.created_at,
        assistant_partial: None,
        thought_partial: None,
        metrics_json: None,
        tool_total: 0,
        tool_pending: 0,
        tool_running: 0,
        tool_completed: 0,
        tool_failed: 0,
    };
    let _ = store.insert_session_turn(turn).await;
    state.publish_event(event).await;

    let tx = state.ensure_scheduler(session.clone()).await;
    let queued = QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    Ok((run_id, saved))
}

pub(crate) async fn init_subagents(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: AgentInitReq,
) -> ApiResult<AgentInitResp> {
    if req.agents.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "agents is required"));
    }

    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(internal_api_error)?;
    let max_subagents = resolve_max_subagents_per_call(&settings);
    if req.agents.len() > max_subagents {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            format!("max {max_subagents} subagents per call"),
        ));
    }
    if req
        .response_mode
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "response_mode is not supported; use subagent_wait to await",
        ));
    }
    let worktree_selection = parse_subagent_worktree(req.worktree.as_deref())
        .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;

    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "workspace not found"))?;

    let mut labels = Vec::with_capacity(req.agents.len());
    let mut seen_labels = HashSet::new();
    for (idx, agent) in req.agents.iter().enumerate() {
        let label = agent
            .label
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    format!("agent {} label is required", idx + 1),
                )
            })?;
        if !seen_labels.insert(label.to_string()) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("duplicate subagent label '{label}'"),
            ));
        }
        labels.push(label.to_string());
    }
    for label in &labels {
        if store
            .subagent_label_exists(parent.task_id, label)
            .await
            .map_err(internal_api_error)?
        {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("subagent label '{label}' already exists for this task"),
            ));
        }
    }

    let mut provider_ids = HashSet::new();
    for agent in &req.agents {
        let provider_id = agent
            .harness
            .as_deref()
            .unwrap_or(&parent.provider_id)
            .trim();
        if provider_id.is_empty() {
            return Err(api_error(StatusCode::BAD_REQUEST, "harness is required"));
        }
        provider_ids.insert(provider_id.to_string());
    }

    let parent_worktree_execution = crate::api::tasks::resolve_existing_worktree_execution(
        &state,
        &store,
        &workspace,
        parent.worktree_id,
    )
    .await
    .map_err(internal_api_error)?;
    let parent_worktree = parent_worktree_execution.worktree.clone();
    let resolved_parent_execution_environment = parent_worktree_execution.execution_environment();
    if parent.execution_environment != resolved_parent_execution_environment {
        tracing::warn!(
            session_id = %parent.id.0,
            stored = parent.execution_environment.as_str(),
            resolved = resolved_parent_execution_environment.as_str(),
            "parent session execution_environment drifted from resolved worktree identity"
        );
    }
    let install_target = execution_effective::effective_install_target_for_environment(
        state.as_ref(),
        workspace.id,
        resolved_parent_execution_environment,
    )
    .await
    .map_err(internal_api_error)?;
    let managed = crate::installer::load_agent_server_config(&state.core.data_root)
        .await
        .unwrap_or_default();
    let matrix = crate::provider_matrix::load_matrix_cached(
        &state.core.data_root,
        &state.providers.matrix_cache,
    )
    .await;
    let mut known_providers = {
        let statuses = state.providers.statuses.lock().await;
        statuses.keys().cloned().collect::<HashSet<_>>()
    };
    for entry in &matrix.providers {
        if entry.kind == crate::provider_matrix::ProviderMatrixEntryKind::Harness {
            known_providers.insert(entry.id.clone());
        }
    }
    let mut available_providers = known_providers.iter().cloned().collect::<Vec<_>>();
    available_providers.sort();

    for provider_id in &provider_ids {
        if !known_providers.contains(provider_id) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!(
                    "unknown harness '{provider_id}'; available harnesses: {}",
                    available_providers.join(", ")
                ),
            ));
        }

        let status = provider_status_for_target(
            state.as_ref(),
            &managed,
            &matrix,
            provider_id,
            install_target,
        )
        .await;
        if !crate::provider_usability::provider_status_is_usable(&status) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!(
                    "harness '{provider_id}' is not ready: {}",
                    crate::provider_usability::provider_status_unusable_reason(&status)
                        .unwrap_or_else(|| "provider not ready for use".to_string())
                ),
            ));
        }
    }

    let mut model_catalogs: HashMap<String, Option<ModelCatalog>> = HashMap::new();
    for provider_id in &provider_ids {
        let catalog = load_provider_model_catalog_for_execution_environment(
            &state,
            &workspace,
            provider_id,
            resolved_parent_execution_environment,
        )
        .await;
        match catalog {
            Ok(cat) => {
                model_catalogs.insert(provider_id.clone(), cat);
            }
            Err(err) => {
                return Err(api_error(StatusCode::BAD_REQUEST, err));
            }
        }
    }

    let worktree_plan = if worktree_selection == SubagentWorktreeSelection::New {
        let base_commit_sha = crate::git_status::worktree_rev_parse_head(&state, &parent_worktree)
            .await
            .map_err(|e| {
                let msg = e.to_string().to_lowercase();
                if msg.contains("ambiguous argument 'head'")
                    || msg.contains("unknown revision or path not in the working tree")
                {
                    return api_error(
                        StatusCode::BAD_REQUEST,
                        "git repo has no commits; create an initial commit before creating a worktree",
                    );
                }
                internal_api_error(e)
            })?;
        let vcs = vcs::driver_for_kind(parent_worktree.vcs_kind.clone());
        let (dirty_files, dirty_additions, dirty_deletions) =
            diff_worktree_summary_for_session(&state, &parent_worktree, &base_commit_sha)
                .await
                .map_err(internal_api_error)?;
        if dirty_files > 0 || dirty_additions > 0 || dirty_deletions > 0 {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "Your worktree has uncommitted changes. Before starting new subagents in new worktree mode, you must commit or stash your changes to be explicit about whether subagents should inherit these diffs.",
            ));
        }
        Some((vcs.kind(), base_commit_sha))
    } else {
        None
    };

    let request_json = Some(build_subagent_request_json(&req.agents));
    let mut requested_tool_call_id = req
        .tool_call_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let invocation_id = requested_tool_call_id
        .clone()
        .unwrap_or_else(|| format!("subagent-{}", uuid::Uuid::new_v4()));
    let tool_call_id = requested_tool_call_id
        .take()
        .unwrap_or_else(|| invocation_id.clone());

    let mut parent_turn_id = None;
    if !tool_call_id.trim().is_empty() {
        if let Ok(Some(tool)) = store.get_session_turn_tool(parent.id, &tool_call_id).await {
            parent_turn_id = Some(tool.turn_id);
        }
    }
    if parent_turn_id.is_none() {
        if let Ok(turns) = store
            .list_session_turns_page_by_seq(parent.id, None, Some(5))
            .await
        {
            for turn in turns.iter().rev() {
                if matches!(
                    turn.status,
                    SessionTurnStatus::Running | SessionTurnStatus::Queued
                ) {
                    parent_turn_id = Some(turn.turn_id);
                    break;
                }
            }
        }
    }

    let now = chrono::Utc::now();
    let invocation = SubagentInvocation {
        id: invocation_id.clone(),
        tool_call_id: tool_call_id.clone(),
        parent_session_id: parent.id,
        parent_turn_id,
        requested_count: req.agents.len() as i64,
        request_json,
        status: "requested".to_string(),
        created_at: now,
        updated_at: now,
        children: Vec::new(),
    };
    store
        .upsert_subagent_invocation(invocation)
        .await
        .map_err(internal_api_error)?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_created",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "requested",
            "requested_count": req.agents.len(),
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let running_at = chrono::Utc::now();
    store
        .update_subagent_invocation_status(&invocation_id, "running", running_at)
        .await
        .map_err(internal_api_error)?;
    emit_subagent_invocation_notice(
        &state,
        parent.id,
        parent_turn_id,
        serde_json::json!({
            "kind": "subagent_invocation_updated",
            "invocation_id": invocation_id.clone(),
            "tool_call_id": tool_call_id.clone(),
            "status": "running",
            "child_session_ids": Vec::<String>::new(),
        }),
    )
    .await?;

    let child_ids = Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let parent_effective = parent_worktree_execution.effective.clone();

    let mut futures = Vec::with_capacity(req.agents.len());
    for (idx, agent) in req.agents.into_iter().enumerate() {
        let state = state.clone();
        let parent = parent.clone();
        let workspace = workspace.clone();
        let model_catalogs = model_catalogs.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let child_ids = child_ids.clone();
        let parent_turn_id = parent_turn_id;
        let label = labels
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("Subagent {}", idx + 1));
        let worktree_plan = worktree_plan.clone();
        let parent_effective = parent_effective.clone();
        futures.push(async move {
            let store = state
                .store_for_session(parent.id)
                .await
                .map_err(internal_api_error)?;
            let prompt = agent.prompt.trim().to_string();
            if prompt.is_empty() {
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    format!("agent {} prompt is required", idx + 1),
                ));
            }
            let harness_defaulted = agent.harness.is_none();
            let provider_id = agent
                .harness
                .as_deref()
                .unwrap_or(&parent.provider_id)
                .trim()
                .to_string();
            if harness_defaulted {
                state
                    .emit_product_fallback_applied_counter(
                        "sessions.subagent_init",
                        "harness_default_parent",
                        None,
                    )
                    .await;
            }
            let catalog = model_catalogs.get(&provider_id).and_then(|v| v.as_ref());
            let fallback_model = if agent.model.is_none() {
                if provider_id == parent.provider_id {
                    Some(parent.model_id.as_str())
                } else {
                    default_catalog_model_id(catalog)
                }
            } else {
                None
            };
            if agent.model.is_none() && fallback_model.is_none() {
                state
                    .emit_compat_payload_reject_counter(
                        "sessions.subagent_init",
                        "missing_model_without_default",
                        Some(("provider_id", &provider_id)),
                    )
                    .await;
                return Err(api_error(
                    StatusCode::BAD_REQUEST,
                    format!("model is required for harness '{provider_id}'"),
                ));
            }
            if agent.model.is_none() {
                let fallback = if provider_id == parent.provider_id {
                    "model_default_parent"
                } else {
                    "model_default_catalog"
                };
                state
                    .emit_product_fallback_applied_counter("sessions.subagent_init", fallback, None)
                    .await;
            }
            let resolved = resolve_model_id(
                agent.model.as_deref(),
                agent.reasoning_effort.as_deref(),
                fallback_model,
                catalog,
            )
            .map_err(|error| api_error(StatusCode::BAD_REQUEST, error))?;

            let prompt_length = prompt.chars().count() as i64;
            let reasoning_effort = resolved.reasoning_effort.clone();

            let worktree_id = match worktree_selection {
                SubagentWorktreeSelection::Inherit => parent.worktree_id,
                SubagentWorktreeSelection::New => {
                    let (vcs_kind, base_commit_sha) = worktree_plan.clone().ok_or_else(|| {
                        api_error(StatusCode::INTERNAL_SERVER_ERROR, "worktree plan missing")
                    })?;
                    let worktree = create_subagent_worktree(
                        &state,
                        &store,
                        &workspace,
                        parent.task_id,
                        &base_commit_sha,
                        vcs_kind,
                        &parent_effective,
                    )
                    .await?;
                    worktree.id
                }
            };

            let session = store
                .create_session_with_reasoning_effort(
                    parent.task_id,
                    parent.workspace_id,
                    worktree_id,
                    resolved_parent_execution_environment,
                    provider_id.clone(),
                    resolved.model_id.clone(),
                    reasoning_effort.clone(),
                    "subagent".into(),
                    Some(parent.id),
                    Some("sub_agent".to_string()),
                    None,
                )
                .await
                .map_err(internal_api_error)?;
            if let Err(err) = state
                .global_store()
                .upsert_workspace_session_index(session.id, parent.workspace_id)
                .await
            {
                tracing::warn!(
                    session_id = %session.id.0,
                    "failed to update subagent session index: {err:?}"
                );
            }
            if store
                .update_session_title(session.id, label.clone())
                .await
                .is_err()
            {
                tracing::warn!(session_id = %session.id.0, "failed to set subagent label");
            }

            let child_created_at = chrono::Utc::now();
            let (run_id, _message) = enqueue_subagent_prompt(&state, &session, prompt).await?;
            let child_session_id = session.id;
            let child = SubagentInvocationChild {
                invocation_id: invocation_id.clone(),
                child_session_id,
                run_id: Some(run_id),
                position: idx as i64,
                status: "running".to_string(),
                label: Some(label),
                harness: Some(provider_id),
                model: Some(resolved.full_model_id),
                reasoning_effort,
                prompt_length,
                created_at: child_created_at,
                updated_at: child_created_at,
            };
            store
                .upsert_subagent_invocation_child(child.clone())
                .await
                .map_err(internal_api_error)?;

            let child_ids_snapshot = {
                let mut ids = child_ids.lock().await;
                let child_id_string = child_session_id.0.to_string();
                ids.push(child_id_string);
                ids.clone()
            };
            emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "running",
                    "child_session_ids": child_ids_snapshot,
                }),
            )
            .await?;

            Ok(child)
        });
    }

    let children = match futures::future::try_join_all(futures).await {
        Ok(children) => children,
        Err(err) => {
            let updated_at = chrono::Utc::now();
            if let Ok(store) = state.store_for_session(parent.id).await {
                if let Err(e) = store
                    .update_subagent_invocation_status(&invocation_id, "failed", updated_at)
                    .await
                {
                    tracing::warn!(error = ?e, "failed to update subagent invocation status");
                }
            }
            let child_session_ids = {
                let ids = child_ids.lock().await;
                ids.clone()
            };
            let _ = emit_subagent_invocation_notice(
                &state,
                parent.id,
                parent_turn_id,
                serde_json::json!({
                    "kind": "subagent_invocation_updated",
                    "invocation_id": invocation_id.clone(),
                    "tool_call_id": tool_call_id.clone(),
                    "status": "failed",
                    "child_session_ids": child_session_ids,
                }),
            )
            .await;
            return Err(err);
        }
    };

    for child in children.iter().cloned() {
        let state = state.clone();
        let invocation_id = invocation_id.clone();
        let tool_call_id = tool_call_id.clone();
        let parent_id = parent.id;
        let parent_worktree_id = parent.worktree_id;
        tokio::spawn(async move {
            if let Err(error) = run_subagent_child(&state, child, parent_worktree_id).await {
                tracing::warn!(error = %error, "subagent execution failed");
            }
            if let Err(error) = finalize_subagent_invocation(
                &state,
                &invocation_id,
                &tool_call_id,
                parent_id,
                parent_turn_id,
            )
            .await
            {
                tracing::warn!(error = %error, "failed to finalize subagent invocation");
            }
        });
    }

    let results = futures::future::try_join_all(children.iter().map(|child| {
        let context_window = match (child.harness.as_deref(), child.model.as_deref()) {
            (Some(provider_id), Some(model_id)) => {
                estimate_context_window_for_prompt_len(provider_id, model_id, child.prompt_length)
            }
            _ => None,
        };
        build_subagent_result(
            &state,
            parent.worktree_id,
            child,
            "running".to_string(),
            None,
            context_window,
        )
    }))
    .await
    .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?;

    Ok(AgentInitResp {
        status: "running".to_string(),
        results,
    })
}

pub(crate) async fn reply_to_subagent(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: AgentReplyReq,
) -> ApiResult<AgentReplyResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;

    let label = req.label.trim();
    if label.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "label is required"));
    }
    let child = store
        .get_subagent_session_by_label(parent.id, label)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "subagent label not found"))?;

    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "prompt is required"));
    }

    if store
        .get_running_turn_for_session(child.id)
        .await
        .map_err(internal_api_error)?
        .is_some()
    {
        return Err(api_error(
            StatusCode::CONFLICT,
            "subagent_busy: The subagent is still running. Please await it with subagent_wait or interrupt it with subagent_interrupt.",
        ));
    }

    let mut context_window =
        estimate_context_window_for_prompt(&child.provider_id, &child.model_id, &prompt);
    if context_window.is_none() {
        context_window = context_window_for_session(&state, child.id).await;
    }

    let (_run_id, _message) = enqueue_subagent_prompt(&state, &child, prompt).await?;
    let worktree_path = worktree_path_for_child(&state, parent.worktree_id, child.id).await;

    Ok(AgentReplyResp {
        label: label.to_string(),
        status: "running".to_string(),
        context_window,
        worktree_path,
    })
}

pub(crate) async fn list_subagents(
    state: Arc<AppState>,
    parent_id: SessionId,
) -> ApiResult<Vec<SubagentListItem>> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;
    let subs = store
        .list_subagent_sessions(parent.id)
        .await
        .map_err(internal_api_error)?;

    let mut results = Vec::with_capacity(subs.len());
    for sub in subs {
        let label = sub.title.trim().to_string();
        let status = match sub.status {
            SessionStatus::Active => "active",
            SessionStatus::Completed => "completed",
            SessionStatus::Failed => "failed",
            SessionStatus::Cancelled => "cancelled",
        }
        .to_string();
        let context_window = context_window_for_session(&state, sub.id).await;
        let worktree_path = worktree_path_for_child(&state, parent.worktree_id, sub.id).await;
        results.push(SubagentListItem {
            label,
            status,
            context_window,
            worktree_path,
        });
    }

    Ok(results)
}

pub(crate) async fn interrupt_subagents(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: SubagentInterruptReq,
) -> ApiResult<SubagentInterruptResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;

    let use_all = req.all.unwrap_or(false);
    let labels = match (use_all, req.label) {
        (true, Some(_)) => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "provide either label or all",
            ));
        }
        (true, None) => {
            let subs = store
                .list_subagent_sessions(parent.id)
                .await
                .map_err(internal_api_error)?;
            subs.into_iter()
                .map(|sub| sub.title.trim().to_string())
                .collect::<Vec<_>>()
        }
        (false, Some(label)) => vec![label],
        (false, None) => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "label or all is required",
            ));
        }
    };

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(api_error(StatusCode::BAD_REQUEST, "label cannot be empty"));
        }
        let child = store
            .get_subagent_session_by_label(parent.id, trimmed)
            .await
            .map_err(internal_api_error)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("subagent label '{trimmed}' not found"),
                )
            })?;

        let tx = state.ensure_scheduler(child.clone()).await;
        let interrupt = InterruptTelemetryContext::new(uuid::Uuid::new_v4().to_string());
        let _ = tx.send(SchedulerCommand::Interrupt(interrupt)).await;

        let context_window = context_window_for_session(&state, child.id).await;
        results.push(
            build_subagent_result_for_session(
                &state,
                parent.worktree_id,
                &child,
                trimmed.to_string(),
                "interrupt_requested".to_string(),
                None,
                context_window,
            )
            .await
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?,
        );
    }

    Ok(SubagentInterruptResp {
        status: "interrupt_requested".to_string(),
        results,
    })
}

pub(crate) async fn wait_for_subagents(
    state: Arc<AppState>,
    parent_id: SessionId,
    req: SubagentWaitReq,
) -> ApiResult<SubagentWaitResp> {
    let (store, parent) = load_parent_session(state.as_ref(), parent_id).await?;

    let mut labels = match (req.label, req.labels) {
        (Some(label), None) => vec![label],
        (None, Some(labels)) => labels,
        (Some(_), Some(_)) => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "provide either label or labels",
            ));
        }
        (None, None) => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "label or labels is required",
            ));
        }
    };
    if labels.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "labels is required"));
    }
    let mut seen = HashSet::new();
    for label in &mut labels {
        let trimmed = label.trim().to_string();
        if trimmed.is_empty() {
            return Err(api_error(StatusCode::BAD_REQUEST, "label cannot be empty"));
        }
        if !seen.insert(trimmed.clone()) {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("duplicate label '{trimmed}'"),
            ));
        }
        *label = trimmed;
    }

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let child = store
            .get_subagent_session_by_label(parent.id, &label)
            .await
            .map_err(internal_api_error)?
            .ok_or_else(|| {
                api_error(
                    StatusCode::NOT_FOUND,
                    format!("subagent label '{label}' not found"),
                )
            })?;

        let running_turn = store
            .get_running_turn_for_session(child.id)
            .await
            .map_err(internal_api_error)?;

        let (status, run_id) = if let Some(turn) = running_turn {
            let run_id = turn.run_id.ok_or_else(|| {
                api_error(
                    StatusCode::BAD_REQUEST,
                    "subagent run_id missing; cannot wait",
                )
            })?;
            let status = match wait_for_run_terminal_turn(&state, child.id, run_id).await {
                Ok(turn) => subagent_status_from_turn_status(turn.status).to_string(),
                Err(_) => "unknown".to_string(),
            };
            (status, Some(run_id))
        } else if let Some(turn) = store
            .get_latest_turn_for_session(child.id)
            .await
            .map_err(internal_api_error)?
        {
            let status = subagent_status_from_turn_status(turn.status).to_string();
            (status, turn.run_id)
        } else {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("subagent '{label}' has no runs to wait for"),
            ));
        };

        let content = match run_id {
            Some(run_id) => store
                .get_last_assistant_message_for_run(child.id, run_id)
                .await
                .ok()
                .flatten()
                .map(|m| m.content),
            None => None,
        };
        let context_window = match run_id {
            Some(run_id) => context_window_for_run(&state, child.id, run_id).await,
            None => context_window_for_session(&state, child.id).await,
        };

        results.push(
            build_subagent_result_for_session(
                &state,
                parent.worktree_id,
                &child,
                label,
                status,
                content,
                context_window,
            )
            .await
            .map_err(|error| api_error(StatusCode::INTERNAL_SERVER_ERROR, error))?,
        );
    }

    Ok(SubagentWaitResp {
        status: crate::api::sessions::aggregate_subagent_status(&results).to_string(),
        results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::Utc;
    use serde_json::json;

    use ctx_core::ids::{SessionEventId, TurnId};

    fn terminal_event(status: &str) -> SessionEvent {
        SessionEvent {
            seq: 1,
            id: SessionEventId::new(),
            session_id: SessionId::new(),
            run_id: Some(RunId::new()),
            turn_id: Some(TurnId::new()),
            event_type: SessionEventType::TurnFinished,
            payload_json: json!({ "status": status }),
            transient: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn subagent_terminal_status_uses_turn_finished_failed_payload() {
        assert_eq!(
            subagent_terminal_status_from_event(&terminal_event("failed")),
            Some("failed")
        );
    }

    #[test]
    fn subagent_terminal_status_uses_turn_finished_interrupted_payload() {
        assert_eq!(
            subagent_terminal_status_from_event(&terminal_event("interrupted")),
            Some("interrupted")
        );
    }
}
