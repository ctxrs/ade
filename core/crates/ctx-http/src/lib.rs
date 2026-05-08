pub mod api;
mod async_util;
pub mod daemon;
mod execution_effective;
pub mod git_status;
mod installer;
pub mod provider_guard;
mod provider_launch;
pub mod provider_restart;
pub mod resource_telemetry;
pub mod scheduler;
pub(crate) mod workspace_provider_model_preferences;
mod workspace_runtime;

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
