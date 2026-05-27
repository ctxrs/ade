use super::*;

impl RouteBuilder {
    pub fn provider_accounts(&self) -> ProviderAccountsHandle {
        ProviderAccountsHandle::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            Arc::clone(&self.state.providers),
        )
    }
    pub fn provider_bootstrap(&self) -> ProviderBootstrapHandle {
        ProviderBootstrapHandle::new(
            self.state.core.data_root.clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }
    pub(super) fn provider_workspace_launch_runtime(&self) -> Arc<ProviderWorkspaceLaunchRuntime> {
        Arc::new(ProviderWorkspaceLaunchRuntime::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
            Arc::clone(&self.state.execution.harness),
        ))
    }
    pub fn provider_options(&self) -> ProviderOptionsHandle {
        ProviderOptionsHandle::new(self.provider_workspace_launch_runtime())
    }
    pub fn provider_workspace_auth(&self) -> ProviderWorkspaceAuthHandle {
        ProviderWorkspaceAuthHandle::new(self.provider_workspace_launch_runtime())
    }
    pub fn provider_status(&self) -> ProviderStatusHandle {
        ProviderStatusHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }
    pub fn provider_admin(&self) -> ProviderAdminHandle {
        ProviderAdminHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }
    pub fn provider_install(&self) -> ProviderInstallHandle {
        ProviderInstallHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }
    pub fn provider_auth_import(&self) -> ProviderAuthImportHandle {
        ProviderAuthImportHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
        )
    }
    pub fn provider_usage(&self) -> ProviderUsageHandle {
        ProviderUsageHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.core.shutdown_tx.clone(),
        )
    }
    pub fn provider_harness_config(&self) -> ProviderHarnessConfigHandle {
        ProviderHarnessConfigHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
        )
    }
}
