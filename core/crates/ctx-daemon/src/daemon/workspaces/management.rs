use crate::daemon::WorkspacesHandle;

impl WorkspacesHandle {
    #[cfg(target_os = "macos")]
    pub fn shared_vm_container_runtime_available(&self) -> bool {
        ctx_harness_runtime::local_runtime_available(
            &self.state.core.data_root,
            &ctx_settings_model::ContainerRuntimeKind::SharedVmContainer,
        )
    }
}
