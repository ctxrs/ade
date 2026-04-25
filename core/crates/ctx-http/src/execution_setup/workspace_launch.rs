use super::*;

impl ExecutionSetupCoordinator {
    pub(super) async fn run_workspace_launch(
        self: Arc<Self>,
        job: Arc<LaunchJob>,
        workspace: Workspace,
        settings: ExecutionSettings,
        daemon_url: String,
    ) {
        #[cfg(test)]
        eprintln!("run_workspace_launch: begin workspace={:?}", workspace.id);
        let launch_started = std::time::Instant::now();
        let observer = LaunchObserver {
            coordinator: Arc::clone(&self),
            job: Arc::clone(&job),
        };
        let is_host_mode = matches!(settings.mode, ExecutionMode::Host);
        let run_result = if is_host_mode {
            Ok(())
        } else {
            async {
                // Any compatible runtime prewarm can still save duplicate image work.
                // We check launch readiness after the join before deciding which launch path
                // to take.
                let join_shared_runtime = self.prewarm.runtime_is_running(&settings, false).await;
                #[cfg(test)]
                eprintln!("run_workspace_launch: join_shared_runtime={join_shared_runtime}");

                if join_shared_runtime {
                    let reusable_container_exists = self
                        .harness
                        .workspace_container_exists(workspace.id)
                        .await
                        .context("failed to probe existing workspace container")?;
                    #[cfg(test)]
                    eprintln!(
                        "run_workspace_launch: reusable_container_exists={reusable_container_exists}"
                    );
                    if reusable_container_exists {
                        #[cfg(test)]
                        eprintln!("run_workspace_launch: ensure_workspace_container_with_observer");
                        return self
                            .harness
                            .ensure_workspace_container_with_observer(
                                &workspace,
                                &settings,
                                &daemon_url,
                                Some(&observer),
                            )
                            .await
                            .context("container runtime failed");
                    }

                    let _runtime_activity = self.harness.begin_runtime_operation();
                    self.harness
                        .ensure_container_machine_ready(&settings.container, Some(&observer))
                        .await
                        .context("sandbox runtime unavailable and execution mode is sandbox")?;

                    let joined_shared_runtime = match self
                        .prewarm
                        .attach_runtime_if_running(&settings, false, Some(&observer))
                        .await
                    {
                        Ok(joined) => joined,
                        Err(err) => {
                            observer.on_log(
                                HarnessSetupPhase::MachineStartOrInit,
                                HarnessSetupLogLevel::Warn,
                                &format!(
                                    "shared runtime warmup failed, continuing with direct launch: {}",
                                    format_error_chain(&err)
                                ),
                            );
                            false
                        }
                    };

                    if joined_shared_runtime {
                        let launch_ready = ctx_harness_runtime::selected_runtime_launch_ready(
                            &self.data_root,
                            &settings.container,
                        )
                        .await
                        .context("failed to inspect container runtime after shared warmup")?;
                        if launch_ready {
                            return self
                                .harness
                                .ensure_workspace_container_after_runtime_ready_with_observer(
                                    &workspace,
                                    &settings,
                                    &daemon_url,
                                    Some(&observer),
                                )
                                .await
                                .context("container runtime failed");
                        }
                    }

                    self.harness
                        .ensure_workspace_container_after_machine_ready_with_observer(
                            &workspace,
                            &settings,
                            &daemon_url,
                            Some(&observer),
                        )
                        .await
                        .context("container runtime failed")
                } else {
                    #[cfg(test)]
                    eprintln!("run_workspace_launch: direct ensure_workspace_container_with_observer");
                    self.harness
                        .ensure_workspace_container_with_observer(
                            &workspace,
                            &settings,
                            &daemon_url,
                            Some(&observer),
                        )
                        .await
                        .context("container runtime failed")
                }
            }
            .await
        };

        match run_result {
            Ok(()) => {
                #[cfg(test)]
                eprintln!("run_workspace_launch: success");
                if !matches!(settings.mode, ExecutionMode::Host) {
                    self.refresh_startup_prewarm_metadata_after_successful_container_launch(
                        &settings.container,
                    )
                    .await;
                    self.emit_phase(
                        &job,
                        HarnessSetupPhase::Ready,
                        ctx_harness_runtime::workspace_launch_ready_message(
                            &settings.container.runtime,
                        ),
                    );
                }
                let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "ready");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "ready");
            }
            Err(err) => {
                #[cfg(test)]
                eprintln!("run_workspace_launch: error={err:#}");
                let message = format_error_chain(&err);
                let phase = job
                    .current_phase()
                    .unwrap_or(HarnessSetupPhase::ContainerStartOrCreate);
                self.emit_log(&job, phase, HarnessSetupLogLevel::Error, &message);
                let terminal =
                    job.mark_terminal(ExecutionLaunchState::Error, Some(message.clone()));
                if let Some(completed) = terminal.completed_phase {
                    self.record_phase_metric(completed.phase, completed.elapsed_ms, "error");
                }
                let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchError {
                    snapshot: terminal.snapshot.clone(),
                });
                self.record_launch_metric(launch_started.elapsed().as_millis() as u64, "error");

                let mut event = OpsEvent::new("error", "execution.launch_error");
                event.meta = Some(json!({
                    "job_id": terminal.snapshot.job_id,
                    "workspace_id": terminal.snapshot.workspace_id,
                    "phase": terminal.snapshot.current_phase,
                    "error": message,
                }));
                self.ops_events.emit(event);
            }
        }

        self.clear_running_launch(workspace.id, &job.job_id).await;
    }

    async fn clear_running_launch(&self, workspace_id: WorkspaceId, job_id: &str) {
        let mut inner = self.inner.lock().await;
        if inner
            .running_launch_by_workspace
            .get(&workspace_id)
            .map(String::as_str)
            == Some(job_id)
        {
            inner.running_launch_by_workspace.remove(&workspace_id);
        }
    }
}
