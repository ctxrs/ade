use super::*;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SharedVmDataDiskGrowthDecision {
    NoAction,
    Grow {
        new_size_bytes: u64,
        additional_bytes: u64,
    },
    HostReserveBlocked {
        available_host_bytes: u64,
        reserve_bytes: u64,
        requested_additional_bytes: u64,
    },
}

pub(super) fn resolve_shared_vm_data_disk_growth_decision(
    current_size_bytes: u64,
    guest_free_bytes: u64,
    host_available_bytes: u64,
) -> SharedVmDataDiskGrowthDecision {
    if guest_free_bytes >= SHARED_VM_DATA_DISK_GROWTH_THRESHOLD_BYTES {
        return SharedVmDataDiskGrowthDecision::NoAction;
    }

    let requested_additional_bytes = SHARED_VM_DATA_DISK_GROWTH_STEP_BYTES;
    let host_growth_budget = host_available_bytes.saturating_sub(SHARED_VM_HOST_DISK_RESERVE_BYTES);
    if host_growth_budget == 0 {
        return SharedVmDataDiskGrowthDecision::HostReserveBlocked {
            available_host_bytes: host_available_bytes,
            reserve_bytes: SHARED_VM_HOST_DISK_RESERVE_BYTES,
            requested_additional_bytes,
        };
    }

    let additional_bytes = requested_additional_bytes.min(host_growth_budget);
    SharedVmDataDiskGrowthDecision::Grow {
        new_size_bytes: current_size_bytes.saturating_add(additional_bytes),
        additional_bytes,
    }
}

const MEBIBYTE_BYTES: u64 = 1024 * 1024;

