mod request;
mod wait;

pub use request::{
    build_subagent_request_json, collect_provider_ids, normalize_subagent_labels,
    parse_subagent_worktree, resolve_max_subagents_per_call, SubagentRequestAgent,
    SubagentWorktreeSelection, DEFAULT_MAX_ACTIVE_SUBAGENTS_PER_PARENT,
    DEFAULT_MAX_SUBAGENTS_PER_CALL, DEFAULT_MAX_SUBAGENT_DEPTH,
};
pub use wait::{
    normalize_wait_agent_ids, parse_wait_mode, parse_wait_until, wait_predicate_satisfied,
    AgentWaitDetail, AgentWaitMode, AgentWaitUntil,
};
