use super::*;

impl HarnessRuntimeManager {
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
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let combined = format!("{stderr}\n{stdout}").trim().to_string();
            if combined.is_empty() {
                anyhow::bail!("podman rm failed for {name} (status: {})", output.status);
            }
            anyhow::bail!("podman rm failed for {name}: {combined}");
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
}
