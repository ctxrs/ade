use super::*;

#[derive(Debug)]
pub(super) struct SharedVmStartLockGuard {
    path: PathBuf,
}

impl Drop for SharedVmStartLockGuard {
    fn drop(&mut self) {
        if self.path.exists() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) fn parse_shared_vm_start_lock_pid(raw: &str) -> Option<u32> {
    raw.trim().parse::<u32>().ok().filter(|pid| *pid > 0)
}

fn try_acquire_shared_vm_start_lock(data_root: &Path) -> Result<Option<SharedVmStartLockGuard>> {
    let lock_path = shared_vm_start_lock_path(data_root);
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(mut file) => {
            writeln!(file, "{}", std::process::id())
                .with_context(|| format!("writing {}", lock_path.display()))?;
            Ok(Some(SharedVmStartLockGuard { path: lock_path }))
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let holder_pid = fs::read_to_string(&lock_path)
                .ok()
                .and_then(|raw| parse_shared_vm_start_lock_pid(&raw));
            let stale_holder = match holder_pid {
                Some(pid) => !shared_vm_server_process_alive(pid),
                None => true,
            };
            if stale_holder {
                fs::remove_file(&lock_path)
                    .with_context(|| format!("removing stale {}", lock_path.display()))?;
            }
            Ok(None)
        }
        Err(err) => Err(err).with_context(|| format!("opening {}", lock_path.display())),
    }
}

pub(super) fn acquire_shared_vm_start_lock(
    data_root: &Path,
    timeout: Duration,
) -> Result<SharedVmStartLockGuard> {
    let lock_path = shared_vm_start_lock_path(data_root);
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(guard) = try_acquire_shared_vm_start_lock(data_root)? {
            return Ok(guard);
        }
        if std::time::Instant::now() >= deadline {
            bail!(
                "timed out waiting for shared VM start lock {}",
                lock_path.display()
            );
        }
        std::thread::sleep(SHARED_VM_START_LOCK_POLL_INTERVAL);
    }
}

pub(super) fn prepare_runtime_layout(data_root: &Path) -> Result<AvfLinuxRuntimeLayout> {
    let vm_root = shared_vm_root(data_root);
    let logs_root = shared_vm_logs_root(data_root);
    let state_path = shared_vm_state_path(data_root);
    let existed = vm_root.exists() && logs_root.exists() && state_path.exists();

    fs::create_dir_all(&vm_root).with_context(|| format!("creating {}", vm_root.display()))?;
    fs::create_dir_all(&logs_root).with_context(|| format!("creating {}", logs_root.display()))?;

    if !state_path.exists() {
        let state = PersistedSharedVmState {
            state: AvfLinuxSharedVmLifecycleState::Stopped,
            guest_identity: supported_guest_identity(),
            runtime_root: None,
            rootfs_image: None,
            kernel_path: None,
            initrd_path: None,
            runtime_version: None,
            runtime_shape_digest: None,
            updated_at: Some(now_timestamp_string()),
            last_started_at: None,
            last_saved_at: None,
            last_stopped_at: None,
            transition_status: None,
            last_start_outcome: None,
            last_stop_outcome: None,
            last_restore_error: None,
            last_save_error: None,
            relay_pid: None,
            guest_agent_pid: None,
            simulated: true,
            notes: vec![
                "shared VM lifecycle scaffold is present, but actual AVF guest boot is not implemented yet".to_string(),
            ],
        };
        persist_state(&state_path, &state)?;
    }

    Ok(AvfLinuxRuntimeLayout {
        protocol_version: HELPER_PROTOCOL_VERSION,
        protocol_schema: HELPER_PROTOCOL_SCHEMA,
        vm_root,
        logs_root,
        state_path,
        layout_status: if existed {
            AvfLinuxRuntimeLayoutStatus::AlreadyPresent
        } else {
            AvfLinuxRuntimeLayoutStatus::Prepared
        },
        notes: vec![
            "shared AVF Linux VM layout is scaffolded under the daemon data root".to_string(),
        ],
    })
}

