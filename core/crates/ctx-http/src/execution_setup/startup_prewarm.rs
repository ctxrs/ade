use super::*;

impl ExecutionSetupCoordinator {
    pub(crate) async fn run_startup_prewarm(&self) {
        let attempted_at = format_ts(Utc::now());
        let db_path = self.data_root.join("db").join("db.sqlite");
        let settings = match Store::open_sqlite(&db_path, None).await {
            Ok(store) => {
                let loaded = crate::settings::load_settings(&store).await;
                store.close().await;
                match loaded {
                    Ok(settings) => settings,
                    Err(err) => {
                        let message = format!("failed to load execution settings: {err:#}");
                        let snapshot = StartupPrewarmSnapshot {
                            state: StartupPrewarmState::Error,
                            target_image: String::new(),
                            needs_prewarm: false,
                            machine_ready: false,
                            image_present: false,
                            image_ref_changed: false,
                            bundled_image_digest_changed: false,
                            last_attempt_at: Some(attempted_at.clone()),
                            last_success_at: None,
                            error: Some(message.clone()),
                        };
                        self.set_startup_snapshot(snapshot).await;
                        let mut event = OpsEvent::new("warn", "execution.startup_prewarm_error");
                        event.meta = Some(json!({"error": message}));
                        self.ops_events.emit(event);
                        return;
                    }
                }
            }
            Err(err) => {
                let message = format!("failed to open global settings store: {err:#}");
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: String::new(),
                    needs_prewarm: false,
                    machine_ready: false,
                    image_present: false,
                    image_ref_changed: false,
                    bundled_image_digest_changed: false,
                    last_attempt_at: Some(attempted_at.clone()),
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
        let exec = settings.execution.unwrap_or_default();
        let image = harness_runtime::resolve_container_image(&exec.container);
        let (initial_machine_ready, initial_image_present) = self
            .startup_runtime_state(&image)
            .await
            .unwrap_or((false, false));

        if !harness_runtime::container_runtime_available(&self.data_root) {
            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Skipped,
                target_image: image,
                needs_prewarm: false,
                machine_ready: false,
                image_present: false,
                image_ref_changed: false,
                bundled_image_digest_changed: false,
                last_attempt_at: Some(attempted_at),
                last_success_at: None,
                error: Some("container runtime unavailable".to_string()),
            };
            self.set_startup_snapshot(snapshot).await;
            return;
        }

        {
            let mut inner = self.inner.lock().await;
            inner.startup = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Running,
                target_image: image.clone(),
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

        let gate = match self.compute_prewarm_gate(&image).await {
            Ok(gate) => gate,
            Err(err) => {
                let message = format_error_chain(&err);
                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: image.clone(),
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
            let snapshot = StartupPrewarmSnapshot {
                state: StartupPrewarmState::Ready,
                target_image: image,
                needs_prewarm: false,
                machine_ready: gate.machine_ready,
                image_present: gate.image_present,
                image_ref_changed: gate.image_ref_changed,
                bundled_image_digest_changed: gate.bundled_image_digest_changed,
                last_attempt_at: Some(attempted_at),
                last_success_at: Some(format_ts(Utc::now())),
                error: None,
            };
            self.set_startup_snapshot(snapshot).await;
            return;
        }

        match self.startup_prewarm_runtime(&exec).await {
            Ok(()) => {
                let (machine_ready, image_present) = match self.startup_runtime_state(&image).await
                {
                    Ok(state) => state,
                    Err(err) => {
                        let message = format_error_chain(&err);
                        let snapshot = StartupPrewarmSnapshot {
                            state: StartupPrewarmState::Error,
                            target_image: image.clone(),
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
                        event.meta = Some(json!({"image": image, "error": message}));
                        self.ops_events.emit(event);
                        return;
                    }
                };

                if machine_ready && !image_present {
                    let message = format!(
                        "startup prewarm completed but harness image '{image}' is still unavailable in the local sandbox runtime"
                    );
                    let snapshot = StartupPrewarmSnapshot {
                        state: StartupPrewarmState::Error,
                        target_image: image.clone(),
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
                    event.meta = Some(json!({"image": image, "error": message}));
                    self.ops_events.emit(event);
                    return;
                }

                if gate.machine_ready && gate.image_present && gate.bundled_image_digest_changed {
                    let message =
                        "startup prewarm downloaded updated local sandbox artifacts, but the loaded harness image is still stale and will be refreshed on the next workspace launch"
                            .to_string();
                    let snapshot = StartupPrewarmSnapshot {
                        state: StartupPrewarmState::Skipped,
                        target_image: image.clone(),
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
                    event.meta = Some(json!({"image": image, "reason": message}));
                    self.ops_events.emit(event);
                    return;
                }
                let needs_prewarm = !machine_ready || !image_present;
                let last_success_at = if machine_ready && image_present {
                    let metadata = StartupPrewarmMetadata {
                        image_ref: image.clone(),
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
                    target_image: image,
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
                event.meta = Some(json!({"image": image, "error": message}));
                self.ops_events.emit(event);

                let snapshot = StartupPrewarmSnapshot {
                    state: StartupPrewarmState::Error,
                    target_image: image,
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
    }

    async fn compute_prewarm_gate(&self, image: &str) -> Result<PrewarmGate> {
        let metadata = read_prewarm_metadata(&self.data_root).await?;
        let (machine_ready, image_present) = self.startup_runtime_state(image).await?;
        let bundled_image_fingerprint = bundled_image_fingerprint(image).await?;

        let image_ref_changed = metadata
            .as_ref()
            .map(|meta| meta.image_ref != image)
            .unwrap_or(false);
        let bundled_image_digest_changed = metadata
            .as_ref()
            .map(|meta| meta.bundled_image_fingerprint != bundled_image_fingerprint)
            .unwrap_or(false);

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

    pub(crate) async fn startup_runtime_state(&self, image: &str) -> Result<(bool, bool)> {
        let machine_ready = normalize_podman_engine_ready_for_gate(
            harness_runtime::podman_engine_ready(&self.data_root).await,
        )?;
        let image_present = if machine_ready {
            harness_runtime::container_image_present(&self.data_root, image).await?
        } else {
            false
        };
        Ok((machine_ready, image_present))
    }

    async fn startup_prewarm_runtime(&self, exec: &ExecutionSettings) -> Result<()> {
        let _artifact_warmup = self.harness.begin_prewarm_artifact_activity();
        self.prewarm
            .ensure_scope(exec, RuntimePrewarmScope::Runtime, None)
            .await
    }

    async fn configured_startup_image(&self) -> Result<String> {
        let db_path = self.data_root.join("db").join("db.sqlite");
        let store = Store::open_sqlite(&db_path, None)
            .await
            .context("open global settings store")?;
        let loaded = crate::settings::load_settings(&store)
            .await
            .context("load execution settings")?;
        store.close().await;
        let exec = loaded.execution.unwrap_or_default();
        Ok(harness_runtime::resolve_container_image(&exec.container))
    }

    pub(crate) async fn refresh_startup_prewarm_metadata_after_successful_container_launch(
        &self,
        image: &str,
    ) {
        let startup_image = match self.configured_startup_image().await {
            Ok(startup_image) => startup_image,
            Err(err) => {
                tracing::warn!(
                    image,
                    error = %format_error_chain(&err),
                    "failed to resolve configured startup image after successful launch; leaving startup prewarm metadata unchanged"
                );
                return;
            }
        };
        if startup_image != image {
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
                    .is_some_and(|metadata| metadata.image_ref != image);
                (metadata.is_none(), image_ref_changed)
            }
            Err(err) => {
                tracing::warn!(
                    image,
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

        let bundled_image_fingerprint = match bundled_image_fingerprint(image).await {
            Ok(fingerprint) => fingerprint,
            Err(err) => {
                tracing::warn!(
                    image,
                    error = %format_error_chain(&err),
                    "failed to compute bundled image fingerprint after successful launch; leaving startup prewarm metadata unchanged"
                );
                return;
            }
        };

        let ready_at = format_ts(Utc::now());
        let metadata = StartupPrewarmMetadata {
            image_ref: image.to_string(),
            bundled_image_fingerprint,
            ready_at: ready_at.clone(),
        };
        if let Err(err) = write_prewarm_metadata(&self.data_root, &metadata).await {
            tracing::warn!(
                image,
                error = %format_error_chain(&err),
                "failed to persist refreshed startup prewarm metadata after successful launch"
            );
            return;
        }

        let mut inner = self.inner.lock().await;
        inner.startup.target_image = image.to_string();
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

    async fn set_startup_snapshot(&self, snapshot: StartupPrewarmSnapshot) {
        let mut inner = self.inner.lock().await;
        inner.startup = snapshot;
    }
}
