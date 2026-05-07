mod agent_control;
mod child_runs;
mod errors;
mod init;
mod providers;
mod request;
mod worktrees;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use ctx_session_service::subagents::{
    build_subagent_request_json, collect_provider_ids, normalize_wait_agent_ids,
    parse_subagent_worktree, parse_wait_mode, parse_wait_until, resolve_max_subagents_per_call,
    wait_predicate_satisfied, AgentWaitDetail, AgentWaitUntil, SubagentRequestAgent,
    SubagentWorktreeSelection, DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT, DEFAULT_MAX_SUBAGENT_DEPTH,
};
use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;
use ctx_session_tools::model_resolution::resolve_model_id;

use crate::api::sessions::{
    context_window_for_run, worktree_path_for_child, AgentDetail, AgentInitReq, AgentResult,
    AgentSummary, ArchiveAgentReq, ArchiveAgentResp, GetAgentReq, GetAgentResp, InterruptAgentReq,
    InterruptAgentResp, SendInputReq, SendInputResp, SpawnAgentReq, SpawnAgentResp, WaitAgentReq,
    WaitAgentResp,
};
use crate::daemon::AppState;
use crate::scheduler::SchedulerCommand;
use crate::settings as user_settings;
use ctx_core::ids::{RunId, SessionId};
use ctx_core::models::{
    MessageDelivery, SessionTurnStatus, SubagentInvocation, SubagentInvocationChild,
};

pub(crate) use self::agent_control::{
    archive_agent, get_agent, interrupt_agent, list_agents, send_input, spawn_agent, wait_agent,
};
use self::child_runs::{
    dispatch_subagent_prompt, emit_subagent_invocation_notice, finalize_subagent_invocation,
    persist_subagent_prompt, run_subagent_child, wait_for_run_assistant_message,
    PersistedSubagentPrompt,
};
use self::errors::{api_error, internal_api_error, load_parent_session, ApiResult};
pub(crate) use self::errors::{SubagentError, SubagentErrorKind};
use self::providers::load_requested_model_catalogs;
use self::request::{default_catalog_model_id, validate_requested_labels};
use self::worktrees::{
    cleanup_archived_subagent_worktree, create_subagent_worktree, plan_subagent_worktree_creation,
};

pub(crate) use init::init_subagents;

#[derive(Clone)]
pub(crate) struct SpawnedChild {
    child: SubagentInvocationChild,
    worktree_path: Option<String>,
    last_event_seq: i64,
}

fn encode_agent_ref(session_id: SessionId) -> String {
    format!(
        "agent_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session_id.0.as_bytes())
    )
}

fn decode_agent_ref(raw: &str) -> Result<SessionId, String> {
    let encoded = raw
        .trim()
        .strip_prefix("agent_")
        .ok_or_else(|| "invalid agent_id".to_string())?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "invalid agent_id".to_string())?;
    let uuid = uuid::Uuid::from_slice(&bytes).map_err(|_| "invalid agent_id".to_string())?;
    Ok(SessionId(uuid))
}

fn encode_run_ref(run_id: RunId) -> String {
    format!(
        "run_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(run_id.0.as_bytes())
    )
}

fn agent_terminal_result_status(status: SessionTurnStatus) -> Option<&'static str> {
    match status {
        SessionTurnStatus::Completed => Some("completed"),
        SessionTurnStatus::Interrupted => Some("interrupted"),
        SessionTurnStatus::Failed => Some("failed"),
        SessionTurnStatus::Queued | SessionTurnStatus::Starting | SessionTurnStatus::Running => {
            None
        }
    }
}

fn agent_delivery_label(delivery: &MessageDelivery) -> &'static str {
    match delivery {
        MessageDelivery::Immediate => "immediate",
        MessageDelivery::Queued => "queued",
    }
}

fn is_active_turn_status(status: &SessionTurnStatus) -> bool {
    matches!(
        status,
        SessionTurnStatus::Queued | SessionTurnStatus::Starting | SessionTurnStatus::Running
    )
}

fn is_terminal_turn_status(status: &SessionTurnStatus) -> bool {
    matches!(
        status,
        SessionTurnStatus::Completed | SessionTurnStatus::Interrupted | SessionTurnStatus::Failed
    )
}

