use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use ctx_core::ids::WorkspaceId;
use ctx_core::models::Workspace;
use ctx_execution_runtime::{
    ContainerExecutionSettings, ExecutionHarness, ExecutionSettings, HarnessSetupObserver,
    HarnessSetupPhase, RuntimeActivityScope, RuntimeEventSink, RuntimeMetricsSink,
    SharedWarmupOperations,
};
use ctx_store::Store;
use ctx_workspace_runtime::HarnessRuntimeManager;

use ctx_observability::ops_events::{
    substrate_lifecycle_observed_event, OpsEvent, OpsEvents, SubstrateLifecycleOpsEventContext,
};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};

pub(crate) struct CtxRuntimeEventSink {
    inner: OpsEvents,
}

impl CtxRuntimeEventSink {
    pub(crate) fn new(inner: OpsEvents) -> Self {
        Self { inner }
    }
}

impl RuntimeEventSink for CtxRuntimeEventSink {
    fn emit_event(&self, level: &'static str, name: &'static str, meta: Option<serde_json::Value>) {
        let mut event = OpsEvent::new(level, name);
        event.meta = meta;
        self.inner.emit(event);
    }

    fn emit_substrate_lifecycle(
        &self,
        record: &ctx_avf_linux_runtime::SubstrateLifecycleRecord,
        source: &'static str,
        workspace_id: Option<WorkspaceId>,
    ) {
        self.inner.emit(substrate_lifecycle_observed_event(
            record,
            SubstrateLifecycleOpsEventContext {
                source,
                workspace_id: workspace_id.map(|value| value.0.to_string()),
            },
        ));
    }
}

pub(crate) struct CtxRuntimeMetricsSink {
    inner: PerfTelemetry,
}

impl CtxRuntimeMetricsSink {
    pub(crate) fn new(inner: PerfTelemetry) -> Self {
        Self { inner }
    }

    fn record_histogram(&self, name: &'static str, value_ms: u64, labels: HashMap<String, String>) {
        let perf = self.inner.clone();
        let metric = PerfMetric {
            name: name.to_string(),
            kind: PerfMetricKind::Histogram,
            unit: "ms".to_string(),
            value: value_ms as f64,
            labels,
        };
        tokio::spawn(async move {
            perf.record_metric(metric, None, None, None).await;
        });
    }
}

fn harness_setup_phase_label(phase: HarnessSetupPhase) -> &'static str {
    match phase {
        HarnessSetupPhase::ArtifactDownload => "artifact_download",
        HarnessSetupPhase::MachineCheck => "machine_check",
        HarnessSetupPhase::MachineStartOrInit => "machine_start_or_init",
        HarnessSetupPhase::ImageCheck => "image_check",
        HarnessSetupPhase::ImageLoad => "image_load",
        HarnessSetupPhase::ContainerCheck => "container_check",
        HarnessSetupPhase::ContainerStartOrCreate => "container_start_or_create",
        HarnessSetupPhase::RuntimeNetworkSetup => "runtime_network_setup",
        HarnessSetupPhase::Ready => "ready",
    }
}

impl RuntimeMetricsSink for CtxRuntimeMetricsSink {
    fn record_phase_duration(
        &self,
        phase: HarnessSetupPhase,
        elapsed_ms: u64,
        result: &'static str,
    ) {
        let mut labels = HashMap::new();
        labels.insert(
            "phase".to_string(),
            harness_setup_phase_label(phase).to_string(),
        );
        labels.insert("result".to_string(), result.to_string());
        self.record_histogram("execution.launch.phase_duration_ms", elapsed_ms, labels);
    }

    fn record_launch_duration(&self, elapsed_ms: u64, result: &'static str) {
        let mut labels = HashMap::new();
        labels.insert("result".to_string(), result.to_string());
        self.record_histogram("execution.launch.total_duration_ms", elapsed_ms, labels);
    }
}

#[cfg(test)]
mod tests {
    use super::harness_setup_phase_label;
    use ctx_execution_runtime::HarnessSetupPhase;

    #[test]
    fn phase_labels_remain_stable_snake_case_values() {
        assert_eq!(
            harness_setup_phase_label(HarnessSetupPhase::MachineStartOrInit),
            "machine_start_or_init",
        );
        assert_eq!(
            harness_setup_phase_label(HarnessSetupPhase::ContainerStartOrCreate),
            "container_start_or_create",
        );
        assert_eq!(
            harness_setup_phase_label(HarnessSetupPhase::RuntimeNetworkSetup),
            "runtime_network_setup",
        );
    }
}

#[derive(Clone)]
pub(crate) struct DefaultWarmupOperations {
    data_root: PathBuf,
    events: Arc<dyn RuntimeEventSink>,
}

impl DefaultWarmupOperations {
    pub(crate) fn new(data_root: PathBuf, events: Arc<dyn RuntimeEventSink>) -> Self {
        Self { data_root, events }
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
            self.events
                .emit_substrate_lifecycle(&record, "runtime_prewarm_launch_ready", None);
        }
        Ok(())
    }

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        observer.on_phase(HarnessSetupPhase::ImageLoad, "warming container builder");
        ctx_harness_runtime::container_builder::ensure_builder_ready(&self.data_root).await?;
        if let Some(record) =
            ctx_harness_runtime::selected_shared_substrate_lifecycle(&self.data_root)?
        {
            self.events
                .emit_substrate_lifecycle(&record, "builder_prewarm", None);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct CtxExecutionHarness {
    inner: Arc<HarnessRuntimeManager>,
}

impl CtxExecutionHarness {
    pub(crate) fn new(inner: Arc<HarnessRuntimeManager>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl ExecutionHarness for CtxExecutionHarness {
    fn begin_runtime_operation(&self) -> RuntimeActivityScope {
        self.inner.begin_runtime_operation_scope()
    }

    fn begin_prewarm_artifact_activity(&self) -> RuntimeActivityScope {
        self.inner.begin_prewarm_artifact_activity_scope()
    }

    async fn workspace_container_exists(&self, workspace_id: WorkspaceId) -> anyhow::Result<bool> {
        self.inner.workspace_container_exists(workspace_id).await
    }

    async fn ensure_workspace_container_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> anyhow::Result<()> {
        self.inner
            .ensure_workspace_container_with_observer(workspace, settings, daemon_url, observer)
            .await
    }

    async fn ensure_container_machine_ready(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> anyhow::Result<()> {
        self.inner
            .ensure_container_machine_ready(settings, observer)
            .await
    }

    async fn ensure_workspace_container_after_runtime_ready_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> anyhow::Result<()> {
        self.inner
            .ensure_workspace_container_after_runtime_ready_with_observer(
                workspace, settings, daemon_url, observer,
            )
            .await
    }

    async fn ensure_workspace_container_after_machine_ready_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> anyhow::Result<()> {
        self.inner
            .ensure_workspace_container_after_machine_ready_with_observer(
                workspace, settings, daemon_url, observer,
            )
            .await
    }

    async fn configured_startup_target(&self) -> anyhow::Result<String> {
        let db_path = self.inner.data_root().join("db").join("db.sqlite");
        let store = Store::open_sqlite(&db_path, Some(1)).await?;
        let settings = ctx_settings_service::load_settings(&store).await?;
        store.close().await;
        Ok(ctx_harness_runtime::runtime_prewarm_target(
            &settings.execution.unwrap_or_default().container,
        ))
    }
}
