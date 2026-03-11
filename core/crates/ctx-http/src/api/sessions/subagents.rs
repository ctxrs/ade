use super::models::{
    load_provider_model_catalog, normalize_effort_id, resolve_model_id, split_model_id,
    ModelCatalog,
};
use super::*;

pub(crate) async fn list_session_subagents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let subs = store
        .list_subagent_sessions(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(subs))
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct SessionSubagentInvocationsQuery {
    turn_id: Option<String>,
}

pub(crate) async fn list_session_subagent_invocations(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<SessionSubagentInvocationsQuery>,
) -> Result<Json<Vec<SubagentInvocation>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let turn_id = match q.turn_id {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(TurnId(
                    uuid::Uuid::parse_str(trimmed).map_err(|_| StatusCode::BAD_REQUEST)?,
                ))
            }
        }
        None => None,
    };

    let invocations = store
        .list_subagent_invocations_for_session(session.id, turn_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(invocations))
}

pub(crate) async fn get_subagent_invocation(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<SubagentInvocation>, StatusCode> {
    let store = state
        .store_for_subagent_invocation(&id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let invocation = store
        .get_subagent_invocation(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(invocation))
}

const DEFAULT_MAX_SUBAGENTS_PER_CALL: usize = 10;

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
#[derive(Debug, Deserialize)]
pub(crate) struct AgentInitReq {
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    response_mode: Option<String>,
    #[serde(default)]
    worktree: Option<String>,
    agents: Vec<AgentInitItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct AgentInitItem {
    prompt: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubagentWorktreeSelection {
    Inherit,
    New,
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

fn build_subagent_request_json(agents: &[AgentInitItem]) -> serde_json::Value {
    let mut items = Vec::with_capacity(agents.len());
    for (idx, agent) in agents.iter().enumerate() {
        let prompt = agent.prompt.trim();
        let mut obj = serde_json::Map::new();
        obj.insert(
            "position".to_string(),
            serde_json::Value::Number(serde_json::Number::from(idx as u64)),
        );
        let prompt_length = prompt.chars().count() as u64;
        obj.insert(
            "prompt_length".to_string(),
            serde_json::Value::Number(serde_json::Number::from(prompt_length)),
        );
        if let Some(label) = agent
            .label
            .as_deref()
            .map(|s| s.trim())
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
            .map(|s| s.trim())
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
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "model".to_string(),
                serde_json::Value::String(model.to_string()),
            );
        }
        if let Some(reasoning_effort) = agent
            .reasoning_effort
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            let norm = normalize_effort_id(reasoning_effort);
            if !norm.is_empty() {
                obj.insert(
                    "reasoning_effort".to_string(),
                    serde_json::Value::String(norm),
                );
            }
        }
        items.push(serde_json::Value::Object(obj));
    }

    serde_json::json!({
        "agents_total": agents.len(),
        "agents": items,
    })
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentInitResp {
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentInitResult {
    pub(super) label: String,
    pub(super) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) worktree_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct ContextWindowSummary {
    pub(super) total: u64,
    pub(super) used: u64,
    pub(super) remaining: u64,
    pub(super) utilization: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubagentWaitReq {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    labels: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubagentWaitResp {
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubagentInterruptReq {
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubagentInterruptResp {
    status: String,
    results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubagentListItem {
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentReplyReq {
    label: String,
    prompt: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentReplyResp {
    label: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worktree_path: Option<String>,
}

async fn wait_for_run_terminal_event(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Result<SessionEventType, String> {
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    if let Some(event) = store
        .get_terminal_event_for_run(session_id, run_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?
    {
        return Ok(event.event_type);
    }

    let mut rx = state.get_broadcaster(session_id).await.subscribe();
    loop {
        match rx.recv().await {
            Ok(event) => {
                if event.run_id == Some(run_id)
                    && matches!(
                        event.event_type,
                        SessionEventType::Done
                            | SessionEventType::Error
                            | SessionEventType::TurnInterrupted
                            | SessionEventType::TurnFinished
                    )
                {
                    return Ok(event.event_type);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                if let Some(event) = store
                    .get_terminal_event_for_run(session_id, run_id)
                    .await
                    .map_err(|e| logs::redact_sensitive(&e.to_string()))?
                {
                    return Ok(event.event_type);
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return Err("session event stream closed".to_string());
            }
        }
    }
}

async fn emit_subagent_invocation_notice(
    state: &Arc<AppState>,
    parent_session_id: SessionId,
    parent_turn_id: Option<TurnId>,
    payload: serde_json::Value,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let store = state
        .store_for_session(parent_session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let event = store
        .append_session_event(
            parent_session_id,
            None,
            parent_turn_id,
            SessionEventType::Notice,
            payload,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state.publish_event(event).await;
    Ok(())
}

fn parse_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(num) => num.as_u64(),
        serde_json::Value::String(raw) => raw.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn parse_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(num) => num.as_f64(),
        serde_json::Value::String(raw) => raw.trim().parse::<f64>().ok(),
        _ => None,
    }
}

pub(super) fn summarize_context_window(
    metrics: &serde_json::Value,
) -> Option<ContextWindowSummary> {
    let obj = metrics.as_object()?;
    let total = obj.get("context_window_tokens").and_then(parse_u64)?;
    let mut used = obj.get("context_tokens_estimate").and_then(parse_u64);
    let mut remaining = obj.get("remaining_tokens_estimate").and_then(parse_u64);

    if used.is_none() {
        if let Some(rem) = remaining {
            used = Some(total.saturating_sub(rem));
        }
    }
    if remaining.is_none() {
        if let Some(used) = used {
            remaining = Some(total.saturating_sub(used));
        }
    }
    let used = used.unwrap_or(0);
    let remaining = remaining.unwrap_or_else(|| total.saturating_sub(used));
    let utilization = obj
        .get("remaining_fraction")
        .and_then(parse_f64)
        .map(|fraction| (1.0 - fraction).clamp(0.0, 1.0))
        .unwrap_or_else(|| {
            if total == 0 {
                0.0
            } else {
                (used as f64 / total as f64).clamp(0.0, 1.0)
            }
        });

    Some(ContextWindowSummary {
        total,
        used,
        remaining,
        utilization,
    })
}

pub(super) fn legacy_context_window_metric_key(
    metrics: &serde_json::Value,
) -> Option<&'static str> {
    let obj = metrics.as_object()?;
    if obj.contains_key("context_window") {
        return Some("context_window");
    }
    if obj.contains_key("window_tokens") {
        return Some("window_tokens");
    }
    if obj.contains_key("total_tokens") {
        return Some("total_tokens");
    }
    if obj.contains_key("used_tokens") {
        return Some("used_tokens");
    }
    if obj.contains_key("remaining_tokens") {
        return Some("remaining_tokens");
    }
    None
}

async fn context_window_for_run(
    state: &Arc<AppState>,
    session_id: SessionId,
    run_id: RunId,
) -> Option<ContextWindowSummary> {
    let store = state.store_for_session(session_id).await.ok()?;
    let turn = store
        .get_latest_turn_for_run(session_id, run_id)
        .await
        .ok()
        .flatten()?;
    let metrics = turn.metrics_json.as_ref()?;
    if let Some(legacy_key) = legacy_context_window_metric_key(metrics) {
        state
            .emit_compat_payload_reject_counter(
                "sessions.context_window_summary",
                "legacy_context_window_key",
                Some(("legacy_key", legacy_key)),
            )
            .await;
    }
    summarize_context_window(metrics)
}

async fn context_window_for_session(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Option<ContextWindowSummary> {
    let store = state.store_for_session(session_id).await.ok()?;
    let turn = store
        .get_latest_turn_for_session(session_id)
        .await
        .ok()
        .flatten()?;
    let metrics = turn.metrics_json.as_ref()?;
    if let Some(legacy_key) = legacy_context_window_metric_key(metrics) {
        state
            .emit_compat_payload_reject_counter(
                "sessions.context_window_summary",
                "legacy_context_window_key",
                Some(("legacy_key", legacy_key)),
            )
            .await;
    }
    summarize_context_window(metrics)
}

fn estimate_context_window_for_prompt_len(
    provider_id: &str,
    model_id: &str,
    prompt_len: i64,
) -> Option<ContextWindowSummary> {
    let total = crate::scheduler::model_context_window(provider_id, model_id)? as u64;
    let chars = prompt_len.max(0) as u64;
    let used = chars.div_ceil(4);
    let remaining = total.saturating_sub(used);
    let utilization = if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    };
    Some(ContextWindowSummary {
        total,
        used,
        remaining,
        utilization,
    })
}

fn estimate_context_window_for_prompt(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<ContextWindowSummary> {
    let prompt_len = prompt.chars().count() as i64;
    estimate_context_window_for_prompt_len(provider_id, model_id, prompt_len)
}

async fn worktree_path_for_child(
    state: &Arc<AppState>,
    parent_worktree_id: WorktreeId,
    child_session_id: SessionId,
) -> Option<String> {
    let store = state.store_for_session(child_session_id).await.ok()?;
    let session = store.get_session(child_session_id).await.ok().flatten()?;
    if session.worktree_id == parent_worktree_id {
        return None;
    }
    let worktree = store
        .get_worktree(session.worktree_id)
        .await
        .ok()
        .flatten()?;
    Some(worktree.root_path)
}

async fn build_subagent_result(
    state: &Arc<AppState>,
    parent_worktree_id: WorktreeId,
    child: &SubagentInvocationChild,
    status: String,
    content: Option<String>,
    context_window: Option<ContextWindowSummary>,
) -> Result<AgentInitResult, String> {
    let label = child
        .label
        .clone()
        .unwrap_or_else(|| format!("Subagent {}", child.position + 1));
    let worktree_path =
        worktree_path_for_child(state, parent_worktree_id, child.child_session_id).await;
    Ok(AgentInitResult {
        label,
        status,
        content,
        context_window,
        worktree_path,
    })
}

async fn build_subagent_result_for_session(
    state: &Arc<AppState>,
    parent_worktree_id: WorktreeId,
    session: &Session,
    label: String,
    status: String,
    content: Option<String>,
    context_window: Option<ContextWindowSummary>,
) -> Result<AgentInitResult, String> {
    let worktree_path = if session.worktree_id == parent_worktree_id {
        None
    } else {
        let store = state
            .store_for_session(session.id)
            .await
            .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
        store
            .get_worktree(session.worktree_id)
            .await
            .map_err(|e| logs::redact_sensitive(&e.to_string()))?
            .map(|worktree| worktree.root_path)
    };
    Ok(AgentInitResult {
        label,
        status,
        content,
        context_window,
        worktree_path,
    })
}

async fn run_subagent_child(
    state: &Arc<AppState>,
    child: SubagentInvocationChild,
    parent_worktree_id: WorktreeId,
) -> Result<AgentInitResult, String> {
    let run_id = child
        .run_id
        .ok_or_else(|| "subagent run_id missing".to_string())?;
    let terminal = wait_for_run_terminal_event(state, child.child_session_id, run_id).await;
    let status = match terminal {
        Ok(SessionEventType::Done) | Ok(SessionEventType::TurnFinished) => "completed",
        Ok(SessionEventType::TurnInterrupted) => "interrupted",
        Ok(SessionEventType::Error) => "failed",
        Ok(_) => "completed",
        Err(_) => "unknown",
    }
    .to_string();

    let child_updated_at = chrono::Utc::now();
    let mut updated_child = child.clone();
    updated_child.status = status.clone();
    updated_child.updated_at = child_updated_at;
    let store = state
        .store_for_session(child.child_session_id)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
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
) -> Result<Worktree, (StatusCode, Json<ApiErrorResp>)> {
    let worktree_id = WorktreeId::new();
    let wt_path = managed_worktree_path(&state.core.data_root, workspace.id, worktree_id);
    if let Some(parent) = wt_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    }
    let branch_name = format!("ctx/{}/{}", task_id.0, worktree_id.0);
    create_worktree(
        &workspace.root_path,
        &wt_path,
        base_commit_sha,
        &branch_name,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

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

    store.insert_worktree(worktree.clone()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    if let Err(e) = state
        .global_store()
        .upsert_workspace_worktree_index(worktree_id, workspace.id)
        .await
    {
        tracing::warn!(
            worktree_id = %worktree_id.0,
            "failed to update worktree index: {e:?}"
        );
    }
    if let Err(e) = worktree_bootstrap::spawn_worktree_bootstrap(
        Arc::clone(state),
        workspace.clone(),
        worktree.clone(),
    )
    .await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "worktree bootstrap failed: {e:?}");
    }
    if let Err(e) =
        attachments::sync_workspace_attachments(Arc::clone(state), workspace, false).await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "attachment sync failed: {e:?}");
    }
    if let Err(e) =
        attachments::ensure_worktree_attachment_mounts_if_materialized(state, workspace, &worktree)
            .await
    {
        tracing::warn!(worktree_id = %worktree_id.0, "attachment mounts failed: {e:?}");
    }

    Ok(worktree)
}

async fn enqueue_subagent_prompt(
    state: &Arc<AppState>,
    session: &Session,
    prompt: String,
) -> Result<(RunId, Message), (StatusCode, Json<ApiErrorResp>)> {
    let store = state.store_for_session(session.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
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
    let saved = store.insert_message(msg).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    state
        .global_store()
        .upsert_workspace_message_index(saved.id, session.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
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
    let queued = crate::scheduler::QueuedMessage {
        message: saved.clone(),
        enqueued_at: Instant::now(),
        run_id: None,
    };
    let _ = tx.send(SchedulerCommand::Enqueue(queued)).await;

    Ok((run_id, saved))
}

pub(crate) async fn mcp_agent_init(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentInitReq>,
) -> Result<Json<AgentInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if req.agents.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "agents is required".to_string(),
            }),
        ));
    }
    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let max_subagents = resolve_max_subagents_per_call(&settings);
    if req.agents.len() > max_subagents {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("max {max_subagents} subagents per call"),
            }),
        ));
    }
    if req
        .response_mode
        .as_deref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "response_mode is not supported; use subagent_wait to await".to_string(),
            }),
        ));
    }
    let worktree_selection = parse_subagent_worktree(req.worktree.as_deref())
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;
    let workspace = state
        .global_store()
        .get_workspace(parent.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "workspace not found".to_string(),
            }),
        ))?;

    let mut labels = Vec::with_capacity(req.agents.len());
    let mut seen_labels = HashSet::new();
    for (idx, agent) in req.agents.iter().enumerate() {
        let label = agent
            .label
            .as_deref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("agent {} label is required", idx + 1),
                }),
            ))?;
        if !seen_labels.insert(label.to_string()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("duplicate subagent label '{label}'"),
                }),
            ));
        }
        labels.push(label.to_string());
    }
    for label in &labels {
        if store
            .subagent_label_exists(parent.task_id, label)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("subagent label '{label}' already exists for this task"),
                }),
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
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "harness is required".to_string(),
                }),
            ));
        }
        provider_ids.insert(provider_id.to_string());
    }

    let available_providers: Vec<String> = {
        let statuses = state.providers.statuses.lock().await;
        let mut ids = statuses.keys().cloned().collect::<Vec<_>>();
        ids.sort();
        ids
    };
    let mut provider_statuses = HashMap::new();
    {
        let statuses = state.providers.statuses.lock().await;
        for provider_id in provider_ids.iter() {
            if let Some(status) = statuses.get(provider_id) {
                provider_statuses.insert(provider_id.clone(), status.clone());
            } else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!(
                            "unknown harness '{provider_id}'; available harnesses: {}",
                            available_providers.join(", ")
                        ),
                    }),
                ));
            }
        }
    }

    for (provider_id, status) in &provider_statuses {
        if !status.installed
            || !matches!(status.health, ctx_providers::adapters::ProviderHealth::Ok)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("harness '{provider_id}' is not installed or unhealthy"),
                }),
            ));
        }
    }

    let mut model_catalogs: HashMap<String, Option<ModelCatalog>> = HashMap::new();
    for provider_id in provider_ids.iter() {
        let catalog = load_provider_model_catalog(&state, &workspace, provider_id).await;
        match catalog {
            Ok(cat) => {
                model_catalogs.insert(provider_id.clone(), cat);
            }
            Err(err) => {
                return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: err })));
            }
        }
    }

    let parent_worktree = store
        .get_worktree(parent.worktree_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent worktree not found".to_string(),
            }),
        ))?;

    let worktree_plan = if worktree_selection == SubagentWorktreeSelection::New {
        let parent_root = StdPath::new(&parent_worktree.root_path);
        let vcs = vcs::driver_for_path(parent_root).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
        let base_commit_sha = vcs.rev_parse_head(parent_root).await.map_err(|e| {
            let msg = e.to_string().to_lowercase();
            if msg.contains("ambiguous argument 'head'")
                || msg.contains("unknown revision or path not in the working tree")
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: "git repo has no commits; create an initial commit before creating a worktree".to_string(),
                    }),
                );
            }
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;

        let (dirty_files, dirty_additions, dirty_deletions) =
            ctx_fs::worktrees::diff_worktree_summary(parent_root, &base_commit_sha)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
        if dirty_files > 0 || dirty_additions > 0 || dirty_deletions > 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "Your worktree has uncommitted changes. Before starting new subagents in new worktree mode, you must commit or stash your changes to be explicit about whether subagents should inherit these diffs.".to_string(),
                }),
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
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state
        .global_store()
        .upsert_workspace_subagent_invocation_index(&invocation_id, parent.workspace_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
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
        futures.push(async move {
            let store = state.store_for_session(parent.id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            let prompt = agent.prompt.trim().to_string();
            if prompt.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("agent {} prompt is required", idx + 1),
                    }),
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
                    catalog.and_then(|c| c.full_ids.first().map(|s| s.as_str()))
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
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("model is required for harness '{provider_id}'"),
                    }),
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
            .map_err(|error| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error })))?;

            let prompt_length = prompt.chars().count() as i64;
            let requested_effort = agent
                .reasoning_effort
                .as_deref()
                .map(normalize_effort_id)
                .filter(|value| !value.is_empty());
            let (_, model_effort) = split_model_id(&resolved.model_id);
            let reasoning_effort = requested_effort.or(model_effort);

            let worktree_id = match worktree_selection {
                SubagentWorktreeSelection::Inherit => parent.worktree_id,
                SubagentWorktreeSelection::New => {
                    let (vcs_kind, base_commit_sha) = worktree_plan.clone().ok_or((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: "worktree plan missing".to_string(),
                        }),
                    ))?;
                    let worktree = create_subagent_worktree(
                        &state,
                        &store,
                        &workspace,
                        parent.task_id,
                        &base_commit_sha,
                        vcs_kind,
                    )
                    .await?;
                    worktree.id
                }
            };

            let session = store
                .create_session(
                    parent.task_id,
                    parent.workspace_id,
                    worktree_id,
                    parent.execution_environment,
                    provider_id.clone(),
                    resolved.model_id.clone(),
                    "subagent".into(),
                    Some(parent.id),
                    Some("sub_agent".to_string()),
                    None,
                )
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;
            if let Err(e) = state
                .global_store()
                .upsert_workspace_session_index(session.id, parent.workspace_id)
                .await
            {
                tracing::warn!(
                    session_id = %session.id.0,
                    "failed to update subagent session index: {e:?}"
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
                model: Some(resolved.model_id),
                reasoning_effort,
                prompt_length,
                created_at: child_created_at,
                updated_at: child_created_at,
            };
            store
                .upsert_subagent_invocation_child(child.clone())
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?;

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
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        )
    })?;

    Ok(Json(AgentInitResp {
        status: "running".to_string(),
        results,
    }))
}

