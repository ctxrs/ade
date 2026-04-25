use super::*;

#[async_trait]
pub(crate) trait SharedWarmupOperations: Send + Sync {
    async fn warm_runtime(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()>;

    async fn warm_runtime_launch_ready(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()>;

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()>;
}

#[derive(Clone)]
pub(crate) struct DefaultWarmupOperations {
    data_root: PathBuf,
    ops_events: crate::ops_events::OpsEvents,
}

impl DefaultWarmupOperations {
    pub(crate) fn new(data_root: PathBuf, ops_events: crate::ops_events::OpsEvents) -> Self {
        Self {
            data_root,
            ops_events,
        }
    }
}

#[async_trait]
impl SharedWarmupOperations for DefaultWarmupOperations {
    async fn warm_runtime(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        ctx_harness_runtime::prewarm_selected_runtime_with_observer(
            &self.data_root,
            &settings.container,
            Some(observer.as_ref()),
        )
        .await
    }

    async fn warm_runtime_launch_ready(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        ctx_harness_runtime::prewarm_selected_runtime_for_launch_with_observer(
            &self.data_root,
            &settings.container,
            Some(observer.as_ref()),
        )
        .await?;
        if let Some(record) =
            ctx_harness_runtime::selected_shared_substrate_lifecycle(&self.data_root)?
        {
            self.ops_events
                .emit(crate::ops_events::substrate_lifecycle_observed_event(
                    &record,
                    crate::ops_events::SubstrateLifecycleOpsEventContext {
                        source: "runtime_prewarm_launch_ready",
                        workspace_id: None,
                    },
                ));
        }
        Ok(())
    }

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        observer.on_phase(HarnessSetupPhase::ImageLoad, "warming container builder");
        crate::container_builder::ensure_builder_ready(&self.data_root).await?;
        if let Some(record) =
            ctx_harness_runtime::selected_shared_substrate_lifecycle(&self.data_root)?
        {
            self.ops_events
                .emit(crate::ops_events::substrate_lifecycle_observed_event(
                    &record,
                    crate::ops_events::SubstrateLifecycleOpsEventContext {
                        source: "builder_prewarm",
                        workspace_id: None,
                    },
                ));
        }
        Ok(())
    }
}
