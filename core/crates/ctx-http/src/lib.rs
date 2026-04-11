pub mod api;
pub mod async_util;
pub mod attachments;
pub mod buffers;
pub mod completions;
pub mod container_builder;
pub mod container_fs;
pub mod daemon;
pub mod dictation_livekit;
pub mod edit_plans;
pub mod execution_effective;
pub mod git_status;
pub mod installer;
pub mod llm;
pub mod logs;
pub mod lsp_catalog;
pub(crate) mod mcp_command;
pub mod memleak_debug;
pub mod merge_queue;
pub mod mobile_e2ee;
pub mod mobile_tunnel;
pub mod ops_events;
pub mod oracle;
pub mod order_seq;
pub mod perf_telemetry;
pub mod process_limits;
pub mod provider_child_reclassifier;
pub mod provider_guard;
pub(crate) mod provider_install_contract;
pub mod provider_launch;
pub mod provider_matrix;
pub(crate) mod provider_model_preferences;
pub mod provider_restart;
pub(crate) mod provider_usability;
pub mod provider_usage;
pub mod resource_governance;
pub mod resource_telemetry;
pub mod resource_utilization;
mod runtime_adapters;
pub mod scheduler;
pub mod settings;
pub mod storage_guard;
pub mod telemetry;
pub mod terminals;
pub mod title_generation;
pub mod title_generation_local;
pub mod tool_cgroup;
pub mod updates;
pub mod vcs_hooks;
pub mod web_sessions;
pub(crate) mod workspace_provider_model_preferences;
pub mod workspace_runtime;
pub mod worktree_bootstrap;
pub mod worktree_data_plane;

#[cfg(feature = "fault_injection")]
pub mod fault_injection;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod execution_setup;

#[cfg(not(feature = "fault_injection"))]
pub mod fault_injection {
    pub fn clear_failpoints() {}
    pub fn set_failpoint(_point: &'static str, _times: u32) {}
    pub fn maybe_fail(_point: &'static str) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
// EXCEPTION: these tests intentionally serialize env-var mutations with a sync lock
// that spans async calls so process-global state cannot interleave across test cases.
#[allow(clippy::await_holding_lock)]
mod lib_tests;
