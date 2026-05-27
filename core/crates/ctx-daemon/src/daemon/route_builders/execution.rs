use super::*;

impl RouteBuilder {
    pub fn execution_launch(&self) -> ExecutionLaunchHandle {
        ExecutionLaunchHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            Arc::clone(&self.state.execution.setup),
            self.state.core.daemon_url.clone(),
        )
    }
    pub fn linux_sandbox_runtime(&self) -> LinuxSandboxRuntimeHandle {
        LinuxSandboxRuntimeHandle::new(
            self.state.core.data_root.clone(),
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            Arc::clone(&self.state.transport.terminals),
            Arc::clone(&self.state.execution.harness),
        )
    }
}
