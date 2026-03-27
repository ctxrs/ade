use super::*;

#[path = "real_vm_runtime/guest_control.rs"]
mod guest_control;
#[path = "real_vm_runtime/resource_management.rs"]
mod resource_management;

use self::guest_control::service_real_shared_vm_control_clients;
pub(super) use self::guest_control::{run_owner_guest_exec_capture, shared_vm_owner_guest_probe_ready};
use self::resource_management::{
    align_down_to_mebibyte, host_available_memory_bytes, maybe_adjust_shared_vm_memory,
    maybe_grow_shared_vm_data_disk, SharedVmResourceState,
};
pub(super) use self::resource_management::{
    resolve_shared_vm_memory_watchdog_exit_action, resolve_shared_vm_memory_watchdog_sample_action,
    SharedVmMemoryWatchdogExitAction, SharedVmMemoryWatchdogSampleAction,
};
#[cfg(test)]
pub(super) use self::guest_control::{
    is_transient_guest_control_connect_nserror, relay_shared_vm_control_client,
};
#[cfg(test)]
pub(super) use self::resource_management::{
    resolve_shared_vm_data_disk_growth_decision, resolve_shared_vm_memory_balloon_action,
    SharedVmDataDiskGrowthDecision, SharedVmMemoryBalloonAction,
};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_host_self() -> libc::mach_port_t;
}

#[cfg(unix)]
pub(super) fn io_error_is_benign(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
    )
}

#[cfg(unix)]
fn write_exec_error_frame_best_effort(
    writer: &mut impl Write,
    code: &str,
    message: impl Into<String>,
) {
    let _ = write_exec_frame(
        writer,
        &AvfLinuxExecFrame::Error(AvfLinuxExecError {
            code: code.to_string(),
            message: message.into(),
        }),
    );
}

#[cfg(unix)]
fn close_guest_exec_stdin_best_effort(writer: &Arc<Mutex<File>>) {
    let Ok(mut guard) = writer.lock() else {
        return;
    };
    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
}

const SHARED_VM_READINESS_PHASE_PREFIX: &str = "[ctx-avf-linux] readiness phase ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SharedVmGuestReadinessReport {
    pub(super) attempts: u32,
    pub(super) elapsed: Duration,
    pub(super) phase_lines: Vec<String>,
}

pub(super) fn format_duration_ms(duration: Duration) -> String {
    format!("{} ms", duration.as_millis())
}

