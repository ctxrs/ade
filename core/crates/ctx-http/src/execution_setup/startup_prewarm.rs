use super::*;

mod gate;
mod metadata;
mod runtime_state;

impl ExecutionSetupCoordinator {
    pub(crate) async fn run_startup_prewarm(self: &Arc<Self>, exec: ExecutionSettings) {
        let attempted_at = format_ts(Utc::now());
        let target = ctx_harness_runtime::runtime_prewarm_target(&exec.container);
        let (initial_machine_ready, initial_image_present) = self
            .startup_runtime_state(&exec.container)
            .await
            .unwrap_or((false, false));
        let previous_last_success_at = {
            let inner = self.inner.lock().await;
            inner.startup.last_success_at.clone()
        };

        if !ctx_harness_runtime::local_runtime_available(&self.data_root, &exec.container.runtime) {
            let staged_status =
                match ctx_linux_sandbox_runtime::stage_linux_sandbox_runtime_downloads(
                    &self.data_root,
                    None,
                )
                .await
                {
                    Ok(status) => Some(status),
                    Err(err) => {
                        tracing::warn!(
                            error = %format_error_chain(&err),
                            "failed to stage Linux sandbox runtime downloads during startup prewarm"
                        );
                        None
                    }
                };
            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Skipped,
                target_image: target,
                needs_prewarm: false,
                machine_ready: false,
                image_present: false,
                image_ref_changed: false,
                bundled_image_digest_changed: false,
                last_attempt_at: Some(attempted_at),
                last_success_at: None,
                error: staged_status
                    .map(|status| status.message)
                    .or_else(|| Some("local sandbox runtime unavailable".to_string())),
            };
            self.set_startup_snapshot(snapshot).await;
            return;
        }

