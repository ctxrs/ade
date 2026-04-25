use super::*;

impl ExecutionSetupCoordinator {
    pub fn new(
        data_root: PathBuf,
        harness: Arc<HarnessRuntimeManager>,
        perf_telemetry: PerfTelemetry,
        ops_events: OpsEvents,
    ) -> Self {
        let operations = Arc::new(DefaultWarmupOperations::new(
            data_root.clone(),
            ops_events.clone(),
        ));
        Self::new_with_operations(data_root, harness, perf_telemetry, ops_events, operations)
    }

    pub(super) fn new_with_operations(
        data_root: PathBuf,
        harness: Arc<HarnessRuntimeManager>,
        perf_telemetry: PerfTelemetry,
        ops_events: OpsEvents,
        operations: Arc<dyn SharedWarmupOperations>,
    ) -> Self {
        Self {
            prewarm: LaunchPrewarmCoordinator::new(operations),
            data_root,
            harness,
            perf_telemetry,
            ops_events,
            inner: Mutex::new(CoordinatorState::default()),
        }
    }

    pub fn spawn_startup_prewarm(self: &Arc<Self>, execution: ExecutionSettings) {
        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            #[cfg(test)]
            // Tests temporarily rebind sandbox CLI process env vars, so startup prewarm
            // must serialize with the shared test lock before it can observe them.
            let _sandbox_cli_env_test_lock = crate::test_support::sandbox_cli_env_test_lock()
                .lock()
                .await;
            coordinator.run_startup_prewarm(execution).await;
        });
    }

    #[cfg_attr(test, allow(dead_code))]
    pub(crate) async fn record_startup_prewarm_error(&self, message: String) {
        let attempted_at = format_ts(Utc::now());
        let snapshot = StartupPrewarmSnapshot {
            state: StartupPrewarmState::Error,
            target_image: String::new(),
            needs_prewarm: false,
            machine_ready: false,
            image_present: false,
            image_ref_changed: false,
            bundled_image_digest_changed: false,
            last_attempt_at: Some(attempted_at),
            last_success_at: None,
            error: Some(message.clone()),
        };
        self.set_startup_snapshot(snapshot).await;
        let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
        event.meta = Some(json!({"error": message}));
        self.ops_events.emit(event);
    }

    pub async fn start_workspace_launch(
        self: &Arc<Self>,
        workspace: Workspace,
        settings: ExecutionSettings,
        daemon_url: String,
    ) -> ExecutionLaunchSnapshot {
        let (job, snapshot) = {
            let mut inner = self.inner.lock().await;

            if let Some(existing_job_id) = inner
                .running_launch_by_workspace
                .get(&workspace.id)
                .cloned()
            {
                if let Some(existing) = inner.launch_jobs.get(&existing_job_id) {
                    return existing.snapshot();
                }
                // Stale pointer: drop it so a new launch can start cleanly.
                inner.running_launch_by_workspace.remove(&workspace.id);
            }

            let job_id = uuid::Uuid::new_v4().to_string();
            let job = Arc::new(LaunchJob::new(job_id.clone(), workspace.id));
            if matches!(settings.mode, ExecutionMode::Host) {
                seed_workspace_launch_initial_state(job.as_ref(), &settings);
            } else {
                let _ = job.transition_phase(
                    HarnessSetupPhase::MachineCheck,
                    "checking container runtime",
                );
            }
            let snapshot = job.snapshot();

            inner
                .running_launch_by_workspace
                .insert(workspace.id, job_id.clone());
            inner.remember_launch_job(Arc::clone(&job));

            (job, snapshot)
        };

        let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchSnapshot {
            snapshot: snapshot.clone(),
        });

        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator
                .run_workspace_launch(job, workspace, settings, daemon_url)
                .await;
        });

        snapshot
    }

    pub async fn start_runtime_prewarm(
        self: &Arc<Self>,
        settings: ExecutionSettings,
        scope: RuntimePrewarmScope,
    ) -> ExecutionLaunchSnapshot {
        let (shared_job, snapshot) = {
            let mut inner = self.inner.lock().await;
            if let Some(existing) = inner.prewarm_jobs.find_compatible(&settings, scope) {
                if existing.request_scope(scope) {
                    return existing.snapshot();
                }
                inner
                    .prewarm_jobs
                    .remove_if_current(existing.key(), &existing);
            }

            let job = Arc::new(SharedPrewarmLaunchJob::new(
                uuid::Uuid::new_v4().to_string(),
                &settings,
                scope,
            ));
            let snapshot = job.snapshot();
            inner.prewarm_jobs.insert(Arc::clone(&job));
            inner.remember_launch_job(job.job());
            (job, snapshot)
        };

        let launch_job = shared_job.job();
        let _ = launch_job
            .tx
            .send(ExecutionLaunchStreamEvent::LaunchSnapshot {
                snapshot: snapshot.clone(),
            });

        let coordinator = Arc::clone(self);
        tokio::spawn(async move {
            coordinator.run_runtime_prewarm(shared_job, settings).await;
        });

        snapshot
    }

    pub async fn launch_status(&self, job_id: &str) -> Option<ExecutionLaunchSnapshot> {
        let job = {
            let inner = self.inner.lock().await;
            inner.launch_jobs.get(job_id).cloned()
        }?;
        Some(job.snapshot())
    }

    pub async fn subscribe_launch(
        &self,
        job_id: &str,
    ) -> Option<(
        ExecutionLaunchSnapshot,
        broadcast::Receiver<ExecutionLaunchStreamEvent>,
    )> {
        let job = {
            let inner = self.inner.lock().await;
            inner.launch_jobs.get(job_id).cloned()
        }?;
        let rx = job.tx.subscribe();
        let snapshot = job.snapshot();
        Some((snapshot, rx))
    }

    pub async fn startup_status(&self) -> StartupPrewarmSnapshot {
        let inner = self.inner.lock().await;
        inner.startup.clone()
    }
}
