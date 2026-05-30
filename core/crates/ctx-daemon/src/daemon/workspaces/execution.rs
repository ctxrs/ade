use ctx_core::models::{ExecutionEnvironment, Worktree};
use ctx_settings_model::{ExecutionMode, ExecutionSettings};

#[cfg(test)]
mod tests;

pub struct ResolvedExistingWorktreeExecution {
    pub worktree: Worktree,
    pub effective: ExecutionSettings,
}

impl ResolvedExistingWorktreeExecution {
    pub fn execution_environment(&self) -> ExecutionEnvironment {
        execution_environment_from_settings(&self.effective)
    }
}

pub fn execution_environment_from_settings(settings: &ExecutionSettings) -> ExecutionEnvironment {
    match settings.mode {
        ExecutionMode::Host => ExecutionEnvironment::Host,
        ExecutionMode::Sandbox => ExecutionEnvironment::Sandbox,
    }
}
