mod child_runs;
mod errors;
mod init;
mod providers;
mod request;
mod worktrees;

use std::collections::HashSet;
use std::sync::Arc;

use axum::http::StatusCode;

use crate::api::sessions::{
    build_subagent_result, build_subagent_result_for_session, context_window_for_run,
    context_window_for_session, estimate_context_window_for_prompt,
    estimate_context_window_for_prompt_len, resolve_model_id, worktree_path_for_child,
    AgentInitReq, AgentInitResp, AgentReplyReq, AgentReplyResp, SubagentInterruptReq,
    SubagentInterruptResp, SubagentListItem, SubagentWaitReq, SubagentWaitResp,
};
use crate::daemon::AppState;
use crate::scheduler::{InterruptTelemetryContext, SchedulerCommand};
use crate::settings as user_settings;
use ctx_core::ids::SessionId;
use ctx_core::models::{
    SessionStatus, SessionTurnStatus, SubagentInvocation, SubagentInvocationChild,
};

use self::child_runs::{
    emit_subagent_invocation_notice, enqueue_subagent_prompt, finalize_subagent_invocation,
    run_subagent_child, subagent_status_from_turn_status, wait_for_run_terminal_turn,
};
use self::errors::{api_error, internal_api_error, load_parent_session, ApiResult};
use self::providers::load_requested_model_catalogs;
use self::request::{
    build_subagent_request_json, collect_provider_ids, default_catalog_model_id,
    parse_subagent_worktree, resolve_max_subagents_per_call, validate_requested_labels,
};
use self::worktrees::{create_subagent_worktree, plan_subagent_worktree_creation};
pub(crate) use init::init_subagents;

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
                .map(|message| message.content),
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
