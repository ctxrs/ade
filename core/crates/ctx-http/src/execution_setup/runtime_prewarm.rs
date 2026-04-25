use super::*;

impl ExecutionSetupCoordinator {
    pub(super) async fn run_runtime_prewarm(
        self: Arc<Self>,
        shared_job: Arc<SharedPrewarmLaunchJob>,
        settings: ExecutionSettings,
    ) {
        let launch_started = std::time::Instant::now();
        let job = shared_job.job();
        let observer = LaunchObserver {
            coordinator: Arc::clone(&self),
            job: Arc::clone(&job),
        };
        let is_host_mode = matches!(settings.mode, ExecutionMode::Host);
        let runtime_target = ctx_harness_runtime::runtime_prewarm_target(&settings.container);
        if is_host_mode {
            if let Some(terminal) = shared_job.complete_ready() {
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            self.clear_running_prewarm(&shared_job).await;
            return;
        }

        let run_result: Result<RequestedPrewarmScope> = async {
            let mut runtime_ready = false;
            let mut launch_ready = false;
            let mut builder_ready = false;

            loop {
                let requested_scope = shared_job.requested_scope();
                if requested_scope.requires_launch_ready_runtime() && !launch_ready {
                    if !ctx_harness_runtime::local_runtime_available(
                        &self.data_root,
                        &settings.container.runtime,
                    ) {
                        return Err(anyhow::anyhow!("local sandbox runtime unavailable"));
                    }
                    let _artifact_warmup = self.harness.begin_prewarm_artifact_activity();
                    self.prewarm
                        .ensure_runtime(&settings, true, Some(&observer))
                        .await?;
                    self.validate_runtime_prewarm_completion(&settings, &runtime_target, true)
                        .await?;
                    runtime_ready = true;
                    launch_ready = true;
                    continue;
                }

                if requested_scope.runtime_requested() && !runtime_ready {
                    if !ctx_harness_runtime::local_runtime_available(
                        &self.data_root,
                        &settings.container.runtime,
                    ) {
                        return Err(anyhow::anyhow!("local sandbox runtime unavailable"));
                    }
                    let _artifact_warmup = self.harness.begin_prewarm_artifact_activity();
                    self.prewarm
                        .ensure_runtime(&settings, false, Some(&observer))
                        .await?;
                    self.validate_runtime_prewarm_completion(&settings, &runtime_target, false)
                        .await?;
                    runtime_ready = true;
                    continue;
                }

                if requested_scope.builder_requested() && !builder_ready {
                    self.wait_for_builder_completion(observer.clone()).await?;
                    builder_ready = true;
                    continue;
                }

                if let Some(requested_scope) = shared_job
                    .reserve_ready_completion_if_scope_satisfied(
                        runtime_ready,
                        launch_ready,
                        builder_ready,
                    )
                {
                    return Ok(requested_scope);
                }

                tokio::task::yield_now().await;
            }
        }
        .await;

        match run_result {
            Ok(requested_scope) => {
                let runtime_kind = &settings.container.runtime;
                let ready_message = runtime_prewarm_ready_phase_message(
                    requested_scope.runtime_requested(),
                    runtime_kind,
                    requested_scope.requires_launch_ready_runtime(),
                );
                self.emit_phase(&job, HarnessSetupPhase::Ready, ready_message);
                let terminal = shared_job.mark_reserved_ready_terminal();
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            Err(err) => {
                self.finish_runtime_prewarm_error(shared_job, job, launch_started, err)
                    .await;
                return;
            }
        }

        self.clear_running_prewarm(&shared_job).await;
    }

    async fn validate_runtime_prewarm_completion(
        &self,
        settings: &ExecutionSettings,
        runtime_target: &str,
        requires_launch_ready_runtime: bool,
    ) -> Result<()> {
        if requires_launch_ready_runtime {
            match ctx_harness_runtime::selected_runtime_launch_readiness_state(
                &self.data_root,
                &settings.container,
            )
            .await
            {
                Ok((true, true)) => Ok(()),
                Ok((vm_ready, image_ready)) => Err(anyhow::anyhow!(
                    ctx_harness_runtime::launch_ready_gap_message(
                        settings.container.runtime.clone(),
                        runtime_target,
                        vm_ready,
                        image_ready,
                    )
                )),
                Err(err) => Err(err),
            }
        } else {
            match ctx_harness_runtime::selected_runtime_state(&self.data_root, &settings.container).await
            {
                Ok((machine_ready, image_present)) if machine_ready && image_present => Ok(()),
                Ok((machine_ready, _image_present)) if machine_ready => Err(anyhow::anyhow!(
                    "runtime prewarm completed but runtime target '{runtime_target}' is still unavailable in the local sandbox runtime"
                )),
                Ok((_machine_ready, _image_present)) => Err(anyhow::anyhow!(
                    "runtime prewarm downloaded startup artifacts for '{runtime_target}', but the local sandbox runtime still needs first-launch startup"
                )),
                Err(err) => Err(err),
            }
        }
    }

    async fn finish_runtime_prewarm_error(
        &self,
        shared_job: Arc<SharedPrewarmLaunchJob>,
        job: Arc<LaunchJob>,
        launch_started: std::time::Instant,
        err: anyhow::Error,
    ) {
        let message = format_error_chain(&err);
        let phase = job.current_phase().unwrap_or(HarnessSetupPhase::ImageLoad);
        self.emit_log(&job, phase, HarnessSetupLogLevel::Error, &message);
        if let Some(terminal) = shared_job.complete_error(message.clone()) {
            if let Some(completed) = terminal.completed_phase {
                self.record_phase_metric(completed.phase, completed.elapsed_ms, "error");
            }
            let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchError {
                snapshot: terminal.snapshot.clone(),
            });
            self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "error");

            let mut event = OpsEvent::new("error", "execution.runtime_prewarm_error");
            event.meta = Some(json!({
                "job_id": terminal.snapshot.job_id,
                "phase": terminal.snapshot.current_phase,
                "error": message,
            }));
            self.ops_events.emit(event);
        }
        self.clear_running_prewarm(&shared_job).await;
    }

    async fn wait_for_builder_completion(self: &Arc<Self>, observer: LaunchObserver) -> Result<()> {
        let coordinator = Arc::clone(self);
        let builder_observer = observer;
        tokio::spawn(async move {
            coordinator
                .prewarm
                .ensure_builder(Some(&builder_observer))
                .await
        })
        .await
        .map_err(|err| anyhow::anyhow!("builder warmup task join failed: {err}"))?
    }

    async fn clear_running_prewarm(&self, shared_job: &Arc<SharedPrewarmLaunchJob>) {
        let mut inner = self.inner.lock().await;
        inner
            .prewarm_jobs
            .remove_if_current(shared_job.key(), shared_job);
    }
}
