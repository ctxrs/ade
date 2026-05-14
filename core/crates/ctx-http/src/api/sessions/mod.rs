#[cfg(test)]
use std::sync::Arc;
use std::time::Instant;

use base64::Engine;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::errors::ApiErrorResp;
use super::shared::{map_file_completions_error, FileCompletionsQuery};
use crate::daemon::SessionsHandle;
use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_observability::logs;
#[cfg(test)]
use ctx_settings_model as user_settings;
use ctx_workspace_services::worktree_vcs::GitStatusEntry;

mod subagents;
pub(super) use subagents::{
    get_session_subagent_invocation, list_session_subagent_invocations, list_session_subagents,
    mcp_archive_agent, mcp_get_agent, mcp_interrupt_agent, mcp_list_agents, mcp_send_input,
    mcp_spawn_agent, mcp_wait_agent,
};
mod control;
pub(super) use control::{
    authenticate_session, cancel_session, interrupt_session, submit_ask_user_question,
};
mod file_completions;
pub(super) use file_completions::session_file_completions;
mod messages;
pub(super) use messages::{delete_session_message, post_message};
mod snapshot;
pub(super) use snapshot::{
    apply_session_diff_patch, get_session_diff, get_session_diff_summary, get_session_events,
    get_session_git_status, get_session_head, get_session_history, get_session_snapshot,
    get_session_state, list_session_turn_tools,
};
mod titles_and_modes;
#[cfg(test)]
use ctx_session_service::title_generation;
pub(super) use titles_and_modes::{generate_session_title, set_session_mode, set_session_model};

#[cfg(test)]
mod tests;

fn session_data_or_status<T>(result: anyhow::Result<Option<T>>) -> Result<T, StatusCode> {
    result
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)
}