fn agent_active_state(status: SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Queued => "queued",
        SessionTurnStatus::Starting => "starting",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed
        | SessionTurnStatus::Interrupted
        | SessionTurnStatus::Failed => "waiting_input",
    }
}

fn agent_health(
    active_turn: Option<&ctx_core::models::SessionTurn>,
    inactivity_timeout: Duration,
) -> &'static str {
    let Some(turn) = active_turn else {
        return "healthy";
    };

    let stalled_after = inactivity_timeout.max(Duration::from_millis(1));
    let slow_after = stalled_after.checked_div(2).unwrap_or(stalled_after);
    let age = chrono::Utc::now()
        .signed_duration_since(turn.updated_at)
        .to_std()
        .unwrap_or_default();
    if age >= stalled_after {
        "stalled"
    } else if !slow_after.is_zero() && age >= slow_after {
        "slow"
    } else {
        "healthy"
    }
}

async fn latest_terminal_turn_for_session(
    store: &ctx_store::Store,
    session_id: SessionId,
    latest_turn: Option<&ctx_core::models::SessionTurn>,
) -> ApiResult<Option<ctx_core::models::SessionTurn>> {
    if latest_turn
        .as_ref()
        .is_some_and(|turn| is_terminal_turn_status(&turn.status))
    {
        return Ok(latest_turn.cloned());
    }

    let mut before_seq = latest_turn.as_ref().and_then(|turn| turn.start_seq);
    loop {
        let page = store
            .list_session_turns_page_by_seq(session_id, before_seq, Some(50))
            .await
            .map_err(internal_api_error)?;
        if page.is_empty() {
            return Ok(None);
        }
        if let Some(turn) = page
            .iter()
            .rev()
            .find(|turn| is_terminal_turn_status(&turn.status))
            .cloned()
        {
            return Ok(Some(turn));
        }
        before_seq = page.first().and_then(|turn| turn.start_seq);
        if before_seq.is_none() {
            return Ok(None);
        }
    }
}

async fn resolve_child_agent_session(
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    raw_agent_id: &str,
) -> ApiResult<ctx_core::models::Session> {
    let agent_id = decode_agent_ref(raw_agent_id)
        .map_err(|error| api_error(SubagentErrorKind::BadRequest, error))?;
    let child = store
        .get_active_subagent_session(parent.id, agent_id)
        .await
        .map_err(internal_api_error)?
        .ok_or_else(|| api_error(SubagentErrorKind::NotFound, "agent not found"))?;
    Ok(child)
}

async fn build_agent_summary(
    store: &ctx_store::Store,
    session_id: SessionId,
    session_title: &str,
    inactivity_timeout: Duration,
) -> ApiResult<(AgentSummary, Option<ctx_core::models::SessionTurn>)> {
    let latest_turn = store
        .get_latest_turn_for_session(session_id)
        .await
        .map_err(internal_api_error)?;
    let latest_terminal_turn =
        latest_terminal_turn_for_session(store, session_id, latest_turn.as_ref()).await?;
    let active_turn = match store.get_running_turn_for_session(session_id).await {
        Ok(Some(turn)) => Some(turn),
        Ok(None) => latest_turn
            .clone()
            .filter(|turn| is_active_turn_status(&turn.status)),
        Err(error) => return Err(internal_api_error(error)),
    };
    let task_label = {
        let trimmed = session_title.trim();
        if trimmed.is_empty() {
            format!("agent-{}", session_id.0)
        } else {
            trimmed.to_string()
        }
    };
    let summary = AgentSummary {
        agent_id: encode_agent_ref(session_id),
        task_label,
        state: active_turn
            .as_ref()
            .map(|turn| agent_active_state(turn.status.clone()).to_string())
            .unwrap_or_else(|| "waiting_input".to_string()),
        health: agent_health(active_turn.as_ref(), inactivity_timeout).to_string(),
        current_run_id: active_turn
            .as_ref()
            .and_then(|turn| turn.run_id.map(encode_run_ref)),
        latest_result_status: latest_terminal_turn
            .as_ref()
            .and_then(|turn| agent_terminal_result_status(turn.status.clone()))
            .map(str::to_string),
        last_progress_at: active_turn
            .as_ref()
            .or(latest_turn.as_ref())
            .map(|turn| turn.updated_at.to_rfc3339()),
        last_event_seq: store
            .get_session_last_event_seq(session_id)
            .await
            .map_err(internal_api_error)?,
    };
    Ok((summary, latest_terminal_turn))
}

