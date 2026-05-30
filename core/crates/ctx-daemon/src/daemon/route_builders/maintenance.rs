use super::*;

impl maintenance_deps::MaintenanceRouteDeps {
    pub fn run_archive(&self) -> RunArchiveHandle {
        RunArchiveHandle::new(self.workspace_store_lookup())
    }
    pub fn org_policy(&self) -> OrgPolicyHandle {
        OrgPolicyHandle::new(self.global_store.clone())
    }
    pub fn merge_queue_api(&self) -> MergeQueueApiHandle {
        MergeQueueApiHandle::new(Arc::clone(&self.merge_queue_host))
    }
    pub fn update_release(&self) -> UpdateReleaseHandle {
        UpdateReleaseHandle::new(self.data_root.clone())
    }
    pub fn update_activity(&self) -> UpdateActivityHandle {
        UpdateActivityHandle::new(
            self.global_store.clone(),
            self.stores.clone(),
            Arc::clone(&self.update_drain),
            self.data_root.clone(),
        )
    }
    pub fn settings(&self) -> SettingsHandle {
        SettingsHandle::new(
            self.global_store.clone(),
            self.telemetry.clone(),
            self.perf_telemetry.clone(),
            Arc::clone(&self.resource_sampler),
            Arc::clone(&self.resource_governance),
            Arc::clone(&self.providers),
            Arc::clone(&self.terminals),
        )
    }
    pub fn resource_utilization(&self) -> ResourceUtilizationHandle {
        ResourceUtilizationHandle::new(
            self.workspace_store_lookup(),
            Arc::clone(&self.providers),
            Arc::clone(&self.resource_sampler),
        )
    }
    pub fn telemetry(&self) -> TelemetryHandle {
        TelemetryHandle::new(
            self.data_root.clone(),
            self.telemetry.clone(),
            self.perf_telemetry.clone(),
        )
    }
    pub fn update_drain(&self) -> UpdateDrainHandle {
        UpdateDrainHandle::new(
            self.global_store.clone(),
            self.stores.clone(),
            Arc::clone(&self.update_drain),
        )
    }
    pub(super) fn daemon_shutdown_with_session_routes(
        &self,
        session_routes: &session_deps::SessionRouteDeps,
    ) -> DaemonShutdownHandle {
        let shutdown_host = DaemonShutdownHost::new(DaemonShutdownHostParts {
            global_store: self.global_store.clone(),
            stores: self.stores.clone(),
            session_stores: session_routes.session_store_lookup(),
            session_lifecycle: Arc::clone(&self.sessions),
            session_publication: session_routes.session_publication_effects(),
            provider_lifecycle: Arc::clone(&self.providers),
            update_drain: Arc::clone(&self.update_drain),
            substrate_lifecycle: Arc::clone(&self.harness),
            shutdown_signal: self.shutdown_tx.clone(),
        });
        DaemonShutdownHandle::new(self.local_shutdown_token.clone(), shutdown_host)
    }
}
