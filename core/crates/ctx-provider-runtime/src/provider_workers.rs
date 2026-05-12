use std::collections::HashSet;
use std::sync::Arc;

use ctx_providers::adapters::{
    ProviderAdapter, ProviderRestartMode, ProviderSessionSweepConfig, ProviderSessionSweepStats,
};

use crate::ProviderRuntime;

impl ProviderRuntime {
    pub async fn provider_worker_adapters_for_shutdown(
        &self,
    ) -> Vec<(String, Arc<dyn ProviderAdapter>)> {
        self.all_provider_adapter_entries().await
    }

    pub async fn sweep_provider_workers_once(
        &self,
        config: ProviderSessionSweepConfig,
    ) -> ProviderSessionSweepStats {
        let mut stats = ProviderSessionSweepStats::default();
        let mut seen = HashSet::<usize>::new();
        for (_, adapter) in self.provider_worker_adapters_for_shutdown().await {
            let identity = (Arc::as_ptr(&adapter) as *const ()) as usize;
            if !seen.insert(identity) {
                continue;
            }
            match adapter.reap_idle_sessions(config).await {
                Ok(adapter_stats) => {
                    stats.reaped += adapter_stats.reaped;
                    stats.skipped_busy += adapter_stats.skipped_busy;
                    stats.dead_removed += adapter_stats.dead_removed;
                    stats.status_errors += adapter_stats.status_errors;
                }
                Err(err) => {
                    stats.status_errors += 1;
                    tracing::debug!(err = %err, "provider worker sweep failed");
                }
            }
        }
        stats
    }

    pub async fn shutdown_provider_adapters(&self, reason: &str) {
        for (id, adapter) in self.provider_worker_adapters_for_shutdown().await {
            if let Err(err) = adapter
                .restart(reason, ProviderRestartMode::Immediate)
                .await
            {
                tracing::debug!(
                    "failed to stop provider adapter {id} during daemon shutdown: {err:#}"
                );
            }
        }
    }

