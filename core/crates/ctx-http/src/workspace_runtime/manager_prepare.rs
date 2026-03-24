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
}