pub(super) fn shared_vm_state(data_root: &Path) -> Result<AvfLinuxSharedVmStateResponse> {
    let vm_root = shared_vm_root(data_root);
    let logs_root = shared_vm_logs_root(data_root);
    let state_path = shared_vm_state_path(data_root);
    let log_path = shared_vm_log_path(data_root);
    let mut persisted = load_state(&state_path)?;
    if let Some(state) = persisted.as_mut() {
        let missing_owner = state
            .relay_pid
            .is_some_and(|pid| !shared_vm_server_process_alive(pid));
        let missing_simulated_guest = state.simulated
            && state
                .guest_agent_pid
                .is_some_and(|pid| !shared_vm_server_process_alive(pid));
        if matches!(state.state, AvfLinuxSharedVmLifecycleState::Running)
            && (missing_owner || missing_simulated_guest)
        {
            if let Some(pid) = state.relay_pid.take() {
                stop_shared_vm_server(pid);
            }
            if let Some(pid) = state.guest_agent_pid.take() {
                stop_shared_vm_server(pid);
            }
            let memory_pressure_note = shared_vm_memory_pressure_stop_requested_note(data_root)?;
            state.state = if memory_pressure_note.is_some() {
                AvfLinuxSharedVmLifecycleState::Error
            } else {
                AvfLinuxSharedVmLifecycleState::Stopped
            };
            state.updated_at = Some(now_timestamp_string());
            state.last_stopped_at = state.updated_at.clone();
            state.last_saved_at = None;
            state.last_stop_outcome = Some(AvfLinuxSharedVmStopOutcome::ColdStop);
            state.last_save_error = None;
            state.transition_status = if memory_pressure_note.is_some() {
                None
            } else {
                Some(AvfLinuxSharedVmTransitionStatus::Stopped)
            };
            state.notes = vec![if let Some(note) = memory_pressure_note {
                clear_shared_vm_memory_pressure_stop_request(data_root);
                format!(
                    "shared VM owner exited after an emergency host-memory stop request: {note}"
                )
            } else if state.simulated {
                "shared VM relay or simulated guest-agent process was not alive; marking the scaffolded VM stopped"
                    .to_string()
            } else {
                "shared VM owner process was not alive; marking the AVF VM stopped".to_string()
            }];
            persist_state(&state_path, state)?;
        }
    }
    Ok(map_state_response(
        persisted.as_ref(),
        data_root,
        vm_root,
        logs_root,
        state_path,
        log_path,
    ))
}