fn align_down_to_mebibyte(bytes: u64) -> u64 {
    bytes - (bytes % MEBIBYTE_BYTES)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SharedVmMemoryBalloonAction {
    NoAction,
    Reclaim {
        new_target_bytes: u64,
        available_host_bytes: u64,
        aggressive: bool,
    },
    Grow {
        new_target_bytes: u64,
        available_host_bytes: u64,
        guest_available_bytes: u64,
    },
    EmergencyStop {
        available_host_bytes: u64,
        current_target_bytes: u64,
        floor_bytes: u64,
    },
}

pub(super) fn resolve_shared_vm_memory_balloon_action(
    current_target_bytes: u64,
    ceiling_bytes: u64,
    floor_bytes: u64,
    guest_available_bytes: Option<u64>,
    host_available_bytes: u64,
) -> SharedVmMemoryBalloonAction {
    let ceiling_bytes = align_down_to_mebibyte(ceiling_bytes.max(MEBIBYTE_BYTES));
    let floor_bytes = align_down_to_mebibyte(floor_bytes.max(MEBIBYTE_BYTES)).min(ceiling_bytes);
    let current_target_bytes =
        align_down_to_mebibyte(current_target_bytes).clamp(floor_bytes, ceiling_bytes);

    if host_available_bytes < SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES {
        if current_target_bytes <= floor_bytes {
            return SharedVmMemoryBalloonAction::EmergencyStop {
                available_host_bytes: host_available_bytes,
                current_target_bytes,
                floor_bytes,
            };
        }
        return SharedVmMemoryBalloonAction::Reclaim {
            new_target_bytes: floor_bytes,
            available_host_bytes: host_available_bytes,
            aggressive: true,
        };
    }

    if host_available_bytes < SHARED_VM_HOST_MEMORY_RESERVE_BYTES
        && current_target_bytes > floor_bytes
    {
        return SharedVmMemoryBalloonAction::Reclaim {
            new_target_bytes: current_target_bytes
                .saturating_sub(SHARED_VM_MEMORY_BALLOON_STEP_BYTES)
                .max(floor_bytes),
            available_host_bytes: host_available_bytes,
            aggressive: false,
        };
    }

    let Some(guest_available_bytes) = guest_available_bytes else {
        return SharedVmMemoryBalloonAction::NoAction;
    };
    if guest_available_bytes >= SHARED_VM_GUEST_MEMORY_GROW_THRESHOLD_BYTES
        || current_target_bytes >= ceiling_bytes
        || host_available_bytes
            <= SHARED_VM_HOST_MEMORY_RESERVE_BYTES + SHARED_VM_MEMORY_BALLOON_STEP_BYTES
    {
        return SharedVmMemoryBalloonAction::NoAction;
    }

    SharedVmMemoryBalloonAction::Grow {
        new_target_bytes: current_target_bytes
            .saturating_add(SHARED_VM_MEMORY_BALLOON_STEP_BYTES)
            .min(ceiling_bytes),
        available_host_bytes: host_available_bytes,
        guest_available_bytes,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SharedVmMemoryWatchdogSampleAction {
    NoAction {
        next_consecutive_emergency_samples: u32,
    },
    RequestStop {
        next_consecutive_emergency_samples: u32,
        available_host_bytes: u64,
    },
}

pub(super) fn resolve_shared_vm_memory_watchdog_sample_action(
    consecutive_emergency_samples: u32,
    available_host_bytes: u64,
) -> SharedVmMemoryWatchdogSampleAction {
    if available_host_bytes >= SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES {
        return SharedVmMemoryWatchdogSampleAction::NoAction {
            next_consecutive_emergency_samples: 0,
        };
    }

    let next_consecutive_emergency_samples = consecutive_emergency_samples.saturating_add(1);
    if next_consecutive_emergency_samples >= SHARED_VM_MEMORY_WATCHDOG_CONFIRMATION_POLLS {
        return SharedVmMemoryWatchdogSampleAction::RequestStop {
            next_consecutive_emergency_samples,
            available_host_bytes,
        };
    }

    SharedVmMemoryWatchdogSampleAction::NoAction {
        next_consecutive_emergency_samples,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SharedVmMemoryWatchdogExitAction {
    OwnerExitedAfterRequest,
    OwnerExitedAfterSigterm,
    EscalateToSigkill,
}

pub(super) fn resolve_shared_vm_memory_watchdog_exit_action(
    owner_exited_after_request: bool,
    owner_exited_after_sigterm: bool,
) -> SharedVmMemoryWatchdogExitAction {
    if owner_exited_after_request {
        SharedVmMemoryWatchdogExitAction::OwnerExitedAfterRequest
    } else if owner_exited_after_sigterm {
        SharedVmMemoryWatchdogExitAction::OwnerExitedAfterSigterm
    } else {
        SharedVmMemoryWatchdogExitAction::EscalateToSigkill
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug)]
enum SharedVmGuestControlConnectOutcome {
    Connected(File),
    Retryable(String),
    Fatal(String),
}

#[cfg(target_os = "macos")]
pub(super) fn is_transient_guest_control_connect_nserror(domain: &str, code: isize) -> bool {
    domain == "NSPOSIXErrorDomain"
        && matches!(
            code as i32,
            libc::ECONNRESET
                | libc::ECONNABORTED
                | libc::ECONNREFUSED
                | libc::ETIMEDOUT
                | libc::EAGAIN
                | libc::EINTR
                | libc::ENOTCONN
        )
}

#[cfg(all(target_os = "macos", unix))]
fn connect_shared_vm_guest_control_socket_once(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
) -> Result<SharedVmGuestControlConnectOutcome> {
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
                        let domain = error.domain().to_string();
                        let code = error.code();
                        let message = format_nserror(error);
                        if is_transient_guest_control_connect_nserror(&domain, code) {
                            SharedVmGuestControlConnectOutcome::Retryable(message)
                        } else {
                            SharedVmGuestControlConnectOutcome::Fatal(message)
                        }
                    } else if connection.is_null() {
                        SharedVmGuestControlConnectOutcome::Fatal(
                            "guest control connection completed without a socket".to_string(),
                        )
                    } else {
                        let fd = unsafe { (*connection).fileDescriptor() };
                        if fd < 0 {
                            SharedVmGuestControlConnectOutcome::Fatal(
                                "guest control connection reported a closed file descriptor"
                                    .to_string(),
                            )
                        } else {
                            let dup_fd = unsafe { libc::dup(fd) };
                            if dup_fd < 0 {
                                SharedVmGuestControlConnectOutcome::Fatal(format!(
                                    "duplicating guest control file descriptor failed: {}",
                                    std::io::Error::last_os_error()
                                ))
                            } else {
                                SharedVmGuestControlConnectOutcome::Connected(unsafe {
                                    File::from_raw_fd(dup_fd)
                                })
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
        Ok(result) => Ok(result),
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
pub(super) fn connect_shared_vm_guest_control_socket(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
) -> Result<File> {
    let deadline = std::time::Instant::now() + GUEST_EXEC_CONNECT_TIMEOUT;
    loop {
        match connect_shared_vm_guest_control_socket_once(queue, virtual_machine)? {
            SharedVmGuestControlConnectOutcome::Connected(file) => return Ok(file),
            SharedVmGuestControlConnectOutcome::Retryable(message) => {
                if std::time::Instant::now() >= deadline {
                    bail!(
                        "timed out waiting for guest vsock port {} after transient connect errors: {}",
                        SHARED_VM_GUEST_CONTROL_VSOCK_PORT,
                        message
                    );
                }
                std::thread::sleep(GUEST_EXEC_CONNECT_RETRY_INTERVAL);
            }
            SharedVmGuestControlConnectOutcome::Fatal(message) => {
                return Err(anyhow::anyhow!(message)).with_context(|| {
                    format!(
                        "connecting to guest vsock port {}",
                        SHARED_VM_GUEST_CONTROL_VSOCK_PORT
                    )
                });
            }
        }
    }
}

#[cfg(all(target_os = "macos", unix))]
pub(super) fn relay_shared_vm_control_client(client: UnixStream, guest: File) -> Result<()> {
    // The listener itself stays nonblocking so the owner loop can poll `accept()`, but the
    // per-client relay must switch back to blocking mode before proxying framed exec traffic.
    // Otherwise large stdin streams like disk-isolated tar imports can race with early guest
    // response frames and spuriously fail on `WouldBlock` while the client has not started
    // reading yet.
    client
        .set_nonblocking(false)
        .context("restoring shared VM control client blocking mode")?;
    let mut client_reader = client;
    let mut guest_reader = guest.try_clone().context("cloning guest control socket")?;
    let client_writer = Arc::new(Mutex::new(
        client_reader
            .try_clone()
            .context("cloning shared VM control client")?,
    ));
    let guest_writer = Arc::new(Mutex::new(guest));

    let _client_forwarder = {
        let guest_writer = Arc::clone(&guest_writer);
        std::thread::spawn(move || loop {
            match read_exec_frame(&mut client_reader) {
                Ok(Some(
                    frame @ (AvfLinuxExecFrame::Request(_)
                    | AvfLinuxExecFrame::Stdin(_)
                    | AvfLinuxExecFrame::CloseStdin
                    | AvfLinuxExecFrame::Resize(_)),
                )) => {
                    let Ok(mut guard) = guest_writer.lock() else {
                        return;
                    };
                    if write_exec_frame(&mut *guard, &frame).is_err() {
                        return;
                    }
                }
                Ok(Some(_)) | Ok(None) => {
                    close_guest_exec_stdin_best_effort(&guest_writer);
                    return;
                }
                Err(_) => {
                    close_guest_exec_stdin_best_effort(&guest_writer);
                    return;
                }
            }
        })
    };

    loop {
        match read_exec_frame(&mut guest_reader) {
            Ok(Some(frame)) => {
                let terminal = matches!(
                    frame,
                    AvfLinuxExecFrame::Exit(_) | AvfLinuxExecFrame::Error(_)
                );
                let mut guard = client_writer.lock().map_err(|_| {
                    anyhow::anyhow!("shared VM control client writer mutex poisoned")
                })?;
                write_exec_frame(&mut *guard, &frame)
                    .context("writing proxied shared VM guest frame")?;
                drop(guard);
                if terminal {
                    return Ok(());
                }
            }
            Ok(None) => {
                let message = "shared VM guest control stream closed before sending an exit frame";
                if let Ok(mut guard) = client_writer.lock() {
                    write_exec_error_frame_best_effort(
                        &mut *guard,
                        "guest_control_stream_closed",
                        message,
                    );
                }
                bail!(message);
            }
            Err(err) => {
                let code = if io_error_is_benign(&err) {
                    "guest_control_stream_closed"
                } else {
                    "guest_control_stream_failed"
                };
                let message = format!("reading shared VM guest response frame failed: {err}");
                if let Ok(mut guard) = client_writer.lock() {
                    write_exec_error_frame_best_effort(&mut *guard, code, &message);
                }
                return Err(err).context("reading shared VM guest response frame");
            }
        }
    }
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
            Ok((mut client, _)) => {
                let guest = match connect_shared_vm_guest_control_socket(queue, virtual_machine) {
                    Ok(guest) => guest,
                    Err(err) => {
                        let message = format!(
                            "connecting to guest vsock port {SHARED_VM_GUEST_CONTROL_VSOCK_PORT}: {err:#}"
                        );
                        append_shared_vm_log_line(data_root, &message)?;
                        let _ = write_exec_frame(
                            &mut client,
                            &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                                code: "guest_control_connect_failed".to_string(),
                                message,
                            }),
                        );
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
struct SharedVmResourceState {
    host_port: libc::mach_port_t,
    next_data_disk_check_at: std::time::Instant,
    last_growth_blocked: bool,
    next_memory_check_at: std::time::Instant,
    memory_ceiling_bytes: u64,
    memory_floor_bytes: u64,
}

#[cfg(target_os = "macos")]
impl SharedVmResourceState {
    fn new(memory_ceiling_bytes: u64, memory_floor_bytes: u64) -> Self {
        Self {
            host_port: unsafe { mach_host_self() },
            next_data_disk_check_at: std::time::Instant::now(),
            last_growth_blocked: false,
            next_memory_check_at: std::time::Instant::now(),
            memory_ceiling_bytes,
            memory_floor_bytes,
        }
    }
}

#[cfg(unix)]
fn parse_single_u64_output(stdout: &[u8], context: &str) -> Result<u64> {
    let raw = String::from_utf8_lossy(stdout).trim().to_string();
    raw.parse::<u64>()
        .with_context(|| format!("parsing `{raw}` as an integer for {context}"))
}

#[cfg(unix)]
fn guest_mount_available_bytes(data_root: &Path, mount_path: &str) -> Result<u64> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let result = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            format!("df -B1 {mount_path} | awk 'NR==2 {{print $4}}'"),
        ],
        Some("root"),
        HashMap::new(),
        None,
    )?;
    ensure_guest_exec_success(
        &format!("reading guest free bytes for {mount_path}"),
        GuestExecCaptureResult {
            exit_code: result.exit_code,
            stdout: result.stdout.clone(),
            stderr: result.stderr.clone(),
        },
    )?;
    parse_single_u64_output(
        &result.stdout,
        &format!("guest free bytes for {mount_path}"),
    )
}

#[cfg(unix)]
fn guest_memory_available_bytes(data_root: &Path) -> Result<u64> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let result = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            "awk '/MemAvailable:/ { print $2 * 1024 }' /proc/meminfo".to_string(),
        ],
        Some("root"),
        HashMap::new(),
        None,
    )?;
    ensure_guest_exec_success(
        "reading guest MemAvailable bytes",
        GuestExecCaptureResult {
            exit_code: result.exit_code,
            stdout: result.stdout.clone(),
            stderr: result.stderr.clone(),
        },
    )?;
    parse_single_u64_output(&result.stdout, "guest MemAvailable bytes")
}

#[cfg(unix)]
fn compact_guest_memory_best_effort(data_root: &Path) {
    let control_socket = shared_vm_control_socket_path(data_root);
    let _ = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            "echo 1 > /proc/sys/vm/compact_memory 2>/dev/null || true".to_string(),
        ],
        Some("root"),
        HashMap::new(),
        None,
    );
}

#[cfg(unix)]
fn guest_data_disk_device_path(data_root: &Path) -> Result<String> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let result = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/bin/sh",
        &["-lc".to_string(), "findmnt -n -o SOURCE /ctx".to_string()],
        Some("root"),
        HashMap::new(),
        None,
    )?;
    ensure_guest_exec_success(
        "reading guest data-disk device for /ctx",
        GuestExecCaptureResult {
            exit_code: result.exit_code,
            stdout: result.stdout.clone(),
            stderr: result.stderr.clone(),
        },
    )?;
    let device = String::from_utf8_lossy(&result.stdout).trim().to_string();
    if device.is_empty() {
        bail!("guest data disk mount `/ctx` resolved to an empty device path");
    }
    Ok(device)
}

