use super::*;

impl HarnessRuntimeManager {
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
    ) -> Result<(avf_linux_vm::AvfLinuxSharedVmState, PathBuf, bool)> {
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
        let guest_worktree_root = ensure_avf_linux_guest_worktree_from_host_copy(
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
            &guest_worktree_root,
            &settings.container,
            AVF_GUEST_HOST_GATEWAY,
            daemon_port,
        )
        .await?
        .egress_guard;
        Ok((workspace_vm, guest_worktree_root, egress_guard))
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
}
