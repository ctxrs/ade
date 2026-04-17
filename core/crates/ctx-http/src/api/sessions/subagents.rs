use super::*;

mod handlers;
mod init;

pub(crate) use handlers::*;
pub(crate) use init::*;

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

#[derive(Debug, Serialize)]
pub(crate) struct AgentInitResp {
    pub(crate) status: String,
    pub(crate) results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentInitResult {
    pub(crate) label: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ContextWindowSummary {
    pub(crate) total: u64,
    pub(crate) used: u64,
    pub(crate) remaining: u64,
    pub(crate) utilization: f64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubagentWaitReq {
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) labels: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubagentWaitResp {
    pub(crate) status: String,
    pub(crate) results: Vec<AgentInitResult>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubagentInterruptReq {
    #[serde(default)]
    pub(crate) label: Option<String>,
    #[serde(default)]
    pub(crate) all: Option<bool>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubagentInterruptResp {
    pub(crate) status: String,
    pub(crate) results: Vec<AgentInitResult>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubagentListItem {
    pub(crate) label: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AgentReplyReq {
    pub(crate) label: String,
    pub(crate) prompt: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentReplyResp {
    pub(crate) label: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) context_window: Option<ContextWindowSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) worktree_path: Option<String>,
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

pub(crate) fn summarize_context_window(
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

pub(crate) fn legacy_context_window_metric_key(
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

pub(crate) async fn context_window_for_session(
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

pub(crate) fn estimate_context_window_for_prompt_len(
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

pub(crate) fn estimate_context_window_for_prompt(
    provider_id: &str,
    model_id: &str,
    prompt: &str,
) -> Option<ContextWindowSummary> {
    let prompt_len = prompt.chars().count() as i64;
    estimate_context_window_for_prompt_len(provider_id, model_id, prompt_len)
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

pub(crate) async fn build_subagent_result(
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

pub(crate) async fn build_subagent_result_for_session(
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

pub(crate) fn aggregate_subagent_status(results: &[AgentInitResult]) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn result_with_status(status: &str) -> AgentInitResult {
        AgentInitResult {
            label: "agent".to_string(),
            status: status.to_string(),
            content: None,
            context_window: None,
            worktree_path: None,
        }
    }

    #[test]
    fn aggregate_subagent_status_reports_unknown() {
        let results = vec![
            result_with_status("completed"),
            result_with_status("unknown"),
        ];
        assert_eq!(aggregate_subagent_status(&results), "unknown");
    }

    #[test]
    fn aggregate_subagent_status_prefers_running_over_unknown() {
        let results = vec![result_with_status("running"), result_with_status("unknown")];
        assert_eq!(aggregate_subagent_status(&results), "running");
    }

    #[test]
    fn summarize_context_window_accepts_canonical_metrics() {
        let metrics = serde_json::json!({
            "context_tokens_estimate": 40,
            "context_window_tokens": 100,
            "remaining_tokens_estimate": 60,
            "remaining_fraction": 0.6,
        });

        let summary =
            summarize_context_window(&metrics).expect("expected canonical metrics to parse");
        assert_eq!(summary.total, 100);
        assert_eq!(summary.used, 40);
        assert_eq!(summary.remaining, 60);
        assert!((summary.utilization - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn summarize_context_window_rejects_legacy_alias_metrics() {
        let legacy = serde_json::json!({
            "context_window": 100,
            "total_tokens": 40,
            "remaining_tokens": 60,
        });
        assert!(summarize_context_window(&legacy).is_none());
    }

    #[test]
    fn legacy_context_window_metric_key_detects_first_legacy_key() {
        let legacy = serde_json::json!({
            "context_window": 100,
            "remaining_tokens": 60,
        });
        assert_eq!(
            legacy_context_window_metric_key(&legacy),
            Some("context_window")
        );
    }

    #[test]
    fn legacy_context_window_metric_key_returns_none_for_canonical_shape() {
        let canonical = serde_json::json!({
            "context_tokens_estimate": 40,
            "context_window_tokens": 100,
            "remaining_tokens_estimate": 60,
        });
        assert_eq!(legacy_context_window_metric_key(&canonical), None);
    }
}
