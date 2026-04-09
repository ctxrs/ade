use super::*;

impl HarnessRuntimeManager {
    pub fn new(data_root: PathBuf) -> Self {
        Self::new_with_ops_events(
            data_root.clone(),
            crate::ops_events::OpsEvents::new(data_root),
        )
    }

    pub fn new_with_ops_events(
        data_root: PathBuf,
        ops_events: crate::ops_events::OpsEvents,
    ) -> Self {
        Self {
            data_root,
            containers: Mutex::new(HashMap::new()),
            last_activity: StdMutex::new(Instant::now()),
            active_runtime_operations: AtomicUsize::new(0),
            active_prewarm_artifact_operations: AtomicUsize::new(0),
            ops_events,
        }
    }

    pub(super) fn emit_substrate_lifecycle_ops_event(
        &self,
        record: &SubstrateLifecycleRecord,
        source: &'static str,
        workspace_id: Option<String>,
    ) {
        let event = crate::ops_events::substrate_lifecycle_observed_event(
            record,
            crate::ops_events::SubstrateLifecycleOpsEventContext {
                source,
                workspace_id,
            },
        );
        self.ops_events.emit(event);
    }

    pub(crate) fn note_runtime_activity(&self) {
        let mut last_activity = match self.last_activity.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *last_activity = Instant::now();
    }

    #[cfg(test)]
    pub(crate) fn runtime_idle_for(&self) -> Duration {
        let last_activity = match self.last_activity.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        last_activity.elapsed()
    }