#[cfg(unix)]
fn grow_guest_data_disk_filesystem(data_root: &Path, device_path: &str) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let device_path_escaped = shell_escape_single_quotes(device_path);
    let result = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            format!(
                "set -eu; device='{device_path}'; device_name=\"$(basename \"$device\")\"; echo 1 > \"/sys/class/block/$device_name/device/rescan\" 2>/dev/null || true; blockdev --rereadpt \"$device\" 2>/dev/null || true; resize2fs \"$device\"",
                device_path = device_path_escaped,
            ),
        ],
        Some("root"),
        HashMap::new(),
        None,
    )?;
    ensure_guest_exec_success(
        &format!("growing guest data-disk filesystem on {device_path}"),
        result,
    )
}

#[cfg(unix)]
fn host_available_bytes(path: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt;

    let path_cstr = std::ffi::CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("building C string for {}", path.display()))?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    let status = unsafe { libc::statfs(path_cstr.as_ptr(), stat.as_mut_ptr()) };
    if status != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("reading filesystem stats for {}", path.display()));
    }
    let stat = unsafe { stat.assume_init() };
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_bsize as u64))
}

#[cfg(target_os = "macos")]
fn host_available_memory_bytes(host_port: libc::mach_port_t) -> Result<u64> {
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        bail!("sysconf(_SC_PAGESIZE) returned an invalid page size");
    }

    let mut stats = std::mem::MaybeUninit::<libc::vm_statistics64_data_t>::uninit();
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let status = unsafe {
        libc::host_statistics64(
            host_port,
            libc::HOST_VM_INFO64,
            stats.as_mut_ptr().cast(),
            &mut count,
        )
    };
    if status != 0 {
        bail!("host_statistics64(HOST_VM_INFO64) failed with kern_return_t {status}");
    }
    let stats = unsafe { stats.assume_init() };
    let available_pages = u64::from(stats.free_count)
        .saturating_add(u64::from(stats.inactive_count))
        .saturating_add(u64::from(stats.speculative_count));
    Ok(available_pages.saturating_mul(page_size as u64))
}