pub(super) fn extract_shared_vm_readiness_phase_lines(stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    [stdout, stderr]
        .into_iter()
        .flat_map(|buffer| {
            String::from_utf8_lossy(buffer)
                .lines()
                .map(str::trim)
                .filter(|line| line.starts_with(SHARED_VM_READINESS_PHASE_PREFIX))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(super) fn summarize_shared_vm_readiness_phase_lines(phase_lines: &[String]) -> String {
    if phase_lines.is_empty() {
        return "no per-phase readiness timings were emitted".to_string();
    }

    phase_lines
        .iter()
        .map(|line| {
            line.strip_prefix(SHARED_VM_READINESS_PHASE_PREFIX)
                .unwrap_or(line.as_str())
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(target_os = "macos")]
pub(super) fn persist_shared_vm_owner_error_state(
    state_path: &Path,
    state: &mut PersistedSharedVmState,
    note: String,
) -> Result<()> {
    state.state = AvfLinuxSharedVmLifecycleState::Error;
    state.simulated = false;
    state.updated_at = Some(now_timestamp_string());
    state.last_stopped_at = state.updated_at.clone();
    state.transition_status = None;
    state.relay_pid = None;
    state.guest_agent_pid = None;
    state.notes = vec![note];
    persist_state(state_path, state)
}

#[cfg(target_os = "macos")]
pub(super) fn shutdown_real_shared_vm_for_exit(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
    data_root: &Path,
) -> String {
    let virtual_machine_ptr = &**virtual_machine as *const VZVirtualMachine;
    let mut notes = Vec::new();
    let mut saved_state_written = false;
    let initial_state = match virtual_machine_state_on_queue(queue, virtual_machine_ptr) {
        Ok(state) => state,
        Err(err) => return format!("failed to query workspace VM state before shutdown: {err:#}"),
    };

    if shared_vm_save_restore_supported() {
        if matches!(
            initial_state,
            VZVirtualMachineState::Running | VZVirtualMachineState::Paused
        ) {
            if initial_state == VZVirtualMachineState::Running {
                match pause_virtual_machine_on_queue(queue, virtual_machine_ptr) {
                    Ok(()) => notes.push("paused workspace VM before save".to_string()),
                    Err(err) => notes.push(format!("pause before save failed: {err:#}")),
                }
            }

            match virtual_machine_state_on_queue(queue, virtual_machine_ptr) {
                Ok(VZVirtualMachineState::Paused) => {
                    let save_path = shared_vm_saved_state_path(data_root);
                    if let Some(parent) = save_path.parent() {
                        if let Err(err) = fs::create_dir_all(parent) {
                            notes.push(format!(
                                "failed to prepare saved-state directory {}: {err:#}",
                                parent.display()
                            ));
                        }
                    }
                    let _ = fs::remove_file(&save_path);
                    match save_virtual_machine_state_on_queue(
                        queue,
                        virtual_machine_ptr,
                        &save_path,
                    ) {
                        Ok(()) => {
                            saved_state_written = true;
                            notes.push(format!(
                                "saved workspace VM state to {}",
                                save_path.display()
                            ));
                        }
                        Err(err) => {
                            notes.push(format!("saving workspace VM state failed: {err:#}"))
                        }
                    }
                }
                Ok(other) => notes.push(format!(
                    "skipped save because workspace VM remained in state {other:?}"
                )),
                Err(err) => notes.push(format!(
                    "failed to re-check workspace VM state before save: {err:#}"
                )),
            }
        } else {
            notes.push(format!(
                "skipped save because workspace VM was in state {initial_state:?}"
            ));
        }
    } else {
        notes.push("save/restore unavailable on this host; stopping workspace VM cold".to_string());
    }

    if saved_state_written {
        notes.push("workspace VM owner exited after save without an additional stop".to_string());
        return notes.join("; ");
    }

    match virtual_machine_can_stop_on_queue(queue, virtual_machine_ptr) {
        Ok(true) => match stop_virtual_machine_on_queue(queue, virtual_machine_ptr) {
            Ok(()) => notes.push("stopped workspace VM owner cleanly".to_string()),
            Err(err) => notes.push(format!("workspace VM stop failed: {err:#}")),
        },
        Ok(false) => notes.push("workspace VM owner could not issue a clean stop".to_string()),
        Err(err) => notes.push(format!(
            "failed to check whether workspace VM could stop: {err:#}"
        )),
    }

    notes.join("; ")
}

#[cfg(target_os = "macos")]
pub(super) fn run_shared_vm(data_root: &Path) -> Result<()> {
    let state_path = shared_vm_state_path(data_root);
    let mut state = load_state(&state_path)?
        .ok_or_else(|| anyhow::anyhow!("shared VM state is missing at {}", state_path.display()))?;
    let rootfs_image = state
        .rootfs_image
        .clone()
        .ok_or_else(|| anyhow::anyhow!("shared VM state is missing rootfs_image"))?;
    let kernel_path = state
        .kernel_path
        .clone()
        .ok_or_else(|| anyhow::anyhow!("shared VM state is missing kernel_path"))?;
    let initrd_path = state
        .initrd_path
        .clone()
        .ok_or_else(|| anyhow::anyhow!("shared VM state is missing initrd_path"))?;
    let runtime_root = state
        .runtime_root
        .clone()
        .ok_or_else(|| anyhow::anyhow!("shared VM state is missing runtime_root"))?;
    let data_disk_image = shared_vm_data_disk_path(data_root);
    if !data_disk_image.is_file() {
        bail!(
            "shared AVF Linux data disk is missing at {}",
            data_disk_image.display()
        );
    }

    let listener = bind_shared_vm_control_listener(data_root)?;
    listener
        .set_nonblocking(true)
        .context("setting shared VM control listener nonblocking")?;
    let saved_state_path = shared_vm_saved_state_path(data_root);
    let preserving_seed_for_restore =
        shared_vm_save_restore_supported() && saved_state_path.is_file();
    append_shared_vm_log_line(
        data_root,
        &format!(
            "starting real AVF Linux VM (rootfs={}, data_disk={}, kernel={}, initrd={})",
            rootfs_image.display(),
            data_disk_image.display(),
            kernel_path.display(),
            initrd_path.display()
        ),
    )?;
    let seed_image =
        stage_shared_vm_cloud_init_seed(data_root, &runtime_root, preserving_seed_for_restore)?;
    if let Some(seed_image) = seed_image.as_ref() {
        let action = if preserving_seed_for_restore {
            "reusing"
        } else {
            "staged"
        };
        append_shared_vm_log_line(
            data_root,
            &format!(
                "{action} AVF cloud-init seed image at {}",
                seed_image.display()
            ),
        )?;
    }
    let kernel_cmdline = load_shared_vm_kernel_cmdline(&runtime_root)?;

    let queue = DispatchQueue::new(
        "rs.ctx.desktop.avf-linux.shared-vm",
        DispatchQueueAttr::SERIAL,
    );
    let build_virtual_machine = || {
        build_real_avf_linux_virtual_machine(
            data_root,
            &rootfs_image,
            &data_disk_image,
            &kernel_path,
            &initrd_path,
            seed_image.as_deref(),
            &kernel_cmdline,
            &queue,
        )
    };
    let mut virtual_machine = build_virtual_machine()?;
    let mut virtual_machine_ptr = &*virtual_machine as *const VZVirtualMachine;
    let restored_from_saved_state = if shared_vm_save_restore_supported()
        && saved_state_path.is_file()
    {
        append_shared_vm_log_line(
            data_root,
            &format!(
                "attempting to restore saved workspace VM state from {}",
                saved_state_path.display()
            ),
        )?;
        match restore_virtual_machine_state_on_queue(&queue, virtual_machine_ptr, &saved_state_path)
            .and_then(|_| resume_virtual_machine_on_queue(&queue, virtual_machine_ptr))
        {
            Ok(()) => {
                append_shared_vm_log_line(
                    data_root,
                    &format!(
                        "restored workspace VM state from {} and resumed the guest",
                        saved_state_path.display()
                    ),
                )?;
                true
            }
            Err(err) => {
                append_shared_vm_log_line(
                    data_root,
                    &format!(
                        "restoring saved workspace VM state from {} failed; falling back to a cold boot: {err:#}",
                        saved_state_path.display()
                    ),
                )?;
                let _ = fs::remove_file(&saved_state_path);
                virtual_machine = build_virtual_machine()?;
                virtual_machine_ptr = &*virtual_machine as *const VZVirtualMachine;
                false
            }
        }
    } else {
        false
    };

    if !restored_from_saved_state {
        let virtual_machine_addr = virtual_machine_ptr as usize;
        let can_start =
            exec_on_dispatch_queue(&queue, "shared AVF Linux VM canStart", move || unsafe {
                let virtual_machine_ptr = virtual_machine_addr as *const VZVirtualMachine;
                (&*virtual_machine_ptr).canStart()
            })?;
        if !can_start {
            bail!("shared AVF Linux VM cannot be started from its current state");
        }
        start_virtual_machine_on_queue(&queue, virtual_machine_ptr)?;
        append_shared_vm_log_line(
            data_root,
            &format!(
                "real AVF Linux VM started successfully; forwarding host control socket {} to guest vsock port {}",
                shared_vm_control_socket_path(data_root).display(),
                SHARED_VM_GUEST_CONTROL_VSOCK_PORT
            ),
        )?;
    } else {
        append_shared_vm_log_line(
            data_root,
            &format!(
                "real AVF Linux VM restored successfully; forwarding host control socket {} to guest vsock port {}",
                shared_vm_control_socket_path(data_root).display(),
                SHARED_VM_GUEST_CONTROL_VSOCK_PORT
            ),
        )?;
    }

    let min_cpu = unsafe { VZVirtualMachineConfiguration::minimumAllowedCPUCount() };
    let max_cpu = unsafe { VZVirtualMachineConfiguration::maximumAllowedCPUCount() };
    let min_memory = unsafe { VZVirtualMachineConfiguration::minimumAllowedMemorySize() };
    let max_memory = unsafe { VZVirtualMachineConfiguration::maximumAllowedMemorySize() };
    let sizing = resolved_avf_vm_sizing_for_host(min_cpu, max_cpu, min_memory, max_memory)?;
    let memory_floor_bytes =
        align_down_to_mebibyte(SHARED_VM_MIN_DEFAULT_MEMORY_BYTES.max(min_memory))
            .min(sizing.memory_size_bytes);
    let _watchdog_pid = spawn_shared_vm_memory_watchdog(data_root, std::process::id())
        .context("spawning shared AVF Linux VM memory watchdog")?;
    let mut resource_state =
        SharedVmResourceState::new(sizing.memory_size_bytes, memory_floor_bytes);
    loop {
        service_real_shared_vm_control_clients(&queue, &virtual_machine, &listener, data_root)?;
        if let Some(note) = shared_vm_memory_pressure_stop_requested_note(data_root)? {
            let shutdown_note =
                shutdown_real_shared_vm_for_exit(&queue, &virtual_machine, data_root);
            clear_shared_vm_memory_pressure_stop_request(data_root);
            let combined_note = format!("{note}; {shutdown_note}");
            append_shared_vm_log_line(data_root, &combined_note)?;
            persist_shared_vm_owner_error_state(&state_path, &mut state, combined_note.clone())?;
            bail!("{combined_note}");
        }
        if shared_vm_shutdown_requested(data_root) {
            let shutdown_note =
                shutdown_real_shared_vm_for_exit(&queue, &virtual_machine, data_root);
            clear_shared_vm_shutdown_request(data_root);
            state.state = AvfLinuxSharedVmLifecycleState::Stopped;
            state.simulated = false;
            state.updated_at = Some(now_timestamp_string());
            state.last_saved_at = shared_vm_saved_state_path(data_root)
                .exists()
                .then(|| state.updated_at.clone())
                .flatten();
            state.last_stopped_at = state.updated_at.clone();
            state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Stopped);
            state.relay_pid = None;
            state.guest_agent_pid = None;
            state.notes = vec![shutdown_note];
            persist_state(&state_path, &state)?;
            append_shared_vm_log_line(
                data_root,
                "shared AVF Linux VM owner honored a shutdown request and exited cleanly",
            )?;
            return Ok(());
        }
        let vm_state = virtual_machine_state_on_queue(&queue, virtual_machine_ptr)?;
        if vm_state == VZVirtualMachineState::Running {
            if let Err(err) =
                maybe_grow_shared_vm_data_disk(&queue, &virtual_machine, data_root, &mut resource_state)
            {
                let shutdown_note =
                    shutdown_real_shared_vm_for_exit(&queue, &virtual_machine, data_root);
                let note = format!(
                    "workspace VM owner stopped because AVF data-disk maintenance failed: {err:#}; {shutdown_note}"
                );
                append_shared_vm_log_line(data_root, &note)?;
                persist_shared_vm_owner_error_state(&state_path, &mut state, note)?;
                return Err(err).context("maintaining AVF data-disk capacity");
            }
            if let Err(err) = maybe_adjust_shared_vm_memory(
                &queue,
                &virtual_machine,
                data_root,
                &mut resource_state,
            ) {
                let shutdown_note =
                    shutdown_real_shared_vm_for_exit(&queue, &virtual_machine, data_root);
                let note = format!(
                    "workspace VM owner stopped because AVF memory maintenance failed: {err:#}; {shutdown_note}"
                );
                append_shared_vm_log_line(data_root, &note)?;
                persist_shared_vm_owner_error_state(&state_path, &mut state, note)?;
                return Err(err).context("maintaining AVF memory pressure controls");
            }
        }
        if matches!(
            vm_state,
            VZVirtualMachineState::Running
                | VZVirtualMachineState::Starting
                | VZVirtualMachineState::Resuming
                | VZVirtualMachineState::Paused
                | VZVirtualMachineState::Pausing
                | VZVirtualMachineState::Saving
                | VZVirtualMachineState::Restoring
        ) {
            std::thread::sleep(SHARED_VM_CONTROL_POLL_INTERVAL);
            continue;
        }
        if vm_state == VZVirtualMachineState::Error {
            let note = "shared AVF Linux VM entered the Virtualization error state".to_string();
            append_shared_vm_log_line(data_root, &note)?;
            persist_shared_vm_owner_error_state(&state_path, &mut state, note.clone())?;
            bail!("{note}");
        }
        append_shared_vm_log_line(
            data_root,
            &format!("shared AVF Linux VM exited control loop with state {vm_state:?}"),
        )?;
        state.state = AvfLinuxSharedVmLifecycleState::Stopped;
        state.simulated = false;
        state.updated_at = Some(now_timestamp_string());
        state.last_saved_at = shared_vm_saved_state_path(data_root)
            .exists()
            .then(|| state.last_saved_at.clone())
            .flatten();
        state.last_stopped_at = state.updated_at.clone();
        state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Stopped);
        state.relay_pid = None;
        state.guest_agent_pid = None;
        state.notes = vec![format!(
            "workspace VM owner exited its control loop with state {vm_state:?}"
        )];
        persist_state(&state_path, &state)?;
        return Ok(());
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn run_shared_vm(_data_root: &Path) -> Result<()> {
    bail!("real shared AVF Linux VM ownership requires macOS")
}

#[cfg(target_os = "macos")]
pub(super) fn run_shared_vm_memory_watchdog(data_root: &Path, owner_pid: u32) -> Result<()> {
    let host_port = unsafe { mach_host_self() };
    let mut consecutive_emergency_samples = 0_u32;
    let mut logged_host_memory_error = false;

    loop {
        if !shared_vm_server_process_alive(owner_pid) || shared_vm_shutdown_requested(data_root) {
            return Ok(());
        }

        match host_available_memory_bytes(host_port) {
            Ok(available_host_bytes) => {
                logged_host_memory_error = false;
                match resolve_shared_vm_memory_watchdog_sample_action(
                    consecutive_emergency_samples,
                    available_host_bytes,
                ) {
                    SharedVmMemoryWatchdogSampleAction::NoAction {
                        next_consecutive_emergency_samples,
                    } => {
                        consecutive_emergency_samples = next_consecutive_emergency_samples;
                    }
                    SharedVmMemoryWatchdogSampleAction::RequestStop {
                        next_consecutive_emergency_samples: _next_consecutive_emergency_samples,
                        available_host_bytes,
                    } => {
                        let note = format!(
                            "host memory pressure emergency watchdog triggered: available host memory fell to {:.2} GiB, below the {:.2} GiB emergency floor",
                            available_host_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                            SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES as f64
                                / (1024.0 * 1024.0 * 1024.0),
                        );
                        append_shared_vm_log_line(data_root, &note)?;
                        request_shared_vm_memory_pressure_stop(data_root, &note)?;
                        let exit_action = if wait_for_process_exit(
                            owner_pid,
                            SHARED_VM_MEMORY_WATCHDOG_EXIT_GRACE,
                        ) {
                            SharedVmMemoryWatchdogExitAction::OwnerExitedAfterRequest
                        } else {
                            append_shared_vm_log_line(
                                data_root,
                                "shared AVF Linux VM owner did not exit after the emergency memory stop request; sending SIGTERM",
                            )?;
                            stop_shared_vm_server(owner_pid);
                            resolve_shared_vm_memory_watchdog_exit_action(
                                false,
                                wait_for_process_exit(
                                    owner_pid,
                                    SHARED_VM_MEMORY_WATCHDOG_EXIT_GRACE,
                                ),
                            )
                        };

                        match exit_action {
                            SharedVmMemoryWatchdogExitAction::OwnerExitedAfterRequest
                            | SharedVmMemoryWatchdogExitAction::OwnerExitedAfterSigterm => {
                                return Ok(());
                            }
                            SharedVmMemoryWatchdogExitAction::EscalateToSigkill => unsafe {
                                libc::kill(owner_pid as i32, libc::SIGKILL);
                            },
                        }
                        append_shared_vm_log_line(
                            data_root,
                            "shared AVF Linux VM memory watchdog escalated to SIGKILL after the owner failed to exit under emergency host pressure",
                        )?;
                        return Ok(());
                    }
                }
            }
            Err(err) => {
                consecutive_emergency_samples = 0;
                if !logged_host_memory_error {
                    append_shared_vm_log_line(
                        data_root,
                        &format!(
                            "shared AVF Linux VM memory watchdog could not read host memory stats: {err:#}"
                        ),
                    )?;
                    logged_host_memory_error = true;
                }
            }
        }

        std::thread::sleep(SHARED_VM_MEMORY_WATCHDOG_POLL_INTERVAL);
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn run_shared_vm_memory_watchdog(_data_root: &Path, _owner_pid: u32) -> Result<()> {
    bail!("shared AVF Linux VM memory watchdog requires macOS")
}

fn spawn_shared_vm_memory_watchdog(data_root: &Path, owner_pid: u32) -> Result<u32> {
    let current_exe = std::env::current_exe().context("resolving helper executable path")?;
    let log_path = shared_vm_log_path(data_root);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log_file_err = log_file
        .try_clone()
        .with_context(|| format!("cloning {}", log_path.display()))?;
    let child = Command::new(current_exe)
        .arg("watch-workspace-vm-memory")
        .arg(data_root)
        .arg(owner_pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err))
        .spawn()
        .context("spawning workspace AVF memory watchdog")?;
    Ok(child.id())
}

fn spawn_real_shared_vm_owner_once(data_root: &Path, readiness_timeout: Duration) -> Result<u32> {
    let current_exe = std::env::current_exe().context("resolving helper executable path")?;
    let log_path = shared_vm_log_path(data_root);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log_file_err = log_file
        .try_clone()
        .with_context(|| format!("cloning {}", log_path.display()))?;
    let child = Command::new(current_exe)
        .arg("run-workspace-vm")
        .arg(data_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err))
        .spawn()
        .context("spawning workspace AVF VM owner process")?;
    let owner_started_at = std::time::Instant::now();
    let control_socket_wait_started_at = std::time::Instant::now();
    wait_for_control_socket(data_root)?;
    append_shared_vm_log_line(
        data_root,
        &format!(
            "shared AVF Linux VM control socket became available in {} after owner spawn",
            format_duration_ms(control_socket_wait_started_at.elapsed())
        ),
    )?;
    let guest_control_ready_wait_started_at = std::time::Instant::now();
    let remaining_after_control_socket =
        readiness_timeout.saturating_sub(owner_started_at.elapsed());
    wait_for_guest_control_ready_marker(data_root, remaining_after_control_socket)?;
    append_shared_vm_log_line(
        data_root,
        &format!(
            "shared AVF Linux guest control ready marker became available in {} after owner spawn",
            format_duration_ms(guest_control_ready_wait_started_at.elapsed())
        ),
    )?;
    let remaining_after_guest_control_ready =
        readiness_timeout.saturating_sub(owner_started_at.elapsed());
    let readiness =
        match wait_for_real_guest_exec_ready(data_root, remaining_after_guest_control_ready) {
            Ok(report) => report,
            Err(err) => {
                stop_shared_vm_server(child.id());
                return Err(err);
            }
        };
    for phase_line in &readiness.phase_lines {
        append_shared_vm_log_line(data_root, phase_line)?;
    }
    append_shared_vm_log_line(
        data_root,
        &format!(
            "shared AVF Linux guest readiness completed in {} across {} attempt(s): {}",
            format_duration_ms(readiness.elapsed),
            readiness.attempts,
            summarize_shared_vm_readiness_phase_lines(&readiness.phase_lines),
        ),
    )?;
    append_shared_vm_log_line(
        data_root,
        &format!(
            "shared AVF Linux VM owner reached launch-ready in {} total",
            format_duration_ms(owner_started_at.elapsed())
        ),
    )?;
    Ok(child.id())
}

pub(super) fn reset_writable_shared_vm_runtime_state(data_root: &Path) -> Result<()> {
    clear_shared_vm_shutdown_request(data_root);
    clear_shared_vm_memory_pressure_stop_request(data_root);
    for path in [
        shared_vm_control_socket_path(data_root),
        shared_vm_guest_agent_socket_path(data_root),
        shared_vm_guest_control_ready_path(data_root),
        shared_vm_saved_state_path(data_root),
        shared_vm_rootfs_path(data_root),
    ] {
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    Ok(())
}

pub(super) fn shared_vm_readiness_failure_requires_writable_rootfs_reset(
    err: &anyhow::Error,
) -> bool {
    let rendered = format!("{err:#}");
    rendered.contains("[ctx-avf-linux] bridge_probe_failed")
}

pub(super) fn spawn_real_shared_vm_owner(
    data_root: &Path,
    readiness_timeout: Duration,
) -> Result<u32> {
    match spawn_real_shared_vm_owner_once(data_root, readiness_timeout) {
        Ok(pid) => Ok(pid),
        Err(err) if shared_vm_readiness_failure_requires_writable_rootfs_reset(&err) => {
            eprintln!(
                "[ctx-avf-linux] guest readiness failed bridge probe; resetting writable rootfs and retrying once: {err:#}"
            );
            reset_writable_shared_vm_runtime_state(data_root)
                .context("resetting writable shared VM runtime state after bridge probe failure")?;
            spawn_real_shared_vm_owner_once(data_root, cold_boot_real_guest_exec_ready_timeout())
                .context("retrying shared AVF VM owner boot after writable rootfs reset")
        }
        Err(err) => Err(err),
    }
}

pub(super) fn spawn_shared_vm_server(data_root: &Path) -> Result<u32> {
    let current_exe = std::env::current_exe().context("resolving helper executable path")?;
    let log_path = shared_vm_log_path(data_root);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log_file_err = log_file
        .try_clone()
        .with_context(|| format!("cloning {}", log_path.display()))?;
    let child = Command::new(current_exe)
        .arg("serve-workspace-vm")
        .arg(data_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err))
        .spawn()
        .context("spawning workspace VM relay process")?;
    wait_for_control_socket(data_root)?;
    Ok(child.id())
}

pub(super) fn spawn_guest_agent_server(data_root: &Path) -> Result<u32> {
    let current_exe = std::env::current_exe().context("resolving helper executable path")?;
    let log_path = shared_vm_log_path(data_root);
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("opening {}", log_path.display()))?;
    let log_file_err = log_file
        .try_clone()
        .with_context(|| format!("cloning {}", log_path.display()))?;
    let child = Command::new(current_exe)
        .arg("serve-guest-agent")
        .arg(data_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file_err))
        .spawn()
        .context("spawning guest-agent relay process")?;
    wait_for_guest_agent_socket(data_root)?;
    Ok(child.id())
}

pub(super) fn wait_for_control_socket(data_root: &Path) -> Result<()> {
    let socket_path = shared_vm_control_socket_path(data_root);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if socket_path.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!(
        "timed out waiting for shared VM control socket {}",
        socket_path.display()
    )
}

pub(super) fn wait_for_guest_agent_socket(data_root: &Path) -> Result<()> {
    let socket_path = shared_vm_guest_agent_socket_path(data_root);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if socket_path.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!(
        "timed out waiting for guest-agent control socket {}",
        socket_path.display()
    )
}

pub(super) fn wait_for_guest_control_ready_marker(
    data_root: &Path,
    timeout: Duration,
) -> Result<()> {
    let marker_path = shared_vm_guest_control_ready_path(data_root);
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if marker_path.exists() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    bail!(
        "timed out waiting for guest control ready marker {}",
        marker_path.display()
    )
}

#[cfg(unix)]
pub(super) fn shared_vm_guest_readiness_args() -> Vec<String> {
    vec![
        String::from("-lc"),
        format!(
            "set -e; ctx_uptime_ms() {{ awk '{{print int($1 * 1000)}}' /proc/uptime; }}; ctx_run_phase() {{ phase=\"$1\"; shift; start_ms=$(ctx_uptime_ms); if timeout --kill-after=1s --preserve-status {phase_timeout_seconds}s \"$@\"; then end_ms=$(ctx_uptime_ms); echo \"{phase_prefix}${{phase}} ok in $((end_ms-start_ms))ms\" >&2; else status=$?; end_ms=$(ctx_uptime_ms); echo \"{phase_prefix}${{phase}} failed with exit $status after $((end_ms-start_ms))ms\" >&2; return $status; fi; }}; ctx_run_phase containerd systemctl is-active --quiet {containerd_service}; ctx_run_phase buildkit systemctl is-active --quiet {buildkit_service}; ctx_run_phase nerdctl sh -lc '{nerdctl_bin} version >/dev/null 2>&1'; ctx_run_phase buildctl sh -lc '{buildctl_bin} --addr {buildkit_socket} debug workers >/dev/null 2>&1'; ctx_run_phase bridge-probe sh -lc 'probe_bridge=ctxavfbr0; ip link delete \"$probe_bridge\" >/dev/null 2>&1 || true; if ! ip link add name \"$probe_bridge\" type bridge >/tmp/ctx-avf-bridge-probe.out 2>/tmp/ctx-avf-bridge-probe.err; then cat /tmp/ctx-avf-bridge-probe.out >&2 || true; cat /tmp/ctx-avf-bridge-probe.err >&2 || true; echo \"[ctx-avf-linux] bridge_probe_failed\" >&2; exit 41; fi; ip link delete \"$probe_bridge\" >/dev/null 2>&1 || true'",
            "set -e; ctx_uptime_ms() {{ awk '{{print int($1 * 1000)}}' /proc/uptime; }}; ctx_run_phase() {{ phase=\"$1\"; shift; start_ms=$(ctx_uptime_ms); if timeout --kill-after=1s --preserve-status {phase_timeout_seconds}s \"$@\"; then end_ms=$(ctx_uptime_ms); echo \"{phase_prefix}${{phase}} ok in $((end_ms-start_ms))ms\" >&2; else status=$?; end_ms=$(ctx_uptime_ms); echo \"{phase_prefix}${{phase}} failed with exit $status after $((end_ms-start_ms))ms\" >&2; return $status; fi; }}; ctx_run_phase containerd systemctl is-active --quiet {containerd_service}; ctx_run_phase buildkit systemctl is-active --quiet {buildkit_service}; ctx_run_phase nerdctl sh -lc '{nerdctl_bin} version >/dev/null 2>&1'; ctx_run_phase buildctl sh -lc '{buildctl_bin} --addr {buildkit_socket} debug workers >/dev/null 2>&1'; ctx_run_phase bridge-probe sh -lc 'probe_bridge=ctxavfbr0; ip link delete \"$probe_bridge\" >/dev/null 2>&1 || true; if ! ip link add name \"$probe_bridge\" type bridge >/tmp/ctx-avf-bridge-probe.out 2>/tmp/ctx-avf-bridge-probe.err; then cat /tmp/ctx-avf-bridge-probe.out >&2 || true; cat /tmp/ctx-avf-bridge-probe.err >&2 || true; echo \"[ctx-avf-linux] bridge_probe_failed\" >&2; exit 41; fi; ip link delete \"$probe_bridge\" >/dev/null 2>&1 || true'",
            containerd_service = SHARED_VM_CONTAINERD_SERVICE_NAME,
            buildkit_service = SHARED_VM_BUILDKIT_SERVICE_NAME,
            nerdctl_bin = SHARED_VM_GUEST_NERDCTL_BIN,
            buildctl_bin = SHARED_VM_GUEST_BUILDKITCTL_BIN,
            buildkit_socket = SHARED_VM_GUEST_BUILDKIT_SOCKET,
            phase_prefix = SHARED_VM_READINESS_PHASE_PREFIX,
            phase_timeout_seconds = SHARED_VM_READINESS_PHASE_TIMEOUT_SECONDS,
        ),
    ]
}

#[cfg(not(unix))]
pub(super) fn shared_vm_guest_readiness_args() -> Vec<String> {
    Vec::new()
}

pub(super) fn default_real_guest_exec_ready_timeout() -> Duration {
    Duration::from_secs(30)
}

pub(super) fn cold_boot_real_guest_exec_ready_timeout() -> Duration {
    Duration::from_secs(90)
}

pub(super) fn real_guest_exec_ready_timeout_for_start(
    rootfs_materialization_note: Option<&str>,
    saved_state_exists: bool,
) -> Duration {
    if rootfs_materialization_note.is_some() || !saved_state_exists {
        cold_boot_real_guest_exec_ready_timeout()
    } else {
        default_real_guest_exec_ready_timeout()
    }
}

#[cfg(unix)]
pub(super) fn wait_for_real_guest_exec_ready(
    data_root: &Path,
    timeout: Duration,
) -> Result<SharedVmGuestReadinessReport> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let started_at = std::time::Instant::now();
    let deadline = std::time::Instant::now() + timeout;
    let mut last_err: Option<anyhow::Error> = None;
    let readiness_args = shared_vm_guest_readiness_args();
    let mut attempts = 0_u32;
    while std::time::Instant::now() < deadline {
        attempts += 1;
        match run_guest_exec_capture_with_socket_timeout(
        match run_guest_exec_capture_with_socket_timeout(
            &control_socket,
            Path::new("/"),
            "/bin/sh",
            &readiness_args,
            Some("root"),
            HashMap::new(),
            None,
            Some(SHARED_VM_READINESS_GUEST_EXEC_IO_TIMEOUT),
        ) {
            Ok(result) if result.exit_code == 0 => {
                return Ok(SharedVmGuestReadinessReport {
                    attempts,
                    elapsed: started_at.elapsed(),
                    phase_lines: extract_shared_vm_readiness_phase_lines(
                        &result.stdout,
                        &result.stderr,
                    ),
                });
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
                last_err = Some(anyhow::anyhow!(
                    "guest exec readiness probe exited {} (stdout='{}', stderr='{}')",
                    result.exit_code,
                    stdout,
                    stderr
                ));
            }
            Err(err) => {
                last_err = Some(err);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let timeout_message = format!(
        "timed out waiting for real AVF guest exec readiness via {} after {} attempt(s) over {}",
        control_socket.display(),
        attempts,
        format_duration_ms(started_at.elapsed())
    );
    match last_err {
        Some(err) => Err(err.context(timeout_message)),
        None => Err(anyhow::anyhow!(timeout_message)),
    }
}

#[cfg(not(unix))]
pub(super) fn wait_for_real_guest_exec_ready(
    _data_root: &Path,
    _timeout: Duration,
) -> Result<SharedVmGuestReadinessReport> {
    Ok(SharedVmGuestReadinessReport {
        attempts: 0,
        elapsed: Duration::ZERO,
        phase_lines: Vec::new(),
    })
}
