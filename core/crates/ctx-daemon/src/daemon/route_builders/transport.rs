use super::*;

impl RouteBuilder {
    pub fn mobile_store(&self) -> MobileStoreHandle {
        MobileStoreHandle::new(self.state.global_store().clone())
    }
    pub fn mobile_runtime(&self) -> MobileRuntimeHandle {
        MobileRuntimeHandle::new(
            self.state.global_store().clone(),
            self.state.transport.mobile_tunnel.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.is_some(),
        )
    }
    pub fn mobile_secure_proxy(&self) -> MobileSecureProxyHandle {
        MobileSecureProxyHandle::new(
            self.state.global_store().clone(),
            self.health(),
            self.state.telemetry.telemetry.clone(),
        )
    }
    fn terminal_launch_host(&self) -> TerminalLaunchHost {
        TerminalLaunchHost::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            Arc::clone(&self.state.execution.harness),
            Arc::clone(&self.state.transport.terminals),
        )
    }
    fn web_session_worker_runtime_host(&self) -> WebSessionWorkerRuntimeHost {
        WebSessionWorkerRuntimeHost::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }
    fn web_session_launch_host(&self) -> WebSessionLaunchHost {
        WebSessionLaunchHost::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
            self.web_session_worker_runtime_host(),
            Arc::clone(&self.state.transport.web_sessions),
        )
    }
    pub fn terminal_route(&self) -> TerminalRouteHandle {
        TerminalRouteHandle::new(
            Arc::clone(&self.state.transport.terminals),
            self.terminal_launch_host(),
        )
    }
    pub fn web_session_route(&self) -> WebSessionRouteHandle {
        WebSessionRouteHandle::new(
            Arc::clone(&self.state.transport.web_sessions),
            self.web_session_launch_host(),
        )
    }
}