#[cfg(target_os = "macos")]
fn shared_vm_memory_target_bytes_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<u64> {
    let virtual_machine_addr = virtual_machine as usize;
    exec_on_dispatch_queue(
        queue,
        "shared AVF Linux VM memory target dispatch",
        move || -> Result<u64> {
            let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
            let balloon_devices: Retained<NSArray<VZMemoryBalloonDevice>> =
                unsafe { virtual_machine.memoryBalloonDevices() };
            let Some(balloon_device) = balloon_devices.iter().next() else {
                bail!("shared AVF Linux VM has no memory balloon devices configured");
            };
            let balloon_device = unsafe {
                &*((&*balloon_device) as *const _ as *const VZVirtioTraditionalMemoryBalloonDevice)
            };
            Ok(unsafe { balloon_device.targetVirtualMachineMemorySize() })
        },
    )?
}

#[cfg(target_os = "macos")]
fn request_shared_vm_memory_target_bytes_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
    target_bytes: u64,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    exec_on_dispatch_queue(
        queue,
        "shared AVF Linux VM memory target request dispatch",
        move || -> Result<()> {
            let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
            let balloon_devices: Retained<NSArray<VZMemoryBalloonDevice>> =
                unsafe { virtual_machine.memoryBalloonDevices() };
            let Some(balloon_device) = balloon_devices.iter().next() else {
                bail!("shared AVF Linux VM has no memory balloon devices configured");
            };
            let balloon_device = unsafe {
                &*((&*balloon_device) as *const _ as *const VZVirtioTraditionalMemoryBalloonDevice)
            };
            unsafe {
                balloon_device.setTargetVirtualMachineMemorySize(target_bytes);
            }
            Ok(())
        },
    )?
}

