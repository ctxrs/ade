use super::*;

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
pub(super) fn relay_socket_copy(mut reader: impl Read, mut writer: impl Write) -> Result<()> {
    match std::io::copy(&mut reader, &mut writer) {
        Ok(_) => Ok(()),
        Err(err) if io_error_is_benign(&err) => Ok(()),
        Err(err) => Err(err).context("relaying socket bytes"),
    }
}

#[cfg(all(target_os = "macos", unix))]
pub(super) fn connect_shared_vm_guest_control_socket(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
) -> Result<File> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let virtual_machine_addr = (&**virtual_machine as *const VZVirtualMachine) as usize;
    let request_sender = sender.clone();
    exec_on_dispatch_queue(
        queue,
        "shared AVF Linux VM guest control connect dispatch",
        move || -> Result<()> {
            let completion_sender = request_sender.clone();
            let completion = RcBlock::new(
                move |connection: *mut VZVirtioSocketConnection, error: *mut NSError| {
                    let result = if !error.is_null() {
                        let error = unsafe { &*error };
                        Err(anyhow::anyhow!(format_nserror(error)))
                    } else if connection.is_null() {
                        Err(anyhow::anyhow!(
                            "guest control connection completed without a socket"
                        ))
                    } else {
                        let fd = unsafe { (*connection).fileDescriptor() };
                        if fd < 0 {
                            Err(anyhow::anyhow!(
                                "guest control connection reported a closed file descriptor"
                            ))
                        } else {
                            let dup_fd = unsafe { libc::dup(fd) };
                            if dup_fd < 0 {
                                Err(anyhow::anyhow!(
                                    "duplicating guest control file descriptor failed: {}",
                                    std::io::Error::last_os_error()
                                ))
                            } else {
                                Ok(dup_fd)
                            }
                        }
                    };
                    let _ = completion_sender.send(result);
                },
            );

            let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
            let socket_devices = unsafe { virtual_machine.socketDevices() };
            let Some(socket_device) = socket_devices.iter().next() else {
                bail!("shared AVF Linux VM has no socket devices configured");
            };
            let socket_device =
                unsafe { &*((&*socket_device) as *const _ as *const VZVirtioSocketDevice) };
            unsafe {
                socket_device.connectToPort_completionHandler(
                    SHARED_VM_GUEST_CONTROL_VSOCK_PORT,
                    &completion,
                );
            }
            Ok(())
        },
    )??;

    match receiver.recv_timeout(GUEST_EXEC_CONNECT_TIMEOUT) {
        Ok(Ok(fd)) => Ok(unsafe { File::from_raw_fd(fd) }),
        Ok(Err(err)) => Err(err).with_context(|| {
            format!(
                "connecting to guest vsock port {}",
                SHARED_VM_GUEST_CONTROL_VSOCK_PORT
            )
        }),
        Err(mpsc::RecvTimeoutError::Timeout) => bail!(
            "timed out waiting for guest vsock port {}",
            SHARED_VM_GUEST_CONTROL_VSOCK_PORT
        ),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("guest control completion handler disconnected unexpectedly")
        }
    }
}

#[cfg(all(target_os = "macos", unix))]
pub(super) fn relay_shared_vm_control_client(client: UnixStream, guest: File) -> Result<()> {
    let mut client_reader = client
        .try_clone()
        .context("cloning shared VM control client")?;
    let mut client_writer = client;
    let mut guest_reader = guest.try_clone().context("cloning guest control socket")?;
    let mut guest_writer = guest;

    let guest_write_fd = guest_writer.as_raw_fd();
    let forward = std::thread::spawn(move || -> Result<()> {
        let result = relay_socket_copy(&mut client_reader, &mut guest_writer);
        let _ = unsafe { libc::shutdown(guest_write_fd, libc::SHUT_WR) };
        result
    });
    let reverse = std::thread::spawn(move || -> Result<()> {
        let result = relay_socket_copy(&mut guest_reader, &mut client_writer);
        let _ = client_writer.shutdown(std::net::Shutdown::Write);
        result
    });

    forward
        .join()
        .map_err(|_| anyhow::anyhow!("shared VM control forward relay thread panicked"))??;
    reverse
        .join()
        .map_err(|_| anyhow::anyhow!("shared VM control reverse relay thread panicked"))??;
    Ok(())
}

#[cfg(all(target_os = "macos", unix))]
pub(super) fn service_real_shared_vm_control_clients(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
    listener: &UnixListener,
    data_root: &Path,
) -> Result<()> {
    loop {
        match listener.accept() {
            Ok((client, _)) => {
                let guest = match connect_shared_vm_guest_control_socket(queue, virtual_machine) {
                    Ok(guest) => guest,
                    Err(err) => {
                        append_shared_vm_log_line(
                            data_root,
                            &format!(
                                "connecting to guest vsock port {SHARED_VM_GUEST_CONTROL_VSOCK_PORT}: {err:#}"
                            ),
                        )?;
                        continue;
                    }
                };
                let log_root = data_root.to_path_buf();
                std::thread::spawn(move || {
                    if let Err(err) = relay_shared_vm_control_client(client, guest) {
                        let _ = append_shared_vm_log_line(
                            &log_root,
                            &format!("real shared VM guest relay failed: {err:#}"),
                        );
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(err) => return Err(err).context("accepting shared VM control client"),
        }
    }
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
            "starting real AVF Linux VM (rootfs={}, kernel={}, initrd={})",
            rootfs_image.display(),
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

    loop {
        service_real_shared_vm_control_clients(&queue, &virtual_machine, &listener, data_root)?;
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
            bail!("shared AVF Linux VM entered the Virtualization error state");
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

pub(super) fn spawn_real_shared_vm_owner(data_root: &Path) -> Result<u32> {
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
    wait_for_control_socket(data_root)?;
    if let Err(err) = wait_for_real_guest_exec_ready(data_root) {
        stop_shared_vm_server(child.id());
        return Err(err);
    }
    Ok(child.id())
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

#[cfg(unix)]
pub(super) fn wait_for_real_guest_exec_ready(data_root: &Path) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut last_err: Option<anyhow::Error> = None;
    while std::time::Instant::now() < deadline {
        match run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/true",
            &[],
            None,
            HashMap::new(),
            None,
        ) {
            Ok(result) if result.exit_code == 0 => return Ok(()),
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
    Err(last_err.unwrap_or_else(|| {
        anyhow::anyhow!(
            "timed out waiting for real AVF guest exec readiness via {}",
            control_socket.display()
        )
    }))
}

#[cfg(not(unix))]
pub(super) fn wait_for_real_guest_exec_ready(_data_root: &Path) -> Result<()> {
    Ok(())
}