pub(crate) async fn mcp_agent_reply(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<AgentReplyReq>,
) -> Result<Json<AgentReplyResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let label = req.label.trim();
    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "label is required".to_string(),
            }),
        ));
    }
    let child = store
        .get_subagent_session_by_label(parent.id, label)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "subagent label not found".to_string(),
            }),
        ))?;

    let prompt = req.prompt.trim().to_string();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    if store
        .get_running_turn_for_session(child.id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            Json(ApiErrorResp {
                error: "subagent_busy: The subagent is still running. Please await it with subagent_wait or interrupt it with subagent_interrupt.".to_string(),
            }),
        ));
    }

    let mut context_window =
        estimate_context_window_for_prompt(&child.provider_id, &child.model_id, &prompt);
    if context_window.is_none() {
        context_window = context_window_for_session(&state, child.id).await;
    }

    let (_run_id, _message) = enqueue_subagent_prompt(&state, &child, prompt).await?;
    let worktree_path = worktree_path_for_child(&state, parent.worktree_id, child.id).await;

    Ok(Json(AgentReplyResp {
        label: label.to_string(),
        status: "running".to_string(),
        context_window,
        worktree_path,
    }))
}

pub(crate) async fn mcp_subagent_list(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SubagentListItem>>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let subs = store.list_subagent_sessions(parent.id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

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

    Ok(Json(results))
}

pub(crate) async fn mcp_subagent_interrupt(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentInterruptReq>,
) -> Result<Json<SubagentInterruptResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let use_all = req.all.unwrap_or(false);
    let labels = match (use_all, req.label) {
        (true, Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "provide either label or all".to_string(),
                }),
            ));
        }
        (true, None) => {
            let subs = store.list_subagent_sessions(parent.id).await.map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;
            subs.into_iter()
                .map(|sub| sub.title.trim().to_string())
                .collect::<Vec<_>>()
        }
        (false, Some(label)) => vec![label],
        (false, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label or all is required".to_string(),
                }),
            ));
        }
    };

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label cannot be empty".to_string(),
                }),
            ));
        }
        let child = store
            .get_subagent_session_by_label(parent.id, trimmed)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: format!("subagent label '{trimmed}' not found"),
                }),
            ))?;

        let tx = state.ensure_scheduler(child.clone()).await;
        let _ = tx.send(SchedulerCommand::Interrupt).await;

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
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp { error }),
                )
            })?,
        );
    }

    Ok(Json(SubagentInterruptResp {
        status: "interrupt_requested".to_string(),
        results,
    }))
}

