use super::*;

#[cfg_attr(test, allow(dead_code))]
pub(in crate::workspace_runtime) async fn ensure_sandbox_machine_running_with_observer(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let machine_name = sandbox_machine_name(data_root);
    let machine_lock = sandbox_machine_singleflight_lock(&machine_name);
    let machine_guard = match machine_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "waiting for concurrent local sandbox runtime init/start operation",
            );
            machine_lock.lock().await
        }
    };
    if !sandbox_machine_required() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            "sandbox machine not required on this platform",
        );
        return Ok(());
    }
    let mut last_err = {
        let mut cmd = sandbox_container_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, SANDBOX_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineCheck,
                    HarnessSetupLogLevel::Info,
                    "local sandbox runtime is already reachable",
                );
                persist_sandbox_machine_cache_to_shared_best_effort(data_root, observer).await;
                return Ok(());
            }
            Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(err) => err.to_string(),
        }
    };

    seed_shared_sandbox_machine_cache_best_effort(data_root, observer).await;

    observe_phase(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        "starting or initializing local sandbox runtime",
    );
    clear_stale_sandbox_machine_temp_state(data_root, &machine_name, observer);
    let mut desired_memory_mb = None;

    let mut wait_after_start = true;
    let mut force_recreate = false;
    let start_out = {
        let mut start = sandbox_container_command(data_root)?;
        start.arg("machine").arg("start").arg(&machine_name);
        command_output_with_timeout(start, SANDBOX_MACHINE_START_TIMEOUT).await?
    };
    if start_out.status.success() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "local sandbox runtime start command completed; waiting for readiness",
        );
    } else {
        let combined = command_output_message(&start_out);
        let combined_lc = combined.to_ascii_lowercase();

        let looks_like_missing_machine = looks_like_missing_machine_error(&combined_lc);

        if looks_like_missing_machine {
            let desired_memory = match desired_memory_mb {
                Some(memory_mb) => memory_mb,
                None => {
                    let memory_mb = configured_sandbox_machine_memory_mb(data_root, observer).await;
                    desired_memory_mb = Some(memory_mb);
                    memory_mb
                }
            };
            observe_phase(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                "materializing local sandbox runtime from managed cache",
            );
            initialize_sandbox_machine(
                data_root,
                &machine_name,
                Some(desired_memory),
                observer,
                &mut last_err,
            )
            .await?;
            observe_phase(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                "waiting for local sandbox runtime readiness",
            );
        } else if looks_like_recoverable_machine_start_error(&combined_lc) {
            if looks_like_running_but_unreachable_machine_start_error(&combined_lc) {
                let message = if combined.is_empty() {
                    "sandbox machine start reported an already-running machine while sandbox CLI remained unreachable; restarting once"
                        .to_string()
                } else {
                    format!(
                        "sandbox machine start reported an already-running machine while sandbox CLI remained unreachable; restarting once: {combined}"
                    )
                };
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &message,
                );
                wait_after_start = false;
                force_recreate = true;
            } else {
                let message = if combined.is_empty() {
                    "sandbox machine start returned a recoverable error; waiting for readiness"
                        .to_string()
                } else {
                    format!(
                        "sandbox machine start returned recoverable error; waiting for readiness: {combined}"
                    )
                };
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &message,
                );
            }
            if !combined.is_empty() {
                last_err = combined;
            }
        } else if combined.is_empty() {
            anyhow::bail!(
                "sandbox machine start failed with non-zero exit {}",
                start_out.status
            );
        } else {
            anyhow::bail!("sandbox machine start failed: {combined}");
        }
    }

    if wait_after_start {
        observe_phase(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            "waiting for local sandbox runtime readiness",
        );
        if wait_for_sandbox_machine_ready(
            data_root,
            observer,
            "local sandbox runtime is ready",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    if !force_recreate {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            "sandbox machine remained unreachable after start; restarting once",
        );
        let stop_out = {
            let mut stop = sandbox_container_command(data_root)?;
            stop.arg("machine").arg("stop").arg(&machine_name);
            command_output_with_timeout(stop, SANDBOX_MACHINE_START_TIMEOUT).await?
        };
        if !stop_out.status.success() {
            let combined = command_output_message(&stop_out);
            if !combined.is_empty() {
                last_err = combined.clone();
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!("sandbox machine stop returned non-zero during recovery: {combined}"),
                );
            }
        }
        let restart_out = {
            let mut start = sandbox_container_command(data_root)?;
            start.arg("machine").arg("start").arg(&machine_name);
            command_output_with_timeout(start, SANDBOX_MACHINE_START_TIMEOUT).await?
        };
        if !restart_out.status.success() {
            let combined = command_output_message(&restart_out);
            if !combined.is_empty() {
                last_err = combined.clone();
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "sandbox machine start returned non-zero during restart recovery: {combined}"
                    ),
                );
            } else {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "sandbox machine start returned non-zero during restart recovery: {}",
                        restart_out.status
                    ),
                );
            }
        }
        observe_phase(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            "waiting for local sandbox runtime readiness",
        );
        if wait_for_sandbox_machine_ready(
            data_root,
            observer,
            "local sandbox runtime recovered after restart",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    let machine_present = sandbox_machine_present(data_root, &machine_name)
        .await
        .unwrap_or(false);
    if machine_present || force_recreate {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            if force_recreate {
                "sandbox machine reported an already-running but unreachable state; recreating machine"
            } else {
                "sandbox machine still unreachable after restart; recreating machine"
            },
        );
        cleanup_ctx_managed_sandbox_helper_processes(data_root, &machine_name, observer);
        clear_stale_sandbox_machine_temp_state(data_root, &machine_name, observer);

        if machine_present {
            let mut rm = sandbox_container_command(data_root)?;
            rm.arg("machine").arg("rm").arg("-f").arg(&machine_name);
            let rm_out = command_output_with_timeout(rm, SANDBOX_MACHINE_START_TIMEOUT).await?;
            if !rm_out.status.success() {
                let combined = command_output_message(&rm_out);
                if !combined.is_empty() {
                    last_err = format!("sandbox machine rm -f failed: {combined}");
                }
            }
        }

        let desired_memory = match desired_memory_mb {
            Some(memory_mb) => memory_mb,
            None => configured_sandbox_machine_memory_mb(data_root, observer).await,
        };

        if let Err(err) = initialize_sandbox_machine(
            data_root,
            &machine_name,
            Some(desired_memory),
            observer,
            &mut last_err,
        )
        .await
        {
            last_err = format!("sandbox machine init failed after recreate: {err:#}");
        } else {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "recreated sandbox machine; waiting for readiness",
            );
            if wait_for_sandbox_machine_ready(
                data_root,
                observer,
                "sandbox machine recovered after recreation",
                &mut last_err,
            )
            .await?
            {
                return Ok(());
            }
        }
    }

    drop(machine_guard);
    if last_err.trim().is_empty() {
        anyhow::bail!("sandbox machine remained unreachable after bounded recovery");
    }
    anyhow::bail!(
        "sandbox machine remained unreachable after bounded recovery: {}",
        last_err.trim()
    );
}