pub(super) fn discard_stale_saved_state_for_cold_stop(data_root: &Path) -> Option<String> {
    let saved_state_path = shared_vm_saved_state_path(data_root);
    match fs::remove_file(&saved_state_path) {
        Ok(()) => Some(format!(
            "discarded stale workspace VM saved state at {} because this stop did not produce a fresh save",
            saved_state_path.display()
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => Some(format!(
            "failed to discard stale workspace VM saved state at {}: {err:#}; {}",
            saved_state_path.display(),
            describe_saved_state_path_context(&saved_state_path)
        )),
    }
}

fn clear_shared_vm_transient_artifacts(data_root: &Path) {
    for path in [
        shared_vm_control_socket_path(data_root),
        shared_vm_guest_agent_socket_path(data_root),
        shared_vm_guest_control_ready_path(data_root),
    ] {
        if path.exists() {
            let _ = fs::remove_file(path);
        }
    }
}

pub(super) fn start_shared_vm(
    data_root: &Path,
    runtime_root: &Path,
    rootfs_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    runtime_version: String,
) -> Result<AvfLinuxSharedVmStateResponse> {
    let start_requested_at = std::time::Instant::now();
    for path in [runtime_root, rootfs_image, kernel_path, initrd_path] {
        if !path.exists() {
            bail!(
                "required AVF Linux runtime path is missing: {}",
                path.display()
            );
        }
    }
    let _ = prepare_runtime_layout(data_root)?;
    clear_shared_vm_shutdown_request(data_root);
    clear_shared_vm_memory_pressure_stop_request(data_root);
    let _start_lock_guard = acquire_shared_vm_start_lock(
        data_root,
        cold_boot_real_guest_exec_ready_timeout() + Duration::from_secs(30),
    )?;
    let state_path = shared_vm_state_path(data_root);
    let mut state = load_state(&state_path)?.unwrap_or_else(default_stopped_state);
    let saved_state_path = shared_vm_saved_state_path(data_root);
    let requested_runtime_shape_digest = shared_vm_runtime_shape_digest(
        runtime_root,
        rootfs_image,
        kernel_path,
        initrd_path,
        runtime_version.as_str(),
    );
    let runtime_shape_changed = match state.runtime_shape_digest.as_deref() {
        Some(previous_digest) => previous_digest != requested_runtime_shape_digest.as_str(),
        None => {
            state.runtime_version.as_deref() != Some(runtime_version.as_str())
                || state.runtime_root.as_ref().map(PathBuf::as_path) != Some(runtime_root)
                || state.initrd_path.as_ref().map(PathBuf::as_path) != Some(initrd_path)
        }
    };
    let mut runtime_restart_note = None;
    let mut owner_alive = state.relay_pid.is_some_and(shared_vm_server_process_alive);
    let mut guest_alive = state
        .guest_agent_pid
        .is_some_and(shared_vm_server_process_alive);
    let mut already_running = if state.simulated {
        owner_alive && guest_alive
    } else {
        owner_alive
    };
    if already_running && runtime_shape_changed {
        let restart_note =
            "requested AVF runtime differs from the running shared VM; forcing a stop before restart"
                .to_string();
        append_shared_vm_log_line(data_root, &restart_note)?;
        let stopped = stop_shared_vm(data_root)
            .context("stopping already-running shared VM after runtime shape change")?;
        if !matches!(stopped.state, AvfLinuxSharedVmLifecycleState::Stopped) {
            bail!(
                "expected shared VM to stop before runtime change restart, found {:?}",
                stopped.state
            );
        }
        reset_writable_shared_vm_runtime_state(data_root)
            .context("resetting writable shared VM runtime state after runtime shape change")?;
        runtime_restart_note = Some(restart_note);
        state = load_state(&state_path)?.unwrap_or_else(default_stopped_state);
        owner_alive = state.relay_pid.is_some_and(shared_vm_server_process_alive);
        guest_alive = state
            .guest_agent_pid
            .is_some_and(shared_vm_server_process_alive);
        already_running = if state.simulated {
            owner_alive && guest_alive
        } else {
            owner_alive
        };
    }
    let mut stale_saved_state_note = None;
    if runtime_shape_changed && saved_state_path.exists() {
        fs::remove_file(&saved_state_path).with_context(|| {
            format!(
                "removing stale saved AVF Linux VM state {}",
                saved_state_path.display()
            )
        })?;
        stale_saved_state_note = Some(format!(
            "discarded saved workspace VM state at {} because the staged runtime changed",
            saved_state_path.display()
        ));
    }
    if runtime_shape_changed {
        let staged_rootfs_path = shared_vm_rootfs_path(data_root);
        if staged_rootfs_path.exists() {
            fs::remove_file(&staged_rootfs_path).with_context(|| {
                format!(
                    "removing stale writable AVF Linux rootfs {}",
                    staged_rootfs_path.display()
                )
            })?;
        }
    }

    let kernel_cmdline = load_shared_vm_kernel_cmdline(runtime_root)?;
    let (boot_kernel_path, kernel_materialization_note) =
        materialize_bootable_kernel_image(data_root, kernel_path)?;
    let (staged_rootfs_image, rootfs_materialization_note) =
        materialize_writable_rootfs_image(data_root, rootfs_image)?;
    let (data_disk_image, data_disk_materialization_note) = materialize_data_disk_image(data_root)?;

    let native_validation_note = if cfg!(test) {
        None
    } else {
        Some(
            validate_real_avf_linux_vm_configuration(
                data_root,
                &staged_rootfs_image,
                &data_disk_image,
                &boot_kernel_path,
                initrd_path,
                &kernel_cmdline,
            )
            .unwrap_or_else(|err| {
                format!("native AVF configuration validation did not complete: {err:#}")
            }),
        )
    };
    let (real_vm_supported, real_vm_support_note) = if cfg!(test) {
        (
            false,
            "test mode keeps the shared VM on the simulated relay path".to_string(),
        )
    } else {
        shared_vm_runtime_supports_real_guest_exec(runtime_root)
    };
    if already_running {
        state.state = AvfLinuxSharedVmLifecycleState::Running;
        state.runtime_root = Some(runtime_root.to_path_buf());
        state.rootfs_image = Some(staged_rootfs_image.clone());
        state.kernel_path = Some(boot_kernel_path.clone());
        state.initrd_path = Some(initrd_path.to_path_buf());
        state.runtime_version = Some(runtime_version);
        state.runtime_shape_digest = Some(requested_runtime_shape_digest.clone());
        state.updated_at = Some(now_timestamp_string());
        state.last_started_at = state.updated_at.clone();
        if runtime_shape_changed {
            state.last_saved_at = None;
        }
        state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Scaffolded);
        state.last_start_outcome = Some(AvfLinuxSharedVmStartOutcome::AlreadyRunning);
        state.last_restore_error = None;
        state.notes = vec![if state.simulated {
            "shared VM relay and guest-agent processes were already alive; reusing the simulated shared VM"
                    .to_string()
        } else {
            "shared VM owner process was already alive; reusing the real AVF VM".to_string()
        }];
        if let Some(note) = kernel_materialization_note.clone() {
            state.notes.push(note);
        }
        if let Some(note) = rootfs_materialization_note.clone() {
            state.notes.push(note);
        }
        if let Some(note) = data_disk_materialization_note.clone() {
            state.notes.push(note);
        }
        state.notes.push(real_vm_support_note.clone());
        if let Some(note) = native_validation_note.clone() {
            state.notes.push(note);
        }
        if let Some(note) = stale_saved_state_note.clone() {
            state.notes.push(note);
        }
        if let Some(note) = runtime_restart_note.clone() {
            state.notes.push(note);
        }
        state.notes.push(format!(
            "shared VM start reused an already-running {} path in {}",
            if state.simulated {
                "simulated"
            } else {
                "real AVF"
            },
            format_duration_ms(start_requested_at.elapsed())
        ));
        let _ = append_shared_vm_log_line(
            data_root,
            &format!(
                "shared VM start reused an already-running {} path in {}",
                if state.simulated {
                    "simulated"
                } else {
                    "real AVF"
                },
                format_duration_ms(start_requested_at.elapsed())
            ),
        );
        persist_state(&state_path, &state)?;
        return shared_vm_state(data_root);
    }
    if let Some(pid) = state.relay_pid.take() {
        stop_shared_vm_server(pid);
    }
    if let Some(pid) = state.guest_agent_pid.take() {
        stop_shared_vm_server(pid);
    }
    clear_shared_vm_transient_artifacts(data_root);
    state.runtime_root = Some(runtime_root.to_path_buf());
    state.rootfs_image = Some(staged_rootfs_image.clone());
    state.kernel_path = Some(boot_kernel_path.clone());
    state.initrd_path = Some(initrd_path.to_path_buf());
    state.runtime_version = Some(runtime_version.clone());
    state.runtime_shape_digest = Some(requested_runtime_shape_digest.clone());
    state.updated_at = Some(now_timestamp_string());
    state.last_started_at = None;
    if runtime_shape_changed {
        state.last_saved_at = None;
    }
    state.transition_status = None;
    state.last_start_outcome = None;
    state.last_restore_error = None;
    state.relay_pid = None;
    state.guest_agent_pid = None;
    state.notes =
        vec!["persisting AVF runtime paths before starting the shared VM owner".to_string()];
    persist_state(&state_path, &state)?;

    let saved_state_exists = saved_state_path.exists();
    let restore_eligible_for_start = !cfg!(test)
        && real_vm_supported
        && shared_vm_save_restore_supported()
        && saved_state_exists;
    let readiness_timeout = real_guest_exec_ready_timeout_for_start(
        rootfs_materialization_note.as_deref(),
        restore_eligible_for_start,
    );
    let timeout_reason = if rootfs_materialization_note.is_some() {
        "writable rootfs was materialized for this start"
    } else if restore_eligible_for_start {
        "saved state is eligible for restore on the real AVF path"
    } else if saved_state_exists {
        "saved state exists but this start cannot restore it on the selected path"
    } else {
        "no saved state exists yet"
    };
    let restore_unavailable_error = (saved_state_exists && !restore_eligible_for_start).then(|| {
        "saved workspace VM state was present, but this start could not use it and proceeded with a cold boot"
            .to_string()
    });
    append_shared_vm_log_line(
        data_root,
        &format!(
            "shared VM start selected {} path with readiness timeout {} because {}",
            if restore_eligible_for_start {
                "restore-candidate"
            } else {
                "cold-boot"
            },
            format_duration_ms(readiness_timeout),
            timeout_reason
        ),
    )?;
    let (relay_pid, guest_agent_pid, simulated, start_outcome, restore_error, mut notes) = if cfg!(
        test
    ) {
        (
            None,
            None,
            true,
            AvfLinuxSharedVmStartOutcome::ColdBoot,
            restore_unavailable_error.clone(),
            vec!["shared VM start was requested in test mode; state is simulated until actual AVF guest boot is implemented".to_string()],
        )
    } else if real_vm_supported {
        let relay_pid = spawn_real_shared_vm_owner(data_root, readiness_timeout)?;
        let owner_state = load_state(&state_path)?.ok_or_else(|| {
            anyhow::anyhow!(
                "shared VM owner reached launch-ready but state disappeared at {}",
                state_path.display()
            )
        })?;
        let start_outcome = owner_state.last_start_outcome.ok_or_else(|| {
            anyhow::anyhow!(
                "shared VM owner reached launch-ready without reporting last_start_outcome in {}",
                state_path.display()
            )
        })?;
        (
            Some(relay_pid),
            None,
            false,
            start_outcome,
            owner_state.last_restore_error.clone(),
            vec![
                "shared VM owner process is running and owns a real AVF Linux VM lifecycle"
                    .to_string(),
                real_vm_support_note.clone(),
            ],
        )
    } else {
        let guest_agent_pid = spawn_guest_agent_server(data_root)?;
        let relay_pid = match spawn_shared_vm_server(data_root) {
            Ok(pid) => pid,
            Err(err) => {
                stop_shared_vm_server(guest_agent_pid);
                return Err(err);
            }
        };
        (
            Some(relay_pid),
            Some(guest_agent_pid),
            true,
            AvfLinuxSharedVmStartOutcome::ColdBoot,
            restore_unavailable_error.clone(),
            vec![
                "shared VM relay and guest-agent processes are running; state remains simulated until actual AVF guest boot is implemented".to_string(),
                real_vm_support_note.clone(),
            ],
        )
    };
    state.state = AvfLinuxSharedVmLifecycleState::Running;
    state.runtime_root = Some(runtime_root.to_path_buf());
    state.rootfs_image = Some(staged_rootfs_image);
    state.kernel_path = Some(boot_kernel_path);
    state.initrd_path = Some(initrd_path.to_path_buf());
    state.runtime_version = Some(runtime_version);
    state.runtime_shape_digest = Some(requested_runtime_shape_digest);
    state.updated_at = Some(now_timestamp_string());
    state.last_started_at = state.updated_at.clone();
    state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Scaffolded);
    state.last_start_outcome = Some(start_outcome);
    state.last_restore_error = restore_error;
    state.relay_pid = relay_pid;
    state.guest_agent_pid = guest_agent_pid;
    state.simulated = simulated;
    if let Some(note) = kernel_materialization_note {
        notes.push(note);
    }
    if let Some(note) = rootfs_materialization_note {
        notes.push(note);
    }
    if let Some(note) = data_disk_materialization_note {
        notes.push(note);
    }
    if let Some(note) = native_validation_note {
        notes.push(note);
    }
    if let Some(note) = stale_saved_state_note {
        notes.push(note);
    }
    if let Some(note) = runtime_restart_note {
        notes.push(note);
    }
    if let Some(note) = restore_unavailable_error {
        notes.push(note);
    }
    notes.push(format!(
        "shared VM start reached launch-ready in {} via {} path",
        format_duration_ms(start_requested_at.elapsed()),
        if simulated { "simulated" } else { "real AVF" }
    ));
    state.notes = notes;
    persist_state(&state_path, &state)?;
    shared_vm_state(data_root)
}

