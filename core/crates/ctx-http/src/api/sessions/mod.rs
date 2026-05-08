use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use base64::Engine;
use sha2::Digest;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::artifacts::persist_blob_bytes;
use super::errors::ApiErrorResp;
use super::redact_json_value;
use super::shared::{load_and_cache_worktree_files, FileCompletionsQuery};
use crate::daemon::execution_effective;
use crate::daemon::git_status::GitStatusEntry;
use crate::daemon::installer;
use crate::daemon::AppState;
use crate::scheduler::SchedulerCommand;
use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_harness_runtime::sandbox_container_command;
use ctx_observability::logs;
use ctx_providers::{
    ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome},
    crp::probe_crp_models,
};
use ctx_sandbox_container_runtime::command_output_with_timeout;
#[cfg(test)]
use ctx_settings_model as user_settings;
use ctx_store::is_unique_constraint_violation;
use ctx_workspace_container::workspace_container_name;
use ctx_workspace_services::file_completions as workspace_file_completions;

mod subagents;
pub(crate) use subagents::{
    context_window_for_run, worktree_path_for_child, AgentDetail, AgentInitItem, AgentInitReq,
    AgentResult, AgentSummary, ArchiveAgentReq, ArchiveAgentResp, GetAgentReq, GetAgentResp,
    InterruptAgentReq, InterruptAgentResp, SendInputReq, SendInputResp, SpawnAgentReq,
    SpawnAgentResp, WaitAgentReq, WaitAgentResp,
};
pub(super) use subagents::{
    get_session_subagent_invocation, list_session_subagent_invocations, list_session_subagents,
    mcp_archive_agent, mcp_get_agent, mcp_interrupt_agent, mcp_list_agents, mcp_send_input,
    mcp_spawn_agent, mcp_wait_agent,
};
mod diff_exec;
pub(crate) use diff_exec::diff_worktree_summary_for_session;
mod control;
pub(super) use control::{
    authenticate_session, cancel_session, interrupt_session, submit_ask_user_question,
};
mod file_completions;
pub(super) use file_completions::session_file_completions;
mod messages;
pub(crate) use messages::ensure_session_turn_for_message;
pub(super) use messages::{delete_session_message, post_message};
mod models;
pub(crate) use models::load_provider_model_catalog_for_execution_environment;
mod snapshot;
pub(super) use snapshot::{
    apply_session_diff_patch, get_session_diff, get_session_diff_summary, get_session_events,
    get_session_git_status, get_session_head, get_session_history, get_session_snapshot,
    get_session_state, list_session_turn_tools,
};
mod titles_and_modes;
#[cfg(test)]
use ctx_session_service::title_generation;
pub(super) use titles_and_modes::{
    generate_session_title, schedule_session_title_generation, set_session_mode, set_session_model,
};
#[cfg(test)]
use titles_and_modes::{generate_title_for_prompt, TitleGenerationSource};

async fn store_for_existing_session_status_allow_archived(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = match state.lookup_session_store(session_id).await {
        crate::daemon::StoreLookup::Found(store) => store,
        crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => {
            return Err(StatusCode::NOT_FOUND);
        }
        crate::daemon::StoreLookup::Unavailable(_) => {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    Ok(store)
}

async fn store_for_existing_session_status(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = store_for_existing_session_status_allow_archived(state, session_id).await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(store)
}

const STORE_OPEN_RETRY_LIMIT: usize = 3;
const STORE_OPEN_RETRY_BASE_MS: u64 = 40;

fn is_transient_store_open_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("database is locked")
        || msg.contains("sqlite_busy")
        || msg.contains("database is busy")
}

async fn store_for_existing_session_status_with_retry(
    state: &Arc<AppState>,
    session_id: SessionId,
    retry_limit: usize,
    retry_base_ms: u64,
) -> Result<ctx_store::Store, StatusCode> {
    let mut attempt = 0usize;
    loop {
        match state.lookup_session_store(session_id).await {
            crate::daemon::StoreLookup::Found(store) => return Ok(store),
            crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => {
                return Err(StatusCode::NOT_FOUND);
            }
            crate::daemon::StoreLookup::Unavailable(err) => {
                if is_transient_store_open_error(&err) && attempt < retry_limit {
                    attempt += 1;
                    let backoff_ms = retry_base_ms.saturating_mul(attempt as u64);
                    tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                    continue;
                }
                tracing::warn!(session_id = %session_id.0, "session store lookup failed: {err:#}");
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
}

async fn store_for_existing_session_status_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, StatusCode> {
    let store = store_for_existing_session_status_with_retry(
        state,
        session_id,
        STORE_OPEN_RETRY_LIMIT,
        STORE_OPEN_RETRY_BASE_MS,
    )
    .await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(store)
}

async fn store_for_existing_session_api_error_allow_archived(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = match state.lookup_session_store(session_id).await {
        crate::daemon::StoreLookup::Found(store) => store,
        crate::daemon::StoreLookup::Missing | crate::daemon::StoreLookup::Deleting => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            ));
        }
        crate::daemon::StoreLookup::Unavailable(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&err.to_string()),
                }),
            ));
        }
    };
    Ok(store)
}

async fn store_for_existing_session_api_error(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = store_for_existing_session_api_error_allow_archived(state, session_id).await?;
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "workspace store unavailable".to_string(),
                }),
            )
        })?
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }
    Ok(store)
}

async fn store_for_existing_session_api_error_for_write(
    state: &Arc<AppState>,
    session_id: SessionId,
) -> Result<ctx_store::Store, (StatusCode, Json<ApiErrorResp>)> {
    let store = match store_for_existing_session_status_with_retry(
        state,
        session_id,
        STORE_OPEN_RETRY_LIMIT,
        STORE_OPEN_RETRY_BASE_MS,
    )
    .await
    {
        Ok(store) => store,
        Err(StatusCode::NOT_FOUND) => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(ApiErrorResp {
                    error: "session not found".to_string(),
                }),
            ));
        }
        Err(StatusCode::INTERNAL_SERVER_ERROR) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "workspace store unavailable".to_string(),
                }),
            ));
        }
        Err(status) => {
            return Err((
                status,
                Json(ApiErrorResp {
                    error: status.to_string(),
                }),
            ));
        }
    };
    if store
        .is_archived_subagent_session(session_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: "workspace store unavailable".to_string(),
                }),
            )
        })?
    {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ));
    }
    Ok(store)
}

#[cfg(test)]
mod tests;