    pub(crate) fn begin_runtime_operation(&self) -> RuntimeOperationGuard<'_> {
        self.note_runtime_activity();
        self.active_runtime_operations
            .fetch_add(1, Ordering::SeqCst);
        RuntimeOperationGuard { manager: self }
    }

    pub(crate) fn begin_prewarm_artifact_activity(&self) -> PrewarmArtifactActivityGuard<'_> {
        self.note_runtime_activity();
        self.active_prewarm_artifact_operations
            .fetch_add(1, Ordering::SeqCst);
        PrewarmArtifactActivityGuard { manager: self }
    }

    pub async fn stats(&self) -> HarnessRuntimeStats {
        let containers = self.containers.lock().await;
        let mut container_allowlist_entries = 0;
        let mut container_external_mounts = 0;
        let mut container_egress_guards = 0;
        for container in containers.values() {
            container_allowlist_entries += container.allowlist.len();
            container_external_mounts += container.external_mounts.len();
            if container.egress_guard {
                container_egress_guards += 1;
            }
        }
        HarnessRuntimeStats {
            container_count: containers.len(),
            container_allowlist_entries,
            container_external_mounts,
            container_egress_guards,
        }
    }

    pub(crate) async fn save_or_stop_selected_shared_substrate(
        &self,
    ) -> Result<Option<SubstrateLifecycleRecord>> {
        if !matches!(
            selected_sandbox_command_backend(&self.data_root),
            Ok(SandboxCommandBackend::SharedVmContainer)
        ) {
            return Ok(None);
        }

        let settings = ContainerExecutionSettings {
            runtime: ContainerRuntimeKind::SharedVmContainer,
            ..ContainerExecutionSettings::default()
        };
        let record = SharedSubstrateLifecycleManager::new(&self.data_root)
            .save_or_stop_shared_runtime(&settings)
            .await?;
        self.emit_substrate_lifecycle_ops_event(&record, "shared_substrate_save_or_stop", None);
        Ok(Some(record))
    }

    pub async fn prepare(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<HarnessExecutionPlan> {
        let substrate =
            UbuntuSandboxSubstrate::from_runtime_kind(settings.container.runtime.clone());
        substrate.ensure_enabled()?;
        let mut env_overrides = HashMap::new();
        env_overrides.insert(
            "CTX_DATA_ROOT_HOST".to_string(),
            self.data_root.to_string_lossy().to_string(),
        );
        env_overrides.insert(CTX_HARNESS_RUNTIME_KIND_ENV.to_string(), "host".to_string());
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::Host,
                env_overrides,
            });
        }
        let _activity = self.begin_runtime_operation();
        if !substrate.is_shared_vm_backed() {
            let sandbox_cli_bin = sandbox_cli_invocation(&self.data_root)
                .context("sandbox container CLI unavailable and execution mode is sandbox")?
                .bin;
            env_overrides.insert(
                CTX_HARNESS_SANDBOX_CLI_PATH_ENV.to_string(),
                sandbox_cli_bin.to_string_lossy().to_string(),
            );
        }
        let proxy_host = if substrate.is_shared_vm_backed() {
            AVF_GUEST_HOST_GATEWAY
        } else {
            "host.containers.internal"
        };
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let container = self
            .ensure_container(
                workspace,
                Some(worktree),
                &settings.container,
                proxy_host,
                daemon_port,
                None,
            )
            .await
            .map_err(|err| anyhow::anyhow!("container runtime failed: {err:#}"))?;

        let container_data_root = container_data_root(&self.data_root, workspace.id);
        tokio::fs::create_dir_all(&container_data_root).await.ok();
        env_overrides.insert(
            "CTX_DATA_ROOT".to_string(),
            container_data_root.to_string_lossy().to_string(),
        );
        env_overrides.insert(
            CTX_HARNESS_RUNTIME_KIND_ENV.to_string(),
            substrate.runtime_kind_env_value().to_string(),
        );
        env_overrides.insert(CTX_HARNESS_LINUX_SANDBOX_ENV.to_string(), "1".to_string());

        let daemon_url = if substrate.is_shared_vm_backed() {
            resolve_daemon_url_for_avf_guest(daemon_url).await?
        } else {
            rewrite_daemon_url_for_container(daemon_url, proxy_host)
        };
        env_overrides.insert("CTX_DAEMON_URL".to_string(), daemon_url);

        env_overrides.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            container.name.clone(),
        );
        let guest_workspace_root =
            ctx_worktree_data_plane::live_workspace_root_for_mode(workspace, ExecutionMode::Sandbox);
        let guest_worktree_root = ctx_worktree_data_plane::live_worktree_root_for_mode(
            workspace,
            worktree,
            ExecutionMode::Sandbox,
        );
        env_overrides.insert(
            "CTX_HARNESS_HOST_WORKTREE_ROOT".to_string(),
            worktree.root_path.clone(),
        );
        env_overrides.insert(
            "CTX_HARNESS_GUEST_WORKTREE_ROOT".to_string(),
            guest_worktree_root.to_string_lossy().to_string(),
        );
        env_overrides.insert(
            "CTX_HARNESS_GUEST_WORKSPACE_ROOT".to_string(),
            guest_workspace_root.to_string_lossy().to_string(),
        );
        if let Some(user) = container_user() {
            env_overrides.insert("CTX_HARNESS_CONTAINER_USER".to_string(), user);
        }

        if substrate.is_shared_vm_backed() {
            let sandbox_instance_id =
                ctx_core::models::sandbox_instance_id_for_workspace(workspace.id);
            let workspace_vm = SharedVmLifecycleOrchestrator::new(&self.data_root)
                .workspace_runtime_state(sandbox_instance_id)?;
            env_overrides.insert(
                "CTX_AVF_WORKSPACE_VM_ROOT".to_string(),
                workspace_vm.vm_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                avf_linux_vm::AVF_LINUX_HELPER_PATH_ENV.to_string(),
                avf_linux_helper_path()?.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                CTX_AVF_HOST_DATA_ROOT_ENV.to_string(),
                self.data_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                "CTX_AVF_WORKSPACE_VM_DATA_ROOT".to_string(),
                avf_linux_workspace_vm_data_root(&self.data_root, workspace.id)
                    .to_string_lossy()
                    .to_string(),
            );
            env_overrides.insert(
                CTX_AVF_WORKSPACE_ID_ENV.to_string(),
                workspace.id.0.to_string(),
            );
            env_overrides.insert(
                CTX_AVF_WORKTREE_ID_ENV.to_string(),
                worktree.id.0.to_string(),
            );
            env_overrides.insert(
                CTX_AVF_HOST_WORKTREE_ROOT_ENV.to_string(),
                worktree.root_path.clone(),
            );
            env_overrides.insert(
                "CTX_AVF_GUEST_WORKTREE_ROOT".to_string(),
                guest_worktree_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                CTX_AVF_REAL_GUEST_EXEC_ENV.to_string(),
                if workspace_vm.simulated { "0" } else { "1" }.to_string(),
            );
            if let Some(log_path) = workspace_vm.log_path.as_ref() {
                env_overrides.insert(
                    "CTX_AVF_WORKSPACE_VM_LOG".to_string(),
                    log_path.to_string_lossy().to_string(),
                );
            }
        }

        Ok(HarnessExecutionPlan {
            runtime: if substrate.is_shared_vm_backed() {
                HarnessRuntimeKind::SharedVmContainer
            } else {
                HarnessRuntimeKind::NativeContainer {
                    name: container.name,
                }
            },
            env_overrides,
        })
    }

    pub async fn ensure_workspace_container(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<()> {
        self.ensure_workspace_container_with_observer(workspace, settings, daemon_url, None)
            .await
    }

    pub async fn ensure_workspace_container_for_worktree(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<()> {
        self.ensure_workspace_container_for_worktree_with_observer(
            workspace, worktree, settings, daemon_url, None,
        )
        .await
    }

    pub async fn ensure_workspace_container_for_worktree_with_observer(
        &self,
        workspace: &Workspace,
        _worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        self.ensure_workspace_container_with_observer(workspace, settings, daemon_url, observer)
            .await
    }

    pub async fn ensure_workspace_container_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        #[cfg(test)]
        eprintln!(
            "ensure_workspace_container_with_observer: workspace={:?} mode={:?}",
            workspace.id, settings.mode
        );
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        let _activity = self.begin_runtime_operation();
        #[cfg(test)]
        eprintln!(
            "ensure_workspace_container_with_observer: before ensure_container_machine_ready"
        );
        self.ensure_container_machine_ready(&settings.container, observer)
            .await
            .context("local sandbox runtime is unavailable")?;
        #[cfg(test)]
        eprintln!("ensure_workspace_container_with_observer: before after_machine_ready");
        self.ensure_workspace_container_after_machine_ready_with_observer(
            workspace, settings, daemon_url, observer,
        )
        .await
    }

    pub(crate) async fn ensure_workspace_container_after_machine_ready_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.ensure_workspace_container_after_readiness_with_observer(
            workspace,
            settings,
            daemon_url,
            observer,
            ContainerReadinessState::MachineReady,
        )
        .await
    }

    async fn ensure_workspace_container_after_readiness_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
        readiness: ContainerReadinessState,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        let substrate =
            UbuntuSandboxSubstrate::from_runtime_kind(settings.container.runtime.clone());
        substrate.ensure_enabled()?;
        let proxy_host = if substrate.is_shared_vm_backed() {
            let sandbox_instance_id =
                ctx_core::models::sandbox_instance_id_for_workspace(workspace.id);
            let record = SharedSubstrateLifecycleManager::new(&self.data_root)
                .ensure_workspace_runtime_ready(sandbox_instance_id, &settings.container, observer)
                .await
                .context("shared VM substrate is unavailable")?;
            self.emit_substrate_lifecycle_ops_event(
                &record,
                "workspace_container_after_readiness",
                Some(workspace.id.0.to_string()),
            );
            AVF_GUEST_HOST_GATEWAY
        } else {
            "host.containers.internal"
        };
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let _ = self
            .ensure_container_after_machine_ready(EnsureContainerRequest {
                workspace,
                worktree: None,
                settings: &settings.container,
                daemon_host: proxy_host,
                daemon_port,
                observer,
                readiness,
            })
            .await?;
        Ok(())
    }

    pub async fn ensure_workspace_container_after_runtime_ready_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        let _activity = self.begin_runtime_operation();
        self.ensure_workspace_container_after_readiness_with_observer(
            workspace,
            settings,
            daemon_url,
            observer,
            ContainerReadinessState::RuntimeReady,
        )
        .await
    }

    pub(crate) async fn ensure_container_machine_ready(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let substrate = UbuntuSandboxSubstrate::from_runtime_kind(settings.runtime.clone());
        substrate.ensure_enabled()?;
        if substrate.is_shared_vm_backed() {
            SharedVmLifecycleOrchestrator::new(&self.data_root)
                .prefetch_runtime(settings, observer)
                .await?;
            return Ok(());
        }
        observe_phase(
            observer,
            HarnessSetupPhase::MachineCheck,
            "checking container runtime",
        );
        if sandbox_engine_ready(&self.data_root).await.unwrap_or(false) {
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Info,
                "local sandbox runtime is already reachable",
            );
            return Ok(());
        }
        if sandbox_cli_invocation(&self.data_root).is_err() {
            let bootstrap = linux_sandbox_runtime_status(&self.data_root).await?;
            anyhow::bail!("{}", bootstrap.message);
        }
        let bootstrap = linux_sandbox_runtime_status(&self.data_root).await?;
        anyhow::bail!("{}", bootstrap.message)
    }

    pub(crate) async fn workspace_container_exists(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        let name = workspace_container_name(workspace_id);
        match container_exists(&self.data_root, &name).await {
            Ok(exists) => Ok(exists),
            Err(err) => {
                if sandbox_engine_ready(&self.data_root).await.unwrap_or(false) {
                    Err(err)
                } else {
                    Ok(false)
                }
            }
        }
    }

    pub(super) async fn ensure_container_image_ready(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let image = resolve_container_image(settings);
        observe_phase(
            observer,
            HarnessSetupPhase::ImageCheck,
            "checking harness image availability",
        );
        if container_image_present(&self.data_root, &image).await? {
            observe_log(
                observer,
                HarnessSetupPhase::ImageCheck,
                HarnessSetupLogLevel::Info,
                "harness image already present",
            );
            return Ok(());
        }
        observe_phase(
            observer,
            HarnessSetupPhase::ImageLoad,
            "loading harness image into local sandbox runtime",
        );
        ensure_container_image_available(&self.data_root, &image, observer).await
    }

    pub async fn container_status(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<HarnessContainerStatus>> {
        let name = format!("ctx-harness-{}", workspace_id.0);
        let container_present = match container_exists(&self.data_root, &name).await {
            Ok(exists) => exists,
            Err(err) => {
                if err
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("sandbox container cli unavailable")
                {
                    false
                } else {
                    return Err(err);
                }
            }
        };
        if container_present {
            let running = container_running(&self.data_root, &name)
                .await?
                .unwrap_or(false);
            let container = {
                let containers = self.containers.lock().await;
                containers.get(&workspace_id).cloned()
            };
            let (known, mount_mode, network_mode, allowlist, egress_guard) =
                if let Some(container) = container {
                    (
                        true,
                        Some(container.mount_mode),
                        Some(container.network_mode),
                        container.allowlist,
                        Some(container.egress_guard),
                    )
                } else {
                    (false, None, None, Vec::new(), None)
                };
            return Ok(Some(HarnessContainerStatus {
                name,
                running,
                known,
                mount_mode,
                network_mode,
                allowlist,
                egress_guard,
            }));
        }
        Ok(None)
    }

    pub async fn stop_container(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        let name = format!("ctx-harness-{}", workspace_id.0);
        let container_present = match container_exists(&self.data_root, &name).await {
            Ok(exists) => exists,
            Err(err) => {
                if err
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("sandbox container cli unavailable")
                {
                    false
                } else {
                    return Err(err);
                }
            }
        };
        if container_present {
            let mut containers = self.containers.lock().await;
            containers.remove(&workspace_id);
            let mut cmd = sandbox_container_command(&self.data_root)?;
            cmd.arg("rm").arg("-f").arg(&name);
            let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
            if output.status.success() {
                return Ok(true);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                if combined.is_empty() {
                    anyhow::bail!("container rm failed for {name} (status: {})", output.status);
                }
                anyhow::bail!("container rm failed for {name}: {combined}");
            }
        }

        let mut containers = self.containers.lock().await;
        containers.remove(&workspace_id);
        Ok(false)
    }

    pub async fn remove_workspace_volume(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        // Best-effort cleanup: callers (e.g. workspace deletion) may ignore failures.
        let name = format!("ctx-ws-{}", workspace_id.0);
        let mut inspect = sandbox_container_command(&self.data_root)?;
        inspect.arg("volume").arg("inspect").arg(&name);
        let out = command_output_with_timeout(inspect, SANDBOX_OP_TIMEOUT).await?;
        if !out.status.success() {
            return Ok(false);
        }

        let mut cmd = sandbox_container_command(&self.data_root)?;
        cmd.arg("volume").arg("rm").arg("-f").arg(&name);
        let out = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
        if out.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let combined = format!("{stderr}\n{stdout}").trim().to_string();
            if combined.is_empty() {
                anyhow::bail!(
                    "container volume rm failed for {name} (status: {})",
                    out.status
                );
            }
            anyhow::bail!("container volume rm failed for {name}: {combined}");
        }
    }
}

pub(crate) struct RuntimeOperationGuard<'a> {
    manager: &'a HarnessRuntimeManager,
}

impl Drop for RuntimeOperationGuard<'_> {
    fn drop(&mut self) {
        self.manager.note_runtime_activity();
        self.manager
            .active_runtime_operations
            .fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) struct PrewarmArtifactActivityGuard<'a> {
    manager: &'a HarnessRuntimeManager,
}

impl Drop for PrewarmArtifactActivityGuard<'_> {
    fn drop(&mut self) {
        self.manager.note_runtime_activity();
        self.manager
            .active_prewarm_artifact_operations
            .fetch_sub(1, Ordering::SeqCst);
    }
}
