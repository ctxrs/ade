pub mod api;
mod async_util;
pub mod daemon;
mod dictation_livekit;
mod execution_effective;
pub mod git_status;
mod installer;
mod memleak_debug;
mod merge_queue;
pub mod provider_guard;
mod provider_launch;
pub mod provider_restart;
mod resource_governance;
pub mod resource_telemetry;
mod runtime_adapters;
pub mod scheduler;
mod storage_guard;
mod terminal_launch;
mod tool_cgroup;
mod vcs_hooks;
mod web_session_launch;
pub(crate) mod workspace_provider_model_preferences;
mod workspace_runtime;
mod worktree_bootstrap;
mod worktree_data_plane;

#[cfg(feature = "fault_injection")]
pub mod fault_injection;

#[cfg(test)]
pub(crate) mod test_support;

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