        {
            let mut inner = self.inner.lock().await;
            inner.startup = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Running,
                target_image: target.clone(),
                needs_prewarm: false,
                machine_ready: initial_machine_ready,
                image_present: initial_image_present,
                image_ref_changed: false,
                bundled_image_digest_changed: false,
                last_attempt_at: Some(attempted_at.clone()),
                last_success_at: inner.startup.last_success_at.clone(),
                error: None,
            };
        }
        let machine_ready_probe =
            self.spawn_startup_machine_ready_probe(&exec, initial_machine_ready);

        let gate = match gate::compute_prewarm_gate_with_runtime_state(
            self,
            &exec.container,
            initial_machine_ready,
            initial_image_present,
        )
        .await
        {
            Ok(gate) => gate,
            Err(err) => {
                let message = format_error_chain(&err);
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: target.clone(),
                    needs_prewarm: true,
                    machine_ready: initial_machine_ready,
                    image_present: initial_image_present,
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
                return;
            }
        };
        if !gate.needs_prewarm {
            let last_success_at = metadata::ensure_ready_startup_prewarm_metadata(
                self,
                &target,
                &gate,
                &attempted_at,
                previous_last_success_at.clone(),
            )
            .await
            .or_else(|| Some(attempted_at.clone()));
            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Ready,
                target_image: target,
                needs_prewarm: false,
                machine_ready: gate.machine_ready,
                image_present: gate.image_present,
                image_ref_changed: gate.image_ref_changed,
                bundled_image_digest_changed: gate.bundled_image_digest_changed,
                last_attempt_at: Some(attempted_at),
                last_success_at,
                error: None,
            };
            self.set_startup_snapshot(snapshot).await;
            return;
        }

        let stale_default_image_reloaded = match self
            .force_reload_stale_default_container_image_if_needed(&exec.container, &gate, None)
            .await
        {
            Ok(reloaded) => reloaded,
            Err(err) => {
                let message = format_error_chain(&err);
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: target.clone(),
                    needs_prewarm: true,
                    machine_ready: gate.machine_ready,
                    image_present: gate.image_present,
                    image_ref_changed: gate.image_ref_changed,
                    bundled_image_digest_changed: gate.bundled_image_digest_changed,
                    last_attempt_at: Some(attempted_at),
                    last_success_at: None,
                    error: Some(message.clone()),
                };
                self.set_startup_snapshot(snapshot).await;
                let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                event.meta = Some(json!({ "image": target, "error": message }));
                self.ops_events.emit(event);
                return;
            }
        };
        let prewarm_result = if stale_default_image_reloaded {
            Ok(())
        } else {
            self.startup_prewarm_runtime(&exec).await
        };

        match prewarm_result {
            Ok(()) => {
                let (machine_ready, image_present) =
                    match self.startup_runtime_state(&exec.container).await {
                        Ok(state) => state,
                        Err(err) => {
                            let message = format_error_chain(&err);
                            let snapshot = StartupPrewarmSnapshot {
                                state: StartupPrewarmState::Error,
                                target_image: target.clone(),
                                needs_prewarm: true,
                                machine_ready: gate.machine_ready,
                                image_present: gate.image_present,
                                image_ref_changed: gate.image_ref_changed,
                                bundled_image_digest_changed: gate.bundled_image_digest_changed,
                                last_attempt_at: Some(attempted_at),
                                last_success_at: None,
                                error: Some(message.clone()),
                            };
                            self.set_startup_snapshot(snapshot).await;
                            let mut event =
                                OpsEvent::new("warn", "execution.startup_prewarm_error");
                            event.meta = Some(json!({"image": target, "error": message}));
                            self.ops_events.emit(event);
                            return;
                        }
                    };

                if machine_ready && !image_present {
                    let message = format!(
                        "startup prewarm completed but runtime target '{target}' is still unavailable in the local sandbox runtime"
                    );
                    let snapshot = StartupPrewarmSnapshot {
                        state: StartupPrewarmState::Error,
                        target_image: target.clone(),
                        needs_prewarm: true,
                        machine_ready,
                        image_present,
                        image_ref_changed: gate.image_ref_changed,
                        bundled_image_digest_changed: gate.bundled_image_digest_changed,
                        last_attempt_at: Some(attempted_at),
                        last_success_at: None,
                        error: Some(message.clone()),
                    };
                    self.set_startup_snapshot(snapshot).await;
                    let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                    event.meta = Some(json!({"image": target, "error": message}));
                    self.ops_events.emit(event);
                    return;
                }

                if gate.machine_ready
                    && gate.image_present
                    && gate.bundled_image_digest_changed
                    && !stale_default_image_reloaded
                {
                    let message =
                        "startup prewarm downloaded updated local sandbox artifacts, but the loaded harness image is still stale and will be refreshed on the next workspace launch"
                            .to_string();
                    let snapshot = StartupPrewarmSnapshot {
                        state: StartupPrewarmState::Skipped,
                        target_image: target.clone(),
                        needs_prewarm: true,
                        machine_ready,
                        image_present,
                        image_ref_changed: gate.image_ref_changed,
                        bundled_image_digest_changed: gate.bundled_image_digest_changed,
                        last_attempt_at: Some(attempted_at),
                        last_success_at: None,
                        error: Some(message.clone()),
                    };
                    self.set_startup_snapshot(snapshot).await;
                    let mut event = OpsEvent::new("warn", "execution.startup_prewarm_deferred");
                    event.meta = Some(json!({"image": target, "reason": message}));
                    self.ops_events.emit(event);
                    return;
                }

                let needs_prewarm = !machine_ready || !image_present;
                let last_success_at = if machine_ready && image_present {
                    let metadata = StartupPrewarmMetadata {
                        image_ref: target.clone(),
                        bundled_image_fingerprint: gate.bundled_image_fingerprint,
                        ready_at: format_ts(Utc::now()),
                    };
                    let _ = write_prewarm_metadata(&self.data_root, &metadata).await;
                    Some(metadata.ready_at)
                } else {
                    None
                };
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Ready,
                    target_image: target,
                    needs_prewarm,
                    machine_ready,
                    image_present,
                    image_ref_changed: if needs_prewarm {
                        gate.image_ref_changed
                    } else {
                        false
                    },
                    bundled_image_digest_changed: if needs_prewarm {
                        gate.bundled_image_digest_changed
                    } else {
                        false
                    },
                    last_attempt_at: Some(attempted_at),
                    last_success_at,
                    error: None,
                };
                self.set_startup_snapshot(snapshot).await;
            }
            Err(err) => {
                let message = format_error_chain(&err);
                tracing::warn!("startup prewarm failed: {message}");
                let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                event.meta = Some(json!({"image": target, "error": message}));
                self.ops_events.emit(event);

                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: target,
                    needs_prewarm: true,
                    machine_ready: gate.machine_ready,
                    image_present: gate.image_present,
                    image_ref_changed: gate.image_ref_changed,
                    bundled_image_digest_changed: gate.bundled_image_digest_changed,
                    last_attempt_at: Some(attempted_at),
                    last_success_at: None,
                    error: Some(err.to_string()),
                };
                self.set_startup_snapshot(snapshot).await;
            }
        }

        if let Some(probe) = machine_ready_probe {
            let _ = probe.await;
        }
    }

    fn spawn_startup_machine_ready_probe(
        self: &Arc<Self>,
        exec: &ExecutionSettings,
        initial_machine_ready: bool,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if initial_machine_ready
            || exec.container.runtime != crate::settings::ContainerRuntimeKind::NativeContainer
        {
            return None;
        }
        let coordinator = Arc::clone(self);
        let settings = exec.container.clone();
        Some(tokio::spawn(async move {
            loop {
                {
                    let inner = coordinator.inner.lock().await;
                    if inner.startup.state != StartupPrewarmState::Running
                        || inner.startup.machine_ready
                    {
                        break;
                    }
                }

                if let Ok((machine_ready, _)) = coordinator.startup_runtime_state(&settings).await {
                    if machine_ready {
                        let mut inner = coordinator.inner.lock().await;
                        if inner.startup.state == StartupPrewarmState::Running {
                            inner.startup.machine_ready = true;
                        }
                        break;
                    }
                }

                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }))
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) async fn compute_prewarm_gate(
        &self,
        settings: &crate::settings::ContainerExecutionSettings,
    ) -> Result<PrewarmGate> {
        gate::compute_prewarm_gate(self, settings).await
    }

    pub(crate) async fn startup_runtime_state(
        &self,
        settings: &crate::settings::ContainerExecutionSettings,
    ) -> Result<(bool, bool)> {
        runtime_state::startup_runtime_state(self, settings).await
    }

    async fn startup_prewarm_runtime(&self, exec: &ExecutionSettings) -> Result<()> {
        runtime_state::startup_prewarm_runtime(self, exec).await
    }

    pub(super) async fn force_reload_stale_default_container_image_if_needed(
        &self,
        settings: &crate::settings::ContainerExecutionSettings,
        gate: &PrewarmGate,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        gate::force_reload_stale_default_container_image_if_needed(self, settings, gate, observer)
            .await
    }

    pub(crate) async fn refresh_startup_prewarm_metadata_after_successful_container_launch(
        &self,
        settings: &crate::settings::ContainerExecutionSettings,
    ) {
        metadata::refresh_startup_prewarm_metadata_after_successful_container_launch(
            self, settings,
        )
        .await;
    }

    pub(super) async fn set_startup_snapshot(&self, snapshot: StartupPrewarmSnapshot) {
        let mut inner = self.inner.lock().await;
        inner.startup = snapshot;
    }
}
