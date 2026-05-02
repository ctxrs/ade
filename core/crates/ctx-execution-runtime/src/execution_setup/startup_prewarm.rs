use super::*;

impl ExecutionSetupCoordinator {
    pub async fn run_startup_prewarm(self: &Arc<Self>, exec: ExecutionSettings) {
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

        let gate = match self
            .compute_prewarm_gate_with_runtime_state(
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
                self.events.emit_event(
                    "warn",
                    "execution.startup_prewarm_error",
                    Some(json!({ "error": message })),
                );
                return;
            }
        };
        if !gate.needs_prewarm {
            let last_success_at = self
                .ensure_ready_startup_prewarm_metadata(
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
                self.events.emit_event(
                    "warn",
                    "execution.startup_prewarm_error",
                    Some(json!({ "image": target, "error": message })),
                );
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
                            self.events.emit_event(
                                "warn",
                                "execution.startup_prewarm_error",
                                Some(json!({ "image": target, "error": message })),
                            );
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
                    self.events.emit_event(
                        "warn",
                        "execution.startup_prewarm_error",
                        Some(json!({ "image": target, "error": message })),
                    );
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
                    self.events.emit_event(
                        "warn",
                        "execution.startup_prewarm_deferred",
                        Some(json!({ "image": target, "reason": message })),
                    );
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
                self.events.emit_event(
                    "warn",
                    "execution.startup_prewarm_error",
                    Some(json!({ "image": target, "error": message })),
                );

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
            || exec.container.runtime != crate::ContainerRuntimeKind::NativeContainer
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

    #[allow(dead_code)]
    pub(super) async fn compute_prewarm_gate(
        &self,
        settings: &crate::ContainerExecutionSettings,
    ) -> Result<PrewarmGate> {
        let (machine_ready, image_present) = self.startup_runtime_state(settings).await?;
        self.compute_prewarm_gate_with_runtime_state(settings, machine_ready, image_present)
            .await
    }

    async fn compute_prewarm_gate_with_runtime_state(
        &self,
        settings: &crate::ContainerExecutionSettings,
        machine_ready: bool,
        image_present: bool,
    ) -> Result<PrewarmGate> {
        let target = ctx_harness_runtime::runtime_prewarm_target(settings);
        let metadata = read_prewarm_metadata(&self.data_root).await?;
        let bundled_image_fingerprint = match settings.runtime {
            crate::ContainerRuntimeKind::NativeContainer => {
                bundled_image_fingerprint(&target).await?
            }
            crate::ContainerRuntimeKind::SharedVmContainer => None,
        };

        let image_ref_changed = metadata
            .as_ref()
            .map(|meta| meta.image_ref != target)
            .unwrap_or(false);
        let bundled_image_digest_changed = match metadata.as_ref() {
            Some(meta) => meta.bundled_image_fingerprint != bundled_image_fingerprint,
            None => image_present && bundled_image_fingerprint.is_some(),
        };

        let needs_prewarm = needs_prewarm(
            machine_ready,
            image_present,
            image_ref_changed,
            bundled_image_digest_changed,
        );

        Ok(PrewarmGate {
            machine_ready,
            image_present,
            image_ref_changed,
            bundled_image_digest_changed,
            needs_prewarm,
            bundled_image_fingerprint,
        })
    }

    pub(crate) async fn startup_runtime_state(
        &self,
        settings: &crate::ContainerExecutionSettings,
    ) -> Result<(bool, bool)> {
        match settings.runtime {
            crate::ContainerRuntimeKind::NativeContainer => {
                let target = ctx_harness_runtime::resolve_container_image(settings);
                let machine_ready = normalize_container_engine_ready_for_gate(
                    ctx_harness_runtime::sandbox_engine_ready(&self.data_root).await,
                )?;
                let image_present = if machine_ready {
                    ctx_harness_runtime::container_image_present(&self.data_root, &target).await?
                } else {
                    false
                };
                Ok((machine_ready, image_present))
            }
            crate::ContainerRuntimeKind::SharedVmContainer => {
                ctx_harness_runtime::selected_runtime_launch_readiness_state(
                    &self.data_root,
                    settings,
                )
                .await
            }
        }
    }

    async fn startup_prewarm_runtime(&self, exec: &ExecutionSettings) -> Result<()> {
        let _artifact_warmup = self.harness.begin_prewarm_artifact_activity();
        let scope = RuntimePrewarmScope::Runtime;
        self.prewarm.ensure_scope(exec, scope, None).await
    }

    pub(super) async fn force_reload_stale_default_container_image_if_needed(
        &self,
        settings: &crate::ContainerExecutionSettings,
        gate: &PrewarmGate,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if !gate.machine_ready || !gate.image_present || !gate.bundled_image_digest_changed {
            return Ok(false);
        }
        if !matches!(
            settings.runtime,
            crate::ContainerRuntimeKind::NativeContainer
        ) {
            return Ok(false);
        }
        let target = ctx_harness_runtime::runtime_prewarm_target(settings);
        if !ctx_sandbox_container_runtime::is_default_container_image(&target) {
            return Ok(false);
        }
        ctx_sandbox_container_runtime::force_reload_default_container_image(
            &self.data_root,
            &ctx_sandbox_container_runtime::SandboxCommandMode::NativeContainer,
            observer,
        )
        .await?;
        Ok(true)
    }

    async fn configured_startup_target(&self) -> Result<String> {
        self.harness.configured_startup_target().await
    }

    async fn ensure_ready_startup_prewarm_metadata(
        &self,
        target: &str,
        gate: &PrewarmGate,
        attempted_at: &str,
        previous_last_success_at: Option<String>,
    ) -> Option<String> {
        let existing = match read_prewarm_metadata(&self.data_root).await {
            Ok(metadata) => metadata,
            Err(err) => {
                tracing::warn!(
                    image = target,
                    error = %format_error_chain(&err),
                    "failed to read startup prewarm metadata while reporting reused readiness"
                );
                return previous_last_success_at;
            }
        };
        if let Some(metadata) = existing.as_ref() {
            if metadata.image_ref == target
                && !gate.image_ref_changed
                && !gate.bundled_image_digest_changed
            {
                return Some(metadata.ready_at.clone());
            }
        }

        let metadata = StartupPrewarmMetadata {
            image_ref: target.to_string(),
            bundled_image_fingerprint: gate.bundled_image_fingerprint.clone(),
            ready_at: attempted_at.to_string(),
        };
        if let Err(err) = write_prewarm_metadata(&self.data_root, &metadata).await {
            let message = format_error_chain(&err);
            tracing::warn!(
                image = target,
                error = %message,
                "failed to backfill startup prewarm metadata for an already-ready runtime"
            );
            self.events.emit_event(
                "warn",
                "execution.startup_prewarm_metadata_error",
                Some(json!({ "image": target, "error": message })),
            );
            return previous_last_success_at;
        }

        Some(metadata.ready_at)
    }

    pub(crate) async fn refresh_startup_prewarm_metadata_after_successful_container_launch(
        &self,
        settings: &crate::ContainerExecutionSettings,
    ) {
        let target = ctx_harness_runtime::runtime_prewarm_target(settings);
        let startup_target = match self.configured_startup_target().await {
            Ok(startup_target) => startup_target,
            Err(err) => {
                tracing::warn!(
                    target,
                    error = %format_error_chain(&err),
                    "failed to resolve configured startup target after successful launch; leaving startup prewarm metadata unchanged"
                );
                return;
            }
        };
        if startup_target != target {
            return;
        }

        let (metadata_missing, metadata_image_ref_changed) = match read_prewarm_metadata(
            &self.data_root,
        )
        .await
        {
            Ok(metadata) => {
                let image_ref_changed = metadata
                    .as_ref()
                    .is_some_and(|metadata| metadata.image_ref != target);
                (metadata.is_none(), image_ref_changed)
            }
            Err(err) => {
                tracing::warn!(
                    target,
                    error = %format_error_chain(&err),
                    "failed to read startup prewarm metadata after successful launch; leaving metadata unchanged"
                );
                return;
            }
        };
        let should_refresh = {
            let inner = self.inner.lock().await;
            metadata_missing
                || metadata_image_ref_changed
                || inner.startup.image_ref_changed
                || inner.startup.bundled_image_digest_changed
        };
        if !should_refresh {
            return;
        }

        let bundled_image_fingerprint = match settings.runtime {
            crate::ContainerRuntimeKind::NativeContainer => {
                match bundled_image_fingerprint(&target).await {
                    Ok(fingerprint) => fingerprint,
                    Err(err) => {
                        tracing::warn!(
                            target,
                            error = %format_error_chain(&err),
                            "failed to compute bundled image fingerprint after successful launch; leaving startup prewarm metadata unchanged"
                        );
                        return;
                    }
                }
            }
            crate::ContainerRuntimeKind::SharedVmContainer => None,
        };

        let ready_at = format_ts(Utc::now());
        let metadata = StartupPrewarmMetadata {
            image_ref: target.clone(),
            bundled_image_fingerprint,
            ready_at: ready_at.clone(),
        };
        if let Err(err) = write_prewarm_metadata(&self.data_root, &metadata).await {
            tracing::warn!(
                target,
                error = %format_error_chain(&err),
                "failed to persist refreshed startup prewarm metadata after successful launch"
            );
            return;
        }

        let mut inner = self.inner.lock().await;
        inner.startup.target_image = target;
        if inner.startup.state == StartupPrewarmState::Running {
            return;
        }
        inner.startup.state = StartupPrewarmState::Ready;
        inner.startup.needs_prewarm = false;
        inner.startup.machine_ready = true;
        inner.startup.image_present = true;
        inner.startup.image_ref_changed = false;
        inner.startup.bundled_image_digest_changed = false;
        inner.startup.last_success_at = Some(ready_at);
        inner.startup.error = None;
    }

    pub(super) async fn set_startup_snapshot(&self, snapshot: StartupPrewarmSnapshot) {
        let mut inner = self.inner.lock().await;
        inner.startup = snapshot;
    }
}