    pub async fn set_provider_session_pinned(&self, session_key: String, pinned: bool) {
        let mut seen = HashSet::<usize>::new();
        for (_, adapter) in self.provider_worker_adapters_for_shutdown().await {
            let identity = (Arc::as_ptr(&adapter) as *const ()) as usize;
            if !seen.insert(identity) {
                continue;
            }
            if let Err(err) = adapter
                .set_session_pinned(session_key.clone(), pinned)
                .await
            {
                tracing::debug!(
                    session_id = %session_key,
                    pinned,
                    err = %err,
                    "failed to update provider worker pin state"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    use anyhow::Result;
    use async_trait::async_trait;
    use ctx_providers::adapters::{
        ProviderAdapter, ProviderHealth, ProviderProcessInfo, ProviderRestartMode,
        ProviderSessionSweepConfig, ProviderSessionSweepStats, ProviderStatus, ProviderUsability,
        RunHandle, TurnInput,
    };

    use super::*;

    #[derive(Default)]
    struct RecordingProviderAdapter {
        restart_calls: StdMutex<Vec<(String, ProviderRestartMode)>>,
        reap_calls: StdMutex<Vec<ProviderSessionSweepConfig>>,
        reap_result: StdMutex<ProviderSessionSweepStats>,
        pin_calls: StdMutex<Vec<(String, bool)>>,
    }

    impl RecordingProviderAdapter {
        fn restart_calls(&self) -> Vec<(String, ProviderRestartMode)> {
            self.restart_calls
                .lock()
                .expect("recording adapter restart lock")
                .clone()
        }

        fn reap_calls(&self) -> Vec<ProviderSessionSweepConfig> {
            self.reap_calls
                .lock()
                .expect("recording adapter reap lock")
                .clone()
        }

        fn set_reap_result(&self, stats: ProviderSessionSweepStats) {
            *self
                .reap_result
                .lock()
                .expect("recording adapter reap result lock") = stats;
        }

        fn pin_calls(&self) -> Vec<(String, bool)> {
            self.pin_calls
                .lock()
                .expect("recording adapter pin lock")
                .clone()
        }
    }

    #[async_trait]
    impl ProviderAdapter for RecordingProviderAdapter {
        async fn inspect(&self) -> Result<ProviderStatus> {
            Ok(ProviderStatus {
                provider_id: "recording".into(),
                installed: true,
                detected_path: None,
                version: Some("test".into()),
                capabilities: None,
                health: ProviderHealth::Ok,
                diagnostics: Vec::new(),
                details: HashMap::new(),
                usability: ProviderUsability::default(),
            })
        }

        async fn run(
            &self,
            _input: TurnInput,
            _workdir: PathBuf,
            _env: HashMap<String, String>,
            _event_sink: tokio::sync::mpsc::Sender<ctx_providers::events::NormalizedEvent>,
            _hooks: ctx_providers::adapters::ProviderRunHooks,
        ) -> Result<RunHandle> {
            anyhow::bail!("not used in test");
        }

        async fn cancel(&self, _handle: &mut RunHandle) -> Result<()> {
            Ok(())
        }

        async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
            Vec::new()
        }

        async fn restart(&self, reason: &str, mode: ProviderRestartMode) -> Result<()> {
            self.restart_calls
                .lock()
                .expect("recording adapter restart lock")
                .push((reason.to_string(), mode));
            Ok(())
        }

        async fn reap_idle_sessions(
            &self,
            config: ProviderSessionSweepConfig,
        ) -> Result<ProviderSessionSweepStats> {
            self.reap_calls
                .lock()
                .expect("recording adapter reap lock")
                .push(config);
            Ok(*self
                .reap_result
                .lock()
                .expect("recording adapter reap result lock"))
        }

        async fn set_session_pinned(&self, session_key: String, pinned: bool) -> Result<()> {
            self.pin_calls
                .lock()
                .expect("recording adapter pin lock")
                .push((session_key, pinned));
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_adapter_collection_includes_root_and_target_adapters() {
        let root_adapter = Arc::new(RecordingProviderAdapter::default());
        let target_adapter = Arc::new(RecordingProviderAdapter::default());
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("root".into(), root_adapter);
        let runtime = ProviderRuntime::new(providers);
        runtime
            .upsert_target_provider_adapter("root@host".into(), target_adapter)
            .await;

        let mut ids = runtime
            .provider_worker_adapters_for_shutdown()
            .await
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        ids.sort();

        assert_eq!(ids, vec!["root".to_string(), "root@host".to_string()]);
    }

    #[tokio::test]
    async fn shutdown_requests_immediate_restart_for_all_worker_adapters() {
        let root_adapter = Arc::new(RecordingProviderAdapter::default());
        let target_adapter = Arc::new(RecordingProviderAdapter::default());
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("root".into(), root_adapter.clone());
        let runtime = ProviderRuntime::new(providers);
        runtime
            .upsert_target_provider_adapter("root@host".into(), target_adapter.clone())
            .await;

        runtime.shutdown_provider_adapters("test shutdown").await;

        assert_eq!(
            root_adapter.restart_calls(),
            vec![("test shutdown".to_string(), ProviderRestartMode::Immediate)]
        );
        assert_eq!(
            target_adapter.restart_calls(),
            vec![("test shutdown".to_string(), ProviderRestartMode::Immediate)]
        );
    }

    #[tokio::test]
    async fn sweep_dedupes_shared_adapters_and_aggregates_stats() {
        let shared_adapter = Arc::new(RecordingProviderAdapter::default());
        shared_adapter.set_reap_result(ProviderSessionSweepStats {
            reaped: 1,
            skipped_busy: 2,
            dead_removed: 0,
            status_errors: 0,
        });
        let other_adapter = Arc::new(RecordingProviderAdapter::default());
        other_adapter.set_reap_result(ProviderSessionSweepStats {
            reaped: 0,
            skipped_busy: 0,
            dead_removed: 1,
            status_errors: 1,
        });

        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("root".into(), shared_adapter.clone());
        providers.insert("other".into(), other_adapter.clone());
        let runtime = ProviderRuntime::new(providers);
        runtime
            .upsert_target_provider_adapter("root@host".into(), shared_adapter.clone())
            .await;

        let config = ProviderSessionSweepConfig {
            idle_ttl: std::time::Duration::from_secs(7),
            max_idle_sessions: 3,
            interval: std::time::Duration::from_secs(11),
        };
        let stats = runtime.sweep_provider_workers_once(config).await;

        assert_eq!(
            stats,
            ProviderSessionSweepStats {
                reaped: 1,
                skipped_busy: 2,
                dead_removed: 1,
                status_errors: 1,
            }
        );
        assert_eq!(shared_adapter.reap_calls().len(), 1);
        assert_eq!(other_adapter.reap_calls().len(), 1);
        assert_eq!(shared_adapter.reap_calls()[0].idle_ttl, config.idle_ttl);
        assert_eq!(
            shared_adapter.reap_calls()[0].max_idle_sessions,
            config.max_idle_sessions
        );
        assert_eq!(shared_adapter.reap_calls()[0].interval, config.interval);
    }

    #[tokio::test]
    async fn pin_propagation_dedupes_shared_worker_adapters() {
        let shared_adapter = Arc::new(RecordingProviderAdapter::default());
        let other_adapter = Arc::new(RecordingProviderAdapter::default());
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("root".into(), shared_adapter.clone());
        providers.insert("other".into(), other_adapter.clone());
        let runtime = ProviderRuntime::new(providers);
        runtime
            .upsert_target_provider_adapter("root@host".into(), shared_adapter.clone())
            .await;

        runtime
            .set_provider_session_pinned("session-1".to_string(), true)
            .await;

        assert_eq!(
            shared_adapter.pin_calls(),
            vec![("session-1".to_string(), true)]
        );
        assert_eq!(
            other_adapter.pin_calls(),
            vec![("session-1".to_string(), true)]
        );
    }
}