pub(super) fn stop_shared_vm(data_root: &Path) -> Result<AvfLinuxSharedVmStateResponse> {
    let state_path = shared_vm_state_path(data_root);
    let Some(mut state) = load_state(&state_path)? else {
        return Ok(AvfLinuxSharedVmStateResponse {
            protocol_version: HELPER_PROTOCOL_VERSION,
            protocol_schema: HELPER_PROTOCOL_SCHEMA,
            state: AvfLinuxSharedVmLifecycleState::Missing,
            vm_root: shared_vm_root(data_root),
            logs_root: shared_vm_logs_root(data_root),
            state_path,
            log_path: Some(shared_vm_log_path(data_root)),
            saved_state_path: Some(shared_vm_saved_state_path(data_root)),
            saved_state_exists: shared_vm_saved_state_path(data_root).exists(),
            runtime_root: None,
            rootfs_image: None,
            kernel_path: None,
            initrd_path: None,
            runtime_version: None,
            runtime_shape_digest: None,
            updated_at: Some(now_timestamp_string()),
            last_started_at: None,
            last_saved_at: None,
            last_stopped_at: None,
            transition_status: Some(AvfLinuxSharedVmTransitionStatus::Missing),
            last_start_outcome: None,
            last_stop_outcome: None,
            last_restore_error: None,
            last_save_error: None,
            relay_pid: None,
            guest_agent_pid: None,
            simulated: true,
            notes: vec!["shared VM layout is missing; nothing to stop".to_string()],
        });
    };
    if !state.simulated {
        if let Some(owner_pid) = state.relay_pid {
            request_shared_vm_shutdown(data_root)?;
            if wait_for_process_exit(owner_pid, SHARED_VM_SHUTDOWN_WAIT_TIMEOUT) {
                clear_shared_vm_shutdown_request(data_root);
                clear_shared_vm_transient_artifacts(data_root);
                let response = shared_vm_state(data_root)?;
                if matches!(response.state, AvfLinuxSharedVmLifecycleState::Stopped) {
                    return Ok(response);
                }
            }
        }
    }
    if let Some(pid) = state.relay_pid.take() {
        stop_shared_vm_server(pid);
    }
    if let Some(pid) = state.guest_agent_pid.take() {
        stop_shared_vm_server(pid);
    }
    clear_shared_vm_shutdown_request(data_root);
    clear_shared_vm_memory_pressure_stop_request(data_root);
    clear_shared_vm_transient_artifacts(data_root);
    state.state = AvfLinuxSharedVmLifecycleState::Stopped;
    state.updated_at = Some(now_timestamp_string());
    state.last_saved_at = None;
    state.last_stopped_at = state.updated_at.clone();
    state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Stopped);
    state.last_stop_outcome = Some(AvfLinuxSharedVmStopOutcome::ColdStop);
    let mut notes = vec![if state.simulated {
        "shared VM lifecycle scaffold is stopped".to_string()
    } else {
        "real shared AVF Linux VM is stopped".to_string()
    }];
    if let Some(note) = discard_stale_saved_state_for_cold_stop(data_root) {
        if note.starts_with("failed to discard") {
            state.last_save_error = Some(note.clone());
        } else {
            state.last_save_error = None;
        }
        notes.push(note);
    } else {
        state.last_save_error = None;
    }
    state.notes = notes;
    persist_state(&state_path, &state)?;
    shared_vm_state(data_root)
}

