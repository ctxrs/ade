use std::sync::Arc;
use std::time::Instant;

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
use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::AppState;
use ctx_core::ids::*;
use ctx_core::models::*;
use ctx_managed_installs as installer;
use ctx_observability::logs;
use ctx_providers::{
    ask_user_question::{AskUserQuestionAnswer, AskUserQuestionOutcome},
    crp::probe_crp_models,
};
#[cfg(test)]
use ctx_settings_model as user_settings;
use ctx_store::is_unique_constraint_violation;
use ctx_workspace_services::file_completions as workspace_file_completions;
use ctx_workspace_services::worktree_vcs::GitStatusEntry;

mod subagents;
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
mod store_lookup;
#[cfg(test)]
pub(in crate::api::sessions) use store_lookup::store_for_existing_session_api_error_allow_archived;
pub(in crate::api::sessions) use store_lookup::{
    store_for_existing_session_api_error, store_for_existing_session_api_error_for_write,
    store_for_existing_session_status, store_for_existing_session_status_allow_archived,
    store_for_existing_session_status_for_write,
};
mod titles_and_modes;
#[cfg(test)]
use ctx_session_service::title_generation;
pub(super) use titles_and_modes::{generate_session_title, set_session_mode, set_session_model};

#[cfg(test)]
mod tests;