#[cfg(target_os = "macos")]
fn maybe_grow_shared_vm_data_disk(
    data_root: &Path,
    resource_state: &mut SharedVmResourceState,
) -> Result<()> {
    let now = std::time::Instant::now();
    if now < resource_state.next_data_disk_check_at {
        return Ok(());
    }
    resource_state.next_data_disk_check_at = now + SHARED_VM_DATA_DISK_POLL_INTERVAL;

    let data_disk_path = shared_vm_data_disk_path(data_root);
    let current_size_bytes = fs::metadata(&data_disk_path)
        .with_context(|| format!("reading {}", data_disk_path.display()))?
        .len();
    let guest_free_bytes = guest_mount_available_bytes(data_root, "/ctx")?;
    let host_available_bytes = host_available_bytes(&data_disk_path)?;

    match resolve_shared_vm_data_disk_growth_decision(
        current_size_bytes,
        guest_free_bytes,
        host_available_bytes,
    ) {
        SharedVmDataDiskGrowthDecision::NoAction => {
            resource_state.last_growth_blocked = false;
            Ok(())
        }
        SharedVmDataDiskGrowthDecision::Grow {
            new_size_bytes,
            additional_bytes,
        } => {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(&data_disk_path)
                .with_context(|| format!("opening {} for growth", data_disk_path.display()))?;
            file.set_len(new_size_bytes).with_context(|| {
                format!(
                    "growing shared AVF data disk {} to {} bytes",
                    data_disk_path.display(),
                    new_size_bytes
                )
            })?;
            let device_path = guest_data_disk_device_path(data_root)?;
            grow_guest_data_disk_filesystem(data_root, &device_path)?;
            resource_state.last_growth_blocked = false;
            append_shared_vm_log_line(
                data_root,
                &format!(
                    "grew AVF data disk {} by {:.2} GiB to {:.2} GiB after guest free space fell to {:.2} GiB",
                    data_disk_path.display(),
                    additional_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    new_size_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    guest_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                ),
            )?;
            Ok(())
        }
        SharedVmDataDiskGrowthDecision::HostReserveBlocked {
            available_host_bytes,
            reserve_bytes,
            requested_additional_bytes,
        } => {
            if !resource_state.last_growth_blocked {
                append_shared_vm_log_line(
                    data_root,
                    &format!(
                        "AVF data-disk growth is blocked by the host reserve: available_host={:.2} GiB reserve={:.2} GiB requested_growth={:.2} GiB guest_free={:.2} GiB",
                        available_host_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        reserve_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        requested_additional_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                        guest_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    ),
                )?;
            }
            resource_state.last_growth_blocked = true;
            if guest_free_bytes < SHARED_VM_DATA_DISK_CRITICAL_FREE_BYTES {
                bail!(
                    "guest data disk at {} is below the critical free-space floor ({:.2} GiB free) and the host reserve prevents further growth",
                    data_disk_path.display(),
                    guest_free_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
                );
            }
            Ok(())
        }
    }
}

