use super::*;

pub(super) async fn has_running_workspace_containers(
    manager: &HarnessRuntimeManager,
) -> Result<bool> {
    let mut cmd = sandbox_container_command(manager.data_root())?;
    cmd.arg("ps").arg("--format").arg("{{.Names}}");
    let output = command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await?;
    if !output.status.success() {
        anyhow::bail!("sandbox CLI ps failed: {}", command_output_message(&output));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(str::trim)
        .any(|name| name.starts_with("ctx-harness-")))
}

pub(super) async fn should_defer_disk_isolated_machine_reconfiguration(
    manager: &HarnessRuntimeManager,
    settings: &ContainerExecutionSettings,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<bool> {
    if !matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
        return Ok(false);
    }
    disk_isolated_workspace_volumes_exist(manager, machine_name, observer).await
}

pub(super) async fn disk_isolated_workspace_volumes_exist(
    manager: &HarnessRuntimeManager,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<bool> {
    if !ensure_engine_ready_for_disk_state_inspection(manager, machine_name, observer).await? {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            "unable to verify disk-isolated workspace volumes before memory reconfiguration; leaving local sandbox runtime unchanged",
        );
        return Ok(true);
    }

    let mut cmd = sandbox_container_command(manager.data_root())?;
    cmd.arg("volume").arg("ls").arg("--format").arg("{{.Name}}");
    let output = match command_output_with_timeout(cmd, SANDBOX_OP_TIMEOUT).await {
        Ok(output) => output,
        Err(err) => {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!(
                    "failed to inspect disk-isolated workspace volumes before memory reconfiguration: {err}"
                ),
            );
            return Ok(true);
        }
    };
    if !output.status.success() {
        let detail = command_output_message(&output);
        let suffix = if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        };
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!(
                "unable to inspect disk-isolated workspace volumes before memory reconfiguration{suffix}"
            ),
        );
        return Ok(true);
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .any(|name| name.starts_with("ctx-ws-")))
}

pub(super) async fn ensure_engine_ready_for_disk_state_inspection(
    manager: &HarnessRuntimeManager,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<bool> {
    if sandbox_engine_ready(manager.data_root()).await? {
        return Ok(true);
    }

    observe_log(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        HarnessSetupLogLevel::Info,
        "starting local sandbox runtime to inspect disk-isolated workspace volumes before memory reconfiguration",
    );
    let mut start = sandbox_container_command(manager.data_root())?;
    start.arg("machine").arg("start").arg(machine_name);
    let output = command_output_with_timeout(start, SANDBOX_MACHINE_START_TIMEOUT).await?;
    if !output.status.success() {
        return Ok(false);
    }

    let deadline = Instant::now() + sandbox_machine_ready_timeout();
    loop {
        if sandbox_engine_ready(manager.data_root()).await? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(sandbox_machine_ready_poll_interval()).await;
    }
}

pub(super) async fn has_running_workspace_containers_for_stopped_machine_reconfiguration(
    manager: &HarnessRuntimeManager,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<bool> {
    match has_running_workspace_containers(manager).await {
        Ok(has_running) => Ok(has_running),
        Err(err) => {
            if sandbox_engine_ready(manager.data_root())
                .await
                .unwrap_or(false)
            {
                return Err(err);
            }
            let machine_name = sandbox_machine_name(manager.data_root());
            match manager.inspect_sandbox_machine_state(&machine_name).await? {
                Some(state) if state.contains("running") || state.contains("starting") => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::MachineStartOrInit,
                        HarnessSetupLogLevel::Warn,
                        "local sandbox runtime appears to be running but unreachable; deferring memory reconfiguration until workload probes recover",
                    );
                    tracing::debug!(
                        "treating workspace container probe failure as busy because the local sandbox runtime still reports a running state: {err:#}"
                    );
                    return Ok(true);
                }
                Some(_) => {}
                None => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::MachineStartOrInit,
                        HarnessSetupLogLevel::Warn,
                        "local sandbox runtime state is unknown while workload probes are unreachable; deferring memory reconfiguration until the runtime can be inspected safely",
                    );
                    tracing::debug!(
                        "treating workspace container probe failure as busy because the local sandbox runtime state is unknown: {err:#}"
                    );
                    return Ok(true);
                }
            }
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "local sandbox runtime is not reachable; continuing memory reconfiguration without workload probe",
            );
            tracing::debug!(
                "treating workspace container probe failure as idle because the sandbox runtime is not reachable: {err:#}"
            );
            Ok(false)
        }
    }
}
