use super::*;
use crate::daemon::sessions::subagents::{SubagentError, SubagentErrorKind};
use ctx_session_service::subagents::{
    legacy_context_window_metric_key, summarize_context_window as summarize_context_window_policy,
    SubagentContextWindowSummary,
};

mod handlers;
mod init;

pub(crate) use handlers::*;
pub(crate) use init::*;

fn subagent_error_response(error: SubagentError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        SubagentErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        SubagentErrorKind::NotFound => StatusCode::NOT_FOUND,
        SubagentErrorKind::Forbidden => StatusCode::FORBIDDEN,
        SubagentErrorKind::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
        SubagentErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}

pub(crate) async fn list_session_subagents(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<SessionSummary>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
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
    let store = store_for_existing_session_status(&state, session_id).await?;
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

pub(crate) async fn get_session_subagent_invocation(
    State(state): State<Arc<AppState>>,
    Path((session_id, id)): Path<(String, String)>,
) -> Result<Json<SubagentInvocation>, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = store_for_existing_session_status(&state, session_id).await?;
    let invocation = store
        .get_subagent_invocation(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if invocation.parent_session_id != session_id {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(Json(invocation))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentInitReq {
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
    #[serde(default)]
    pub(crate) response_mode: Option<String>,
    #[serde(default)]
    pub(crate) worktree: Option<String>,
    pub(crate) agents: Vec<AgentInitItem>,
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct AgentInitItem {
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) harness: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct ContextWindowSummary {
    pub(crate) total: u64,
    pub(crate) used: u64,
    pub(crate) remaining: u64,
    pub(crate) utilization: f64,
}

impl From<SubagentContextWindowSummary> for ContextWindowSummary {
    fn from(summary: SubagentContextWindowSummary) -> Self {
        Self {
            total: summary.total,
            used: summary.used,
            remaining: summary.remaining,
            utilization: summary.utilization,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct SpawnAgentReq {
    #[serde(default)]
    pub(crate) tool_call_id: Option<String>,
    #[serde(default)]
    pub(crate) worktree: Option<String>,
    pub(crate) task_label: String,
    pub(crate) prompt: String,
    #[serde(default)]
    pub(crate) harness: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentSummary {
    pub(crate) agent_id: String,
    pub(crate) task_label: String,
    pub(crate) state: String,
    pub(crate) health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_result_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_progress_at: Option<String>,
    pub(crate) last_event_seq: i64,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<String>,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<ContextWindowSummary>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct AgentDetail {
    pub(crate) agent: AgentSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_result: Option<AgentResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SpawnAgentResp {
    pub(crate) agent: AgentDetail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GetAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct GetAgentResp {
    pub(crate) agent: AgentDetail,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SendInputReq {
    pub(crate) agent_id: String,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) interrupt: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SendInputResp {
    pub(crate) agent: AgentDetail,
    pub(crate) queued_run_id: String,
    pub(crate) delivery: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ArchiveAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ArchiveAgentResp {
    pub(crate) agent_id: String,
    pub(crate) task_label: String,
    pub(crate) archived: bool,
    pub(crate) cleanup_failed: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct WaitAgentReq {
    #[serde(default)]
    pub(crate) agent_id: Option<String>,
    #[serde(default)]
    pub(crate) agent_ids: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default)]
    pub(crate) until: Option<String>,
    #[serde(default)]
    pub(crate) since_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct WaitAgentResp {
    pub(crate) wait_status: String,
    pub(crate) mode: String,
    pub(crate) until: String,
    pub(crate) results: Vec<AgentDetail>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InterruptAgentReq {
    pub(crate) agent_id: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct InterruptAgentResp {
    pub(crate) agent: AgentDetail,
}

pub(crate) fn summarize_context_window(
    metrics: &serde_json::Value,
) -> Option<ContextWindowSummary> {
    summarize_context_window_policy(metrics).map(Into::into)
}

pub(crate) async fn context_window_for_run(
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

pub(crate) async fn worktree_path_for_child(
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