async fn build_agent_detail(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    session: &ctx_core::models::Session,
    inactivity_timeout: Duration,
) -> ApiResult<AgentDetail> {
    let (summary, latest_turn) =
        build_agent_summary(store, session.id, &session.title, inactivity_timeout).await?;
    let latest_result = if let Some(turn) = latest_turn.as_ref() {
        if let Some(status) = agent_terminal_result_status(turn.status.clone()) {
            let content = if let Some(run_id) = turn.run_id {
                if status == "completed" {
                    wait_for_run_assistant_message(state, session.id, run_id)
                        .await
                        .map_err(internal_api_error)?
                } else {
                    store
                        .get_last_assistant_message_for_run(session.id, run_id)
                        .await
                        .map_err(internal_api_error)?
                        .map(|message| message.content)
                }
            } else {
                None
            };
            Some(AgentResult {
                run_id: turn.run_id.map(encode_run_ref),
                status: status.to_string(),
                content,
                context_window: if let Some(run_id) = turn.run_id {
                    context_window_for_run(state, session.id, run_id).await
                } else {
                    None
                },
            })
        } else {
            None
        }
    } else {
        None
    };

    Ok(AgentDetail {
        agent: summary,
        latest_result,
        worktree_path: worktree_path_for_child(state, parent.worktree_id, session.id).await,
    })
}

async fn build_enqueued_agent_detail(
    state: &Arc<AppState>,
    parent: &ctx_core::models::Session,
    session: &ctx_core::models::Session,
    persisted: &PersistedSubagentPrompt,
) -> AgentDetail {
    let task_label = {
        let trimmed = session.title.trim();
        if trimmed.is_empty() {
            format!("agent-{}", session.id.0)
        } else {
            trimmed.to_string()
        }
    };

    AgentDetail {
        agent: AgentSummary {
            agent_id: encode_agent_ref(session.id),
            task_label,
            state: agent_active_state(match persisted.saved_message.delivery {
                MessageDelivery::Queued => SessionTurnStatus::Queued,
                MessageDelivery::Immediate => SessionTurnStatus::Starting,
            })
            .to_string(),
            health: "healthy".to_string(),
            current_run_id: Some(encode_run_ref(persisted.run_id)),
            latest_result_status: None,
            last_progress_at: Some(persisted.saved_message.created_at.to_rfc3339()),
            last_event_seq: persisted.last_event_seq,
        },
        latest_result: None,
        worktree_path: worktree_path_for_child(state, parent.worktree_id, session.id).await,
    }
}

fn build_spawned_agent_detail(spawned: &SpawnedChild) -> AgentDetail {
    let child = &spawned.child;
    AgentDetail {
        agent: AgentSummary {
            agent_id: encode_agent_ref(child.child_session_id),
            task_label: child
                .label
                .clone()
                .unwrap_or_else(|| format!("Subagent {}", child.position + 1)),
            state: "running".to_string(),
            health: "healthy".to_string(),
            current_run_id: child.run_id.map(encode_run_ref),
            latest_result_status: None,
            last_progress_at: Some(child.updated_at.to_rfc3339()),
            last_event_seq: spawned.last_event_seq,
        },
        latest_result: None,
        worktree_path: spawned.worktree_path.clone(),
    }
}

async fn collect_wait_targets(
    store: &ctx_store::Store,
    parent: &ctx_core::models::Session,
    agent_ids: &[String],
) -> ApiResult<Vec<ctx_core::models::Session>> {
    let mut agents = Vec::with_capacity(agent_ids.len());
    for agent_id in agent_ids {
        agents.push(resolve_child_agent_session(store, parent, agent_id).await?);
    }
    Ok(agents)
}
