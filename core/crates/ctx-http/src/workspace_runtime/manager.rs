use super::*;

impl HarnessRuntimeManager {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            containers: Mutex::new(HashMap::new()),
            last_activity: StdMutex::new(Instant::now()),
            active_runtime_operations: AtomicUsize::new(0),
            active_prewarm_artifact_operations: AtomicUsize::new(0),
            reclaim_loop_started: AtomicBool::new(false),
        }
    }

    pub async fn prepare(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<HarnessExecutionPlan> {
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
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            let _activity = self.begin_runtime_operation();
            let (workspace_vm, guest_worktree_root, egress_guard) = self
                .ensure_avf_linux_workspace_worktree_ready_with_observer(
                    workspace, worktree, settings, daemon_url, None,
                )
                .await?;
            let avf_data_root = container_data_root(&self.data_root, workspace.id);
            tokio::fs::create_dir_all(&avf_data_root).await.ok();
            env_overrides.insert(
                "CTX_DATA_ROOT".to_string(),
                avf_data_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                CTX_HARNESS_RUNTIME_KIND_ENV.to_string(),
                "avf_linux_vm".to_string(),
            );
            env_overrides.insert(CTX_HARNESS_LINUX_SANDBOX_ENV.to_string(), "1".to_string());
            env_overrides.insert(
                "CTX_HARNESS_GUEST_WORKSPACE_ROOT".to_string(),
                CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            );
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
                "CTX_DAEMON_URL".to_string(),
                resolve_daemon_url_for_avf_guest(daemon_url).await?,
            );
            if let Some(log_path) = workspace_vm.log_path.as_ref() {
                env_overrides.insert(
                    "CTX_AVF_WORKSPACE_VM_LOG".to_string(),
                    log_path.to_string_lossy().to_string(),
                );
            }
            {
                let mut containers = self.containers.lock().await;
                containers.insert(
                    workspace.id,
                    HarnessContainer {
                        name: format!("ctx-avf-linux-vm-{}", workspace.id.0),
                        mount_mode: settings.container.mount_mode.clone(),
                        network_mode: settings.container.network_mode.clone(),
                        allowlist: settings.container.allowlist.clone(),
                        external_mounts: HashSet::new(),
                        egress_guard,
                    },
                );
            }
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::AvfLinuxVm,
                env_overrides,
            });
        }
        let _activity = self.begin_runtime_operation();
        let podman_bin = ensure_managed_podman_runtime(&self.data_root, None, None)
            .await
            .context("podman unavailable and execution mode is container")?;
        env_overrides.insert(
            PODMAN_PATH_ENV.to_string(),
            podman_bin.to_string_lossy().to_string(),
        );

        let proxy_host = "host.containers.internal";
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
            "podman_container".to_string(),
        );
        env_overrides.insert(CTX_HARNESS_LINUX_SANDBOX_ENV.to_string(), "1".to_string());

        let daemon_url = rewrite_daemon_url_for_container(daemon_url, proxy_host);
        env_overrides.insert("CTX_DAEMON_URL".to_string(), daemon_url);

        env_overrides.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            container.name.clone(),
        );
        if let Some(user) = container_user() {
            env_overrides.insert("CTX_HARNESS_CONTAINER_USER".to_string(), user);
        }

        Ok(HarnessExecutionPlan {
            runtime: HarnessRuntimeKind::Container {
                name: container.name,
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
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            self.ensure_avf_linux_workspace_worktree_ready_with_observer(
                workspace, worktree, settings, daemon_url, observer,
            )
            .await?;
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
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        let _activity = self.begin_runtime_operation();
        self.ensure_container_machine_ready(&settings.container, observer)
            .await
            .context("local sandbox runtime is unavailable")?;
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            ensure_avf_linux_workspace_vm_ready_with_observer(
                &self.data_root,
                workspace.id,
                &settings.container,
                observer,
            )
            .await
            .context("AVF Linux workspace VM is unavailable")?;
            return Ok(());
        }
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
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            ensure_avf_linux_workspace_vm_ready_with_observer(
                &self.data_root,
                workspace.id,
                &settings.container,
                observer,
            )
            .await
            .context("AVF Linux workspace VM is unavailable")?;
            return Ok(());
        }
        let proxy_host = "host.containers.internal";
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

    async fn ensure_avf_linux_workspace_worktree_ready_with_observer(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<(self::avf_linux_vm::AvfLinuxSharedVmState, PathBuf, bool)> {
        self.ensure_workspace_container_with_observer(workspace, settings, daemon_url, observer)
            .await?;
        if !matches!(
            settings.container.mount_mode,
            ContainerMountMode::DiskIsolated
        ) {
            anyhow::bail!(
                "AVF Linux VM host-mounted workspaces are not implemented yet; use disk-isolated mode"
            );
        }
        let workspace_vm = ensure_avf_linux_workspace_vm_ready_with_observer(
            &self.data_root,
            workspace.id,
            &settings.container,
            observer,
        )
        .await?;
        let guest_worktree = ensure_avf_linux_guest_worktree_from_host_copy(
            &self.data_root,
            workspace.id,
            worktree.id,
            Path::new(&workspace.root_path),
            &worktree.base_commit_sha,
            &avf_linux_branch_name_for_worktree(workspace, worktree),
            observer,
        )
        .await?;
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let egress_guard = apply_avf_linux_network_policy(
            &self.data_root,
            workspace.id,
            worktree.id,
            &guest_worktree.guest_root,
            &settings.container,
            AVF_GUEST_HOST_GATEWAY,
            daemon_port,
        )
        .await?
        .egress_guard;
        Ok((workspace_vm, guest_worktree.guest_root, egress_guard))
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
        if matches!(settings.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            prefetch_avf_linux_runtime_with_observer(&self.data_root, settings, observer).await?;
            return Ok(());
        }
        observe_phase(
            observer,
            HarnessSetupPhase::MachineCheck,
            "checking container runtime",
        );
        ensure_managed_podman_runtime(&self.data_root, observer, None).await?;
        if podman_engine_ready(&self.data_root).await.unwrap_or(false) {
            self.reconcile_running_podman_machine_memory(settings, observer)
                .await?;
            if !podman_engine_ready(&self.data_root).await.unwrap_or(false) {
                self.ensure_podman_machine_materialized(settings, observer)
                    .await?;
                ensure_podman_machine_running_with_observer(&self.data_root, observer).await?;
                return Ok(());
            }
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Info,
                "local sandbox runtime is already reachable",
            );
            return Ok(());
        }
        self.ensure_podman_machine_materialized(settings, observer)
            .await?;
        ensure_podman_machine_running_with_observer(&self.data_root, observer).await?;
        Ok(())
    }

    pub(crate) async fn workspace_container_exists(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        let name = workspace_container_name(workspace_id);
        match container_exists(&self.data_root, &name).await {
            Ok(exists) => Ok(exists),
            Err(err) => {
                if podman_engine_ready(&self.data_root).await.unwrap_or(false) {
                    Err(err)
                } else {
                    Ok(false)
                }
            }
        }
    }

    async fn ensure_container_image_ready(
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
        let podman_exists = match container_exists(&self.data_root, &name).await {
            Ok(exists) => exists,
            Err(err) => {
                if err
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("podman binary unavailable")
                {
                    false
                } else {
                    return Err(err);
                }
            }
        };
        if podman_exists {
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

        let state = match avf_linux_workspace_vm_state(&self.data_root, workspace_id) {
            Ok(state) => state,
            Err(_) => return Ok(None),
        };
        if matches!(
            state.state,
            avf_linux_vm::AvfLinuxSharedVmLifecycleState::Missing
        ) {
            return Ok(None);
        }
        let container = {
            let containers = self.containers.lock().await;
            containers.get(&workspace_id).cloned()
        };
        Ok(Some(HarnessContainerStatus {
            name: format!("ctx-avf-linux-vm-{}", workspace_id.0),
            running: matches!(
                state.state,
                avf_linux_vm::AvfLinuxSharedVmLifecycleState::Running
            ),
            known: true,
            mount_mode: container
                .as_ref()
                .map(|value| value.mount_mode.clone())
                .or(Some(ContainerMountMode::DiskIsolated)),
            network_mode: container.as_ref().map(|value| value.network_mode.clone()),
            allowlist: container
                .as_ref()
                .map(|value| value.allowlist.clone())
                .unwrap_or_default(),
            egress_guard: container.as_ref().map(|value| value.egress_guard),
        }))
    }

    pub async fn stop_container(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        let name = format!("ctx-harness-{}", workspace_id.0);
        let podman_exists = match container_exists(&self.data_root, &name).await {
            Ok(exists) => exists,
            Err(err) => {
                if err
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("podman binary unavailable")
                {
                    false
                } else {
                    return Err(err);
                }
            }
        };
        if podman_exists {
            let mut containers = self.containers.lock().await;
            containers.remove(&workspace_id);
            let mut cmd = podman_command(&self.data_root)?;
            cmd.arg("rm").arg("-f").arg(&name);
            let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
            if output.status.success() {
                return Ok(true);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                if combined.is_empty() {
                    anyhow::bail!("podman rm failed for {name} (status: {})", output.status);
                }
                anyhow::bail!("podman rm failed for {name}: {combined}");
            }
        }

        let state = match avf_linux_workspace_vm_state(&self.data_root, workspace_id) {
            Ok(state) => state,
            Err(_) => return Ok(false),
        };
        if matches!(
            state.state,
            avf_linux_vm::AvfLinuxSharedVmLifecycleState::Missing
        ) {
            return Ok(false);
        }
        let stopped = stop_avf_linux_workspace_vm(&self.data_root, workspace_id)?;
        let mut containers = self.containers.lock().await;
        containers.remove(&workspace_id);
        Ok(!matches!(
            stopped.state,
            avf_linux_vm::AvfLinuxSharedVmLifecycleState::Missing
        ))
    }

    pub async fn remove_workspace_volume(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        // Best-effort cleanup: callers (e.g. workspace deletion) may ignore failures.
        let name = format!("ctx-ws-{}", workspace_id.0);
        let mut inspect = podman_command(&self.data_root)?;
        inspect.arg("volume").arg("inspect").arg(&name);
        let out = command_output_with_timeout(inspect, PODMAN_OP_TIMEOUT).await?;
        if !out.status.success() {
            return Ok(false);
        }

        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("volume").arg("rm").arg("-f").arg(&name);
        let out = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
        if out.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let combined = format!("{stderr}\n{stdout}").trim().to_string();
            if combined.is_empty() {
                anyhow::bail!(
                    "podman volume rm failed for {name} (status: {})",
                    out.status
                );
            }
            anyhow::bail!("podman volume rm failed for {name}: {combined}");
        }
    }

    async fn ensure_container(
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

    async fn ensure_container_after_machine_ready(
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
        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
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
            if let Ok(mut cmd) = podman_command(&self.data_root) {
                cmd.arg("rm").arg("-f").arg(&name);
                let _ = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await;
            }
        }

        let exists = if recreate {
            false
        } else {
            container_exists(&self.data_root, &name).await?
        };

        if exists {
            let running = container_running(&self.data_root, &name)
                .await?
                .unwrap_or(false);
            if !running {
                observe_phase(
                    observer,
                    HarnessSetupPhase::ContainerStartOrCreate,
                    "starting existing workspace container",
                );
                let mut cmd = podman_command(&self.data_root)?;
                cmd.arg("start").arg(&name);
                let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let combined = format!("{stderr}\n{stdout}").trim().to_string();
                    if combined.is_empty() {
                        anyhow::bail!("podman start failed for {name} (status: {})", output.status);
                    }
                    anyhow::bail!("podman start failed for {name}: {combined}");
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
            if readiness == ContainerReadinessState::MachineReady {
                self.ensure_container_image_ready(settings, observer)
                    .await?;
            }
            observe_phase(
                observer,
                HarnessSetupPhase::ContainerStartOrCreate,
                "creating workspace container",
            );
            let mut cmd = podman_command(&self.data_root)?;
            cmd.arg("run").arg("-d").arg("--name").arg(&name);
            if should_use_keep_id_userns() {
                cmd.arg("--userns=keep-id");
            }
            if let Some(user) = container_user() {
                cmd.arg("--user").arg(user);
            }
            cmd.arg("--network")
                .arg("slirp4netns:allow_host_loopback=true");
            cmd.arg("--cap-add").arg("NET_ADMIN");
            cmd.arg("--add-host")
                .arg("host.containers.internal:host-gateway");
            for mount in &mount_plan.mounts {
                cmd.arg("--mount").arg(mount);
            }
            cmd.arg(&image);
            cmd.arg("/bin/sh")
                .arg("-c")
                .arg("while true; do sleep 100000; done");
            let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                if combined.is_empty() {
                    anyhow::bail!("podman run failed for {name} (status: {})", output.status);
                }
                anyhow::bail!("podman run failed for {name}: {combined}");
            }
        }

        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            verify_disk_isolated_container_mounts(&self.data_root, workspace, &name).await?;
        }

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