pub(super) fn request_shared_vm_shutdown(data_root: &Path) -> Result<()> {
    let path = shared_vm_shutdown_request_path(data_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, now_timestamp_string()).with_context(|| format!("writing {}", path.display()))
}

pub(super) fn clear_shared_vm_shutdown_request(data_root: &Path) {
    let path = shared_vm_shutdown_request_path(data_root);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn request_shared_vm_memory_pressure_stop(data_root: &Path, note: &str) -> Result<()> {
    let path = shared_vm_memory_pressure_request_path(data_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, note).with_context(|| format!("writing {}", path.display()))
}

pub(super) fn shared_vm_memory_pressure_stop_requested_note(
    data_root: &Path,
) -> Result<Option<String>> {
    let path = shared_vm_memory_pressure_request_path(data_root);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(raw.trim().to_string()))
}

pub(super) fn clear_shared_vm_memory_pressure_stop_request(data_root: &Path) {
    let path = shared_vm_memory_pressure_request_path(data_root);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn shared_vm_shutdown_requested(data_root: &Path) -> bool {
    shared_vm_shutdown_request_path(data_root).exists()
}

pub(super) fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if !shared_vm_server_process_alive(pid) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    !shared_vm_server_process_alive(pid)
}

#[cfg(unix)]
pub(super) fn shared_vm_server_process_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    err.raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
pub(super) fn shared_vm_server_process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
pub(super) fn stop_shared_vm_server(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
pub(super) fn stop_shared_vm_server(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

pub(super) fn shared_vm_runtime_shape_digest(
    runtime_root: &Path,
    rootfs_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    runtime_version: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(runtime_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(runtime_root.display().to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(rootfs_image.display().to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(kernel_path.display().to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(initrd_path.display().to_string().as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn append_shared_vm_log_line(data_root: &Path, line: &str) -> Result<()> {
    use std::io::Write as _;

    let path = shared_vm_log_path(data_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(file, "{line}").with_context(|| format!("writing {}", path.display()))
}
