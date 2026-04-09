use super::*;

impl HarnessRuntimeManager {
    pub(super) async fn ensure_container(
        &self,
        workspace: &Workspace,
        worktree: Option<&Worktree>,
        settings: &ContainerExecutionSettings,
        daemon_host: &str,
        daemon_port: u16,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<HarnessContainer> {
        self.ensure_container_machine_ready(settings, observer)
            .await?;
        let substrate = UbuntuSandboxSubstrate::from_runtime_kind(settings.runtime.clone());
        substrate.ensure_enabled()?;
        if substrate.is_shared_vm_backed() {
            let sandbox_instance_id =
                ctx_core::models::sandbox_instance_id_for_workspace(workspace.id);
            let record = SharedSubstrateLifecycleManager::new(&self.data_root)
                .ensure_workspace_runtime_ready(sandbox_instance_id, settings, observer)
                .await?;
            self.emit_substrate_lifecycle_ops_event(
                &record,
                "container_prepare",
                Some(workspace.id.0.to_string()),
            );
        }
        self.ensure_container_after_machine_ready(EnsureContainerRequest {
            workspace,
            worktree,
            settings,
            daemon_host,
            daemon_port,
            observer,
            readiness: ContainerReadinessState::MachineReady,
        })
        .await
    }

    pub(super) async fn ensure_container_after_machine_ready(
        &self,
        request: EnsureContainerRequest<'_>,
    ) -> Result<HarnessContainer> {
        let EnsureContainerRequest {
            workspace,
            worktree,
            settings,
            daemon_host,
            daemon_port,
            observer,
            readiness,
        } = request;
        let name = format!("ctx-harness-{}", workspace.id.0);
        let image = resolve_container_image(settings);
        #[cfg(test)]
        eprintln!(
            "ensure_container_after_machine_ready: workspace={:?} readiness={:?}",
            workspace.id, readiness
        );
        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            #[cfg(test)]
            eprintln!("ensure_container_after_machine_ready: before ensure_workspace_volume");
            observe_log(
                observer,
                HarnessSetupPhase::ContainerCheck,
                HarnessSetupLogLevel::Info,
                "ensuring workspace volume for disk-isolated mode",
            );
            let _ = ensure_workspace_volume(&self.data_root, workspace.id).await?;
        }
        let mount_plan = build_mounts(&self.data_root, workspace, worktree, settings);
        let mut containers = self.containers.lock().await;
        let mut recreate = false;
        observe_phase(
            observer,
            HarnessSetupPhase::ContainerCheck,
            "checking existing workspace container",
        );
        if let Some(container) = containers.get(&workspace.id).cloned() {
            match cached_container_action(&container, settings, &mount_plan.external_mounts) {
                CachedContainerAction::Reuse => {
                    let exists = container_exists(&self.data_root, &name).await?;
                    let running = if exists {
                        container_running(&self.data_root, &name)
                            .await?
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if exists && running {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ContainerCheck,
                            HarnessSetupLogLevel::Info,
                            "container already ready in runtime cache",
                        );
                        return Ok(container);
                    }
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        if exists {
                            "runtime cache entry stale; workspace container is stopped and will be restarted"
                        } else {
                            "runtime cache entry stale; workspace container is missing and will be recreated"
                        },
                    );
                    containers.remove(&workspace.id);
                }
                CachedContainerAction::Reconfigure => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        "container network policy changed; reconfiguring",
                    );
                }
                CachedContainerAction::Recreate => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        "container configuration changed; recreating",
                    );
                    recreate = true;
                }
            }
        }

        if recreate {
            observe_phase(
                observer,
                HarnessSetupPhase::ContainerStartOrCreate,
                "recreating workspace container",
            );
            if let Ok(mut cmd) = sandbox_container_command(&self.data_root) {
                cmd.arg("rm").arg("-f").arg(&name);
                let _ = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await;
            }
        }

        let mut recreate_for_terminal_contract = false;
        loop {
            let exists = if recreate || recreate_for_terminal_contract {
                false
            } else {
                #[cfg(test)]
                eprintln!("ensure_container_after_machine_ready: before container_exists");
                container_exists(&self.data_root, &name).await?
            };
            #[cfg(test)]
            eprintln!("ensure_container_after_machine_ready: exists={exists}");

            if exists {
                #[cfg(test)]
                eprintln!("ensure_container_after_machine_ready: before container_running");
                let running = container_running(&self.data_root, &name)
                    .await?
                    .unwrap_or(false);
                #[cfg(test)]
                eprintln!("ensure_container_after_machine_ready: running={running}");
                if !running {
                    observe_phase(
                        observer,
                        HarnessSetupPhase::ContainerStartOrCreate,
                        "starting existing workspace container",
                    );
                    let mut cmd = sandbox_container_command(&self.data_root)?;
                    cmd.arg("start").arg(&name);
                    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let combined = format!("{stderr}\n{stdout}").trim().to_string();
                        if combined.is_empty() {
                            anyhow::bail!(
                                "container start failed for {name} (status: {})",
                                output.status
                            );
                        }
                        anyhow::bail!("container start failed for {name}: {combined}");
                    }
                } else {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        "workspace container already running",
                    );
                }
            } else {
                let requires_front_loaded_image_readiness = !(settings.runtime
                    == ContainerRuntimeKind::NativeContainer
                    && readiness == ContainerReadinessState::RuntimeReady);
                if requires_front_loaded_image_readiness {
                    self.ensure_container_image_ready(settings, observer)
                        .await?;
                }
                observe_phase(
                    observer,
                    HarnessSetupPhase::ContainerStartOrCreate,
                    "creating workspace container",
                );
                let mut cmd = sandbox_container_command(&self.data_root)?;
                cmd.arg("run").arg("-d").arg("--name").arg(&name);
                cmd.arg("--hostname")
                    .arg(workspace_container_hostname(workspace));
                if should_use_keep_id_userns() {
                    cmd.arg("--userns=keep-id");
                }
                if let Some(user) = container_user() {
                    cmd.arg("--user").arg(user);
                }
                append_sandbox_container_launch_network_args(&mut cmd, settings);
                for mount in &mount_plan.mounts {
                    cmd.arg("--mount").arg(mount);
                }
                cmd.arg(&image);
                cmd.arg("/bin/sh")
                    .arg("-c")
                    .arg("while true; do sleep 100000; done");
                let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let combined = format!("{stderr}\n{stdout}").trim().to_string();
                    let combined_lower = combined.to_ascii_lowercase();
                    let can_adopt_existing = combined_lower.contains("name-store error")
                        || combined_lower.contains("already used by id");
                    if can_adopt_existing && container_exists(&self.data_root, &name).await? {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ContainerStartOrCreate,
                            HarnessSetupLogLevel::Warn,
                            "container create reported an existing name; adopting the existing workspace container",
                        );
                        let running = container_running(&self.data_root, &name)
                            .await?
                            .unwrap_or(false);
                        if !running {
                            observe_log(
                                observer,
                                HarnessSetupPhase::ContainerStartOrCreate,
                                HarnessSetupLogLevel::Info,
                                "adopted workspace container is stopped; starting it",
                            );
                            let mut start = sandbox_container_command(&self.data_root)?;
                            start.arg("start").arg(&name);
                            let output =
                                command_output_with_timeout(start, SANDBOX_OP_TIMEOUT).await?;
                            if !output.status.success() {
                                let stderr =
                                    String::from_utf8_lossy(&output.stderr).trim().to_string();
                                let stdout =
                                    String::from_utf8_lossy(&output.stdout).trim().to_string();
                                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                                if combined.is_empty() {
                                    anyhow::bail!(
                                        "container start failed for {name} (status: {})",
                                        output.status
                                    );
                                }
                                anyhow::bail!("container start failed for {name}: {combined}");
                            }
                        }
                    } else if combined.is_empty() {
                        anyhow::bail!(
                            "container run failed for {name} (status: {})",
                            output.status
                        );
                    } else {
                        anyhow::bail!("container run failed for {name}: {combined}");
                    }
                }
            }

            match sync_container_terminal_identity(&self.data_root, &name).await {
                Ok(()) => break,
                Err(err) if container_terminal_identity_missing_sudo(&err) => {
                    if recreate_for_terminal_contract {
                        return Err(err.context(
                            "workspace container still lacks terminal sudo support after recreation",
                        ));
                    }
                    if is_default_container_image(&image) {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ImageLoad,
                            HarnessSetupLogLevel::Info,
                            "reloading the default harness image to apply the terminal identity contract",
                        );
                        force_reload_default_container_image(&self.data_root, observer).await?;
                    }
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerStartOrCreate,
                        HarnessSetupLogLevel::Info,
                        "workspace container predates the terminal identity contract; recreating",
                    );
                    let mut cmd = sandbox_container_command(&self.data_root)?;
                    cmd.arg("rm").arg("-f").arg(&name);
                    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let combined = format!("{stderr}\n{stdout}").trim().to_string();
                        if combined.is_empty() {
                            anyhow::bail!(
                                "container rm failed for {name} while refreshing terminal identity contract (status: {})",
                                output.status
                            );
                        }
                        anyhow::bail!(
                            "container rm failed for {name} while refreshing terminal identity contract: {combined}"
                        );
                    }
                    recreate_for_terminal_contract = true;
                }
                Err(err) => return Err(err),
            }
        }

        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            #[cfg(test)]
            eprintln!(
                "ensure_container_after_machine_ready: before verify_disk_isolated_container_mounts"
            );
            verify_disk_isolated_container_mounts(&self.data_root, workspace, &name).await?;
        }

        #[cfg(test)]
        eprintln!("ensure_container_after_machine_ready: before apply_container_network_policy");
        observe_phase(
            observer,
            HarnessSetupPhase::RuntimeNetworkSetup,
            "configuring container network policy",
        );
        let egress_guard = apply_container_network_policy(
            &self.data_root,
            workspace.id,
            &name,
            settings,
            daemon_host,
            daemon_port,
        )
        .await?
        .egress_guard;
        #[cfg(test)]
        eprintln!("ensure_container_after_machine_ready: after apply_container_network_policy");
        observe_log(
            observer,
            HarnessSetupPhase::RuntimeNetworkSetup,
            HarnessSetupLogLevel::Info,
            "container network policy configured",
        );
        let container = HarnessContainer {
            name: name.clone(),
            mount_mode: settings.mount_mode.clone(),
            network_mode: settings.network_mode.clone(),
            allowlist: settings.allowlist.clone(),
            external_mounts: mount_plan.external_mounts,
            egress_guard,
        };
        containers.insert(workspace.id, container.clone());
        Ok(container)
    }
}
