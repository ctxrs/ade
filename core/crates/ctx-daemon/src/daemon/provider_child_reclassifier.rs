use std::sync::Arc;

use ctx_provider_runtime::ProviderRuntime;
use tokio::sync::broadcast;

pub(super) fn spawn_provider_child_reclassifier(host: Arc<ProviderChildReclassifierHost>) {
    ctx_provider_runtime::provider_child_reclassifier::spawn_provider_child_reclassifier(host);
}

pub(in crate::daemon) struct ProviderChildReclassifierHost {
    shutdown_tx: broadcast::Sender<()>,
    providers: Arc<ProviderRuntime>,
}

impl ProviderChildReclassifierHost {
    pub(in crate::daemon) fn new(
        shutdown_tx: broadcast::Sender<()>,
        providers: Arc<ProviderRuntime>,
    ) -> Self {
        Self {
            shutdown_tx,
            providers,
        }
    }
}

#[async_trait::async_trait]
impl ctx_provider_runtime::provider_child_reclassifier::ProviderChildReclassifierHost
    for ProviderChildReclassifierHost
{
    fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown_tx.subscribe()
    }

    async fn provider_process_pids(&self) -> Vec<u32> {
        self.providers.provider_process_pids().await
    }

    fn tool_slice_unit(&self) -> &'static str {
        #[cfg(target_os = "linux")]
        {
            super::tool_cgroup::TOOL_SLICE_UNIT
        }
        #[cfg(not(target_os = "linux"))]
        {
            ""
        }
    }
}