pub(crate) async fn mcp_oracle(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<oracle::OracleRequest>,
) -> Result<Json<oracle::OracleResponse>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let workspace_id = state
        .global_store()
        .get_workspace_id_for_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let store = state.store_for_workspace(workspace_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "prompt is required".to_string(),
            }),
        ));
    }

    let settings = user_settings::load_settings(state.global_store())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let cfg = settings.oracle.as_ref().ok_or((
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: "oracle is not configured".to_string(),
        }),
    ))?;

    let resp = oracle::oracle_one_shot(cfg, req).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;

    Ok(Json(resp))
}

pub(crate) async fn mcp_subagent_wait(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SubagentWaitReq>,
) -> Result<Json<SubagentWaitResp>, (StatusCode, Json<ApiErrorResp>)> {
    let parent_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    let store = state.store_for_session(parent_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let parent = store
        .get_session(parent_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "parent session not found".to_string(),
            }),
        ))?;

    let mut labels = match (req.label, req.labels) {
        (Some(label), None) => vec![label],
        (None, Some(labels)) => labels,
        (Some(_), Some(_)) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "provide either label or labels".to_string(),
                }),
            ));
        }
        (None, None) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label or labels is required".to_string(),
                }),
            ));
        }
    };
    if labels.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "labels is required".to_string(),
            }),
        ));
    }
    let mut seen = HashSet::new();
    for label in labels.iter_mut() {
        let trimmed = label.trim().to_string();
        if trimmed.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "label cannot be empty".to_string(),
                }),
            ));
        }
        if !seen.insert(trimmed.clone()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("duplicate label '{trimmed}'"),
                }),
            ));
        }
        *label = trimmed;
    }

    let mut results = Vec::with_capacity(labels.len());
    for label in labels {
        let child = store
            .get_subagent_session_by_label(parent.id, &label)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?
            .ok_or((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: format!("subagent label '{label}' not found"),
                }),
            ))?;

        let running_turn = store
            .get_running_turn_for_session(child.id)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp {
                        error: logs::redact_sensitive(&e.to_string()),
                    }),
                )
            })?;

        let (status, run_id) = if let Some(turn) = running_turn {
            let run_id = turn.run_id.ok_or((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "subagent run_id missing; cannot wait".to_string(),
                }),
            ))?;
            let terminal = wait_for_run_terminal_event(&state, child.id, run_id).await;
            let status = match terminal {
                Ok(SessionEventType::Done) | Ok(SessionEventType::TurnFinished) => "completed",
                Ok(SessionEventType::TurnInterrupted) => "interrupted",
                Ok(SessionEventType::Error) => "failed",
                Ok(_) => "completed",
                Err(_) => "unknown",
            }
            .to_string();
            (status, Some(run_id))
        } else if let Some(turn) =
            store
                .get_latest_turn_for_session(child.id)
                .await
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ApiErrorResp {
                            error: logs::redact_sensitive(&e.to_string()),
                        }),
                    )
                })?
        {
            let status = match turn.status {
                SessionTurnStatus::Completed => "completed",
                SessionTurnStatus::Interrupted => "interrupted",
                SessionTurnStatus::Failed => "failed",
                SessionTurnStatus::Running | SessionTurnStatus::Queued => "running",
            }
            .to_string();
            (status, turn.run_id)
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("subagent '{label}' has no runs to wait for"),
                }),
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
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResp { error }),
                )
            })?,
        );
    }

    let status = aggregate_subagent_status(&results);

    Ok(Json(SubagentWaitResp {
        status: status.to_string(),
        results,
    }))
}

pub(super) fn aggregate_subagent_status(results: &[AgentInitResult]) -> &'static str {
    if results.iter().any(|r| r.status == "failed") {
        "failed"
    } else if results.iter().any(|r| r.status == "interrupted") {
        "interrupted"
    } else if results.iter().any(|r| r.status == "running") {
        "running"
    } else if results.iter().any(|r| r.status == "unknown") {
        "unknown"
    } else {
        "completed"
    }
}
