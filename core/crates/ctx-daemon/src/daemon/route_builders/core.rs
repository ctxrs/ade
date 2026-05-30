use super::*;

impl core_deps::CoreRouteDeps {
    pub fn auth(&self) -> AuthHandle {
        AuthHandle::new(
            self.auth_token.clone(),
            Arc::clone(&self.mcp_auth),
            self.global_store.clone(),
            self.ops_events.clone(),
        )
    }
    pub fn health(&self) -> HealthHandle {
        HealthHandle::new(
            self.data_root.clone(),
            self.daemon_url.clone(),
            self.auth_token.clone(),
            Arc::clone(&self.storage_guard),
        )
    }
    pub fn diagnostics(&self) -> DiagnosticsHandle {
        DiagnosticsHandle::new(
            self.health(),
            self.data_root.clone(),
            Arc::clone(&self.execution_setup),
            Arc::clone(&self.providers),
        )
    }
    pub fn blob(&self) -> BlobHandle {
        BlobHandle::new(self.data_root.clone(), self.global_store.clone())
    }
    pub fn request_base(&self) -> RequestBaseHandle {
        RequestBaseHandle::new(self.daemon_url.clone(), self.public_base_url.clone())
    }
    pub fn repo_onboarding(&self) -> RepoOnboardingHandle {
        RepoOnboardingHandle::new(self.data_root.clone())
    }
    pub fn logs(&self) -> LogsHandle {
        LogsHandle::new(self.data_root.clone())
    }
    pub fn dictation(&self) -> DictationHandle {
        DictationHandle::new(self.global_store.clone())
    }
}
