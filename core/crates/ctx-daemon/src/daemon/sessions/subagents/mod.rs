mod agent_control;
mod child_runs;
mod context;
mod details;
mod errors;
mod init;
mod providers;
mod request;
mod types;
mod worktrees;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ctx_session_tools::interrupt_telemetry::InterruptTelemetryContext;
use ctx_subagent_service::{
    collect_provider_ids, normalize_wait_agent_ids, parse_wait_mode, parse_wait_until,
    wait_predicate_satisfied, AgentWaitDetail, AgentWaitUntil,
    DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT, DEFAULT_MAX_SUBAGENT_DEPTH,
};

pub use self::types::{
    AgentDetail, AgentInitItem, AgentInitReq, AgentResult, AgentSummary, ArchiveAgentReq,
    ArchiveAgentResp, ContextWindowSummary, GetAgentReq, GetAgentResp, InterruptAgentReq,
    InterruptAgentResp, SendInputReq, SendInputResp, SpawnAgentReq, SpawnAgentResp, WaitAgentReq,
    WaitAgentResp,
};
use crate::daemon::scheduler::SchedulerCommand;
use crate::daemon::DaemonState;
use ctx_core::ids::SessionId;
use ctx_core::models::SubagentInvocationChild;

pub use self::agent_control::{
    archive_agent, get_agent, interrupt_agent, list_agents, send_input, spawn_agent, wait_agent,
};
use self::child_runs::{
    dispatch_subagent_prompt, emit_subagent_invocation_notice, finalize_subagent_invocation,
    persist_subagent_prompt, run_subagent_child, wait_for_run_assistant_message,
    PersistedSubagentPrompt,
};
use self::details::{
    agent_delivery_label, build_agent_detail, build_agent_summary, build_enqueued_agent_detail,
    build_spawned_agent_detail, collect_wait_targets, encode_agent_ref, encode_run_ref,
    is_active_turn_status, resolve_child_agent_session,
};
use self::errors::{api_error, internal_api_error, load_parent_session, ApiResult};
pub use self::errors::{SubagentError, SubagentErrorKind};
use self::providers::load_requested_model_catalogs;
use self::request::ensure_requested_labels_available;
use self::worktrees::{cleanup_archived_subagent_worktree, plan_subagent_worktree_creation};

pub use init::init_subagents;

#[derive(Clone)]
pub struct SpawnedChild {
    child: SubagentInvocationChild,
    worktree_path: Option<String>,
    last_event_seq: i64,
}