#[cfg(target_os = "macos")]
fn maybe_adjust_shared_vm_memory(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
    data_root: &Path,
    resource_state: &mut SharedVmResourceState,
) -> Result<()> {
    let now = std::time::Instant::now();
    if now < resource_state.next_memory_check_at {
        return Ok(());
    }
    resource_state.next_memory_check_at = now + SHARED_VM_MEMORY_POLL_INTERVAL;

    let current_target_bytes = shared_vm_memory_target_bytes_on_queue(queue, virtual_machine)?;
    let host_available_bytes = host_available_memory_bytes(resource_state.host_port)?;
    let guest_available_bytes = if host_available_bytes < SHARED_VM_HOST_MEMORY_RESERVE_BYTES {
        None
    } else {
        guest_memory_available_bytes(data_root).ok()
    };

    match resolve_shared_vm_memory_balloon_action(
        current_target_bytes,
        resource_state.memory_ceiling_bytes,
        resource_state.memory_floor_bytes,
        guest_available_bytes,
        host_available_bytes,
    ) {
        SharedVmMemoryBalloonAction::NoAction => Ok(()),
        SharedVmMemoryBalloonAction::Reclaim {
            new_target_bytes,
            available_host_bytes,
            aggressive,
        } => {
            compact_guest_memory_best_effort(data_root);
            request_shared_vm_memory_target_bytes_on_queue(
                queue,
                virtual_machine,
                new_target_bytes,
            )?;
            append_shared_vm_log_line(
                data_root,
                &format!(
                    "requested AVF memory reclaim from {:.2} GiB to {:.2} GiB after host available memory fell to {:.2} GiB{}",
                    current_target_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    new_target_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    available_host_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    if aggressive {
                        " (emergency reclaim to the configured floor)"
                    } else {
                        ""
                    }
                ),
            )
        }
        SharedVmMemoryBalloonAction::Grow {
            new_target_bytes,
            available_host_bytes,
            guest_available_bytes,
        } => {
            request_shared_vm_memory_target_bytes_on_queue(
                queue,
                virtual_machine,
                new_target_bytes,
            )?;
            append_shared_vm_log_line(
                data_root,
                &format!(
                    "requested AVF memory growth from {:.2} GiB to {:.2} GiB after guest MemAvailable fell to {:.2} GiB with host available memory at {:.2} GiB",
                    current_target_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    new_target_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    guest_available_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                    available_host_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                ),
            )
        }
        SharedVmMemoryBalloonAction::EmergencyStop {
            available_host_bytes,
            current_target_bytes,
            floor_bytes,
        } => {
            let note = format!(
                "host memory pressure emergency: host available memory fell to {:.2} GiB while the AVF memory target was already at its floor of {:.2} GiB (current target {:.2} GiB)",
                available_host_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                floor_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                current_target_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
            );
            request_shared_vm_memory_pressure_stop(data_root, &note)?;
            bail!("{note}");
        }
    }
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
            if let Err(err) = maybe_grow_shared_vm_data_disk(data_root, &mut resource_state) {
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
                virtual_machine_ptr,
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
    wait_for_control_socket(data_root)?;
    if let Err(err) = wait_for_real_guest_exec_ready(data_root, readiness_timeout) {
        stop_shared_vm_server(child.id());
        return Err(err);
    }
    Ok(child.id())
}

pub(super) fn reset_writable_shared_vm_runtime_state(data_root: &Path) -> Result<()> {
    clear_shared_vm_shutdown_request(data_root);
    clear_shared_vm_memory_pressure_stop_request(data_root);
    for path in [
        shared_vm_control_socket_path(data_root),
        shared_vm_guest_agent_socket_path(data_root),
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

#[cfg(unix)]
pub(super) fn shared_vm_guest_readiness_args() -> Vec<String> {
    vec![
        String::from("-lc"),
        format!(
            "set -e; systemctl is-active --quiet {containerd_service}; systemctl is-active --quiet {buildkit_service}; {nerdctl_bin} version >/dev/null 2>&1; {buildctl_bin} --addr {buildkit_socket} debug workers >/dev/null 2>&1; probe_bridge=ctxavfbr0; ip link delete \"$probe_bridge\" >/dev/null 2>&1 || true; if ! ip link add name \"$probe_bridge\" type bridge >/tmp/ctx-avf-bridge-probe.out 2>/tmp/ctx-avf-bridge-probe.err; then cat /tmp/ctx-avf-bridge-probe.out >&2 || true; cat /tmp/ctx-avf-bridge-probe.err >&2 || true; echo \"[ctx-avf-linux] bridge_probe_failed\" >&2; exit 41; fi; ip link delete \"$probe_bridge\" >/dev/null 2>&1 || true",
            containerd_service = SHARED_VM_CONTAINERD_SERVICE_NAME,
            buildkit_service = SHARED_VM_BUILDKIT_SERVICE_NAME,
            nerdctl_bin = SHARED_VM_GUEST_NERDCTL_BIN,
            buildctl_bin = SHARED_VM_GUEST_BUILDKITCTL_BIN,
            buildkit_socket = SHARED_VM_GUEST_BUILDKIT_SOCKET,
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
pub(super) fn wait_for_real_guest_exec_ready(data_root: &Path, timeout: Duration) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let deadline = std::time::Instant::now() + timeout;
    let mut last_err: Option<anyhow::Error> = None;
    let readiness_args = shared_vm_guest_readiness_args();
    while std::time::Instant::now() < deadline {
        match run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/sh",
            &readiness_args,
            Some("root"),
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
pub(super) fn wait_for_real_guest_exec_ready(_data_root: &Path, _timeout: Duration) -> Result<()> {
    Ok(())
}
