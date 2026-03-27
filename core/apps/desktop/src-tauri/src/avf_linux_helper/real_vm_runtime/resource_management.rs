use super::*;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_foundation::NSArray;
#[cfg(target_os = "macos")]
use objc2_virtualization::{VZMemoryBalloonDevice, VZVirtioTraditionalMemoryBalloonDevice};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharedVmDataDiskGrowthDecision {
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

pub(crate) fn resolve_shared_vm_data_disk_growth_decision(
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

pub(super) fn align_down_to_mebibyte(bytes: u64) -> u64 {
    bytes - (bytes % MEBIBYTE_BYTES)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SharedVmMemoryBalloonAction {
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

pub(crate) fn resolve_shared_vm_memory_balloon_action(
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
pub(crate) enum SharedVmMemoryWatchdogSampleAction {
    NoAction {
        next_consecutive_emergency_samples: u32,
    },
    RequestStop {
        next_consecutive_emergency_samples: u32,
        available_host_bytes: u64,
    },
}

pub(crate) fn resolve_shared_vm_memory_watchdog_sample_action(
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
pub(crate) enum SharedVmMemoryWatchdogExitAction {
    OwnerExitedAfterRequest,
    OwnerExitedAfterSigterm,
    EscalateToSigkill,
}

pub(crate) fn resolve_shared_vm_memory_watchdog_exit_action(
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
pub(super) struct SharedVmResourceState {
    host_port: libc::mach_port_t,
    next_data_disk_check_at: std::time::Instant,
    last_growth_blocked: bool,
    next_memory_check_at: std::time::Instant,
    memory_ceiling_bytes: u64,
    memory_floor_bytes: u64,
    memory_controller_decision_trace: SharedVmMemoryControllerDecisionTrace,
}

#[cfg(target_os = "macos")]
impl SharedVmResourceState {
    pub(super) fn new(memory_ceiling_bytes: u64, memory_floor_bytes: u64) -> Self {
        Self {
            host_port: unsafe { mach_host_self() },
            next_data_disk_check_at: std::time::Instant::now(),
            last_growth_blocked: false,
            next_memory_check_at: std::time::Instant::now(),
            memory_ceiling_bytes,
            memory_floor_bytes,
            memory_controller_decision_trace: SharedVmMemoryControllerDecisionTrace::new(),
        }
    }
}

#[cfg(target_os = "macos")]
struct SharedVmMemoryControllerDecisionTrace {
    trace_id: String,
    epoch_millis: u64,
    next_sequence: u64,
}

#[cfg(target_os = "macos")]
impl SharedVmMemoryControllerDecisionTrace {
    fn new() -> Self {
        let epoch_millis = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        {
            Ok(duration) => duration.as_millis().min(u128::from(u64::MAX)) as u64,
            Err(_) => 0,
        };
        Self {
            trace_id: format!(
                "shared-vm-memory-controller-{}-{}",
                std::process::id(),
                epoch_millis
            ),
            epoch_millis,
            next_sequence: 0,
        }
    }

    fn next_sequence(&mut self) -> u64 {
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.next_sequence
    }
}

#[cfg(unix)]
fn parse_single_u64_output(stdout: &[u8], context: &str) -> Result<u64> {
    let raw = String::from_utf8_lossy(stdout).trim().to_string();
    raw.parse::<u64>()
        .with_context(|| format!("parsing `{raw}` as an integer for {context}"))
}

#[cfg(all(target_os = "macos", unix))]
fn guest_mount_available_bytes(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
    mount_path: &str,
) -> Result<u64> {
    let result = run_owner_guest_exec_capture(
        queue,
        virtual_machine,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            format!("df -B1 {mount_path} | awk 'NR==2 {{print $4}}'"),
        ],
        Some("root"),
        HashMap::new(),
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

#[cfg(all(target_os = "macos", unix))]
fn guest_memory_available_bytes(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
) -> Result<u64> {
    let result = run_owner_guest_exec_capture(
        queue,
        virtual_machine,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            "awk '/MemAvailable:/ { print $2 * 1024 }' /proc/meminfo".to_string(),
        ],
        Some("root"),
        HashMap::new(),
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

#[cfg(all(target_os = "macos", unix))]
fn compact_guest_memory_best_effort(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
) {
    let _ = run_owner_guest_exec_capture(
        queue,
        virtual_machine,
        Path::new("/"),
        "/bin/sh",
        &[
            "-lc".to_string(),
            "echo 1 > /proc/sys/vm/compact_memory 2>/dev/null || true".to_string(),
        ],
        Some("root"),
        HashMap::new(),
    );
}

#[cfg(all(target_os = "macos", unix))]
fn guest_data_disk_device_path(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
) -> Result<String> {
    let result = run_owner_guest_exec_capture(
        queue,
        virtual_machine,
        Path::new("/"),
        "/bin/sh",
        &["-lc".to_string(), "findmnt -n -o SOURCE /ctx".to_string()],
        Some("root"),
        HashMap::new(),
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

#[cfg(all(target_os = "macos", unix))]
fn grow_guest_data_disk_filesystem(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
    device_path: &str,
) -> Result<()> {
    let device_path_escaped = shell_escape_single_quotes(device_path);
    let result = run_owner_guest_exec_capture(
        queue,
        virtual_machine,
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
pub(super) fn host_available_memory_bytes(host_port: libc::mach_port_t) -> Result<u64> {
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
fn shared_vm_memory_controller_decision_reason(
    action: &SharedVmMemoryBalloonAction,
    guest_probe_ready: bool,
    current_target_bytes: u64,
    ceiling_bytes: u64,
    floor_bytes: u64,
    guest_available_bytes: Option<u64>,
    host_available_bytes: u64,
) -> &'static str {
    match action {
        SharedVmMemoryBalloonAction::NoAction => {
            if !guest_probe_ready {
                "guest_probe_not_ready"
            } else if host_available_bytes < SHARED_VM_HOST_MEMORY_RESERVE_BYTES
                && current_target_bytes <= floor_bytes
            {
                "at_floor_while_host_below_reserve"
            } else if let Some(guest_available_bytes) = guest_available_bytes {
                if guest_available_bytes >= SHARED_VM_GUEST_MEMORY_GROW_THRESHOLD_BYTES {
                    "guest_memory_above_growth_threshold"
                } else if current_target_bytes >= ceiling_bytes {
                    "at_memory_ceiling"
                } else if host_available_bytes
                    <= SHARED_VM_HOST_MEMORY_RESERVE_BYTES + SHARED_VM_MEMORY_BALLOON_STEP_BYTES
                {
                    "host_growth_budget_exhausted"
                } else {
                    "no_action"
                }
            } else {
                "guest_memory_unavailable"
            }
        }
        SharedVmMemoryBalloonAction::Reclaim { aggressive, .. } => {
            if *aggressive {
                "host_memory_emergency"
            } else {
                "host_memory_below_reserve"
            }
        }
        SharedVmMemoryBalloonAction::Grow { .. } => "guest_memory_below_growth_threshold",
        SharedVmMemoryBalloonAction::EmergencyStop { .. } => "host_memory_emergency_at_floor",
    }
}

#[cfg(target_os = "macos")]
fn shared_vm_memory_controller_reason_codes(reason: &str) -> &'static [&'static str] {
    match reason {
        "host_memory_emergency" | "host_memory_emergency_at_floor" => &["host_emergency"],
        "host_memory_below_reserve"
        | "at_floor_while_host_below_reserve"
        | "host_growth_budget_exhausted" => &["host_pressure_reclaim"],
        "guest_memory_below_growth_threshold" | "at_memory_ceiling" => &["guest_demand_grow"],
        _ => &["stable_band"],
    }
}

#[cfg(target_os = "macos")]
fn shared_vm_host_pressure_state_name(host_available_bytes: u64) -> &'static str {
    if host_available_bytes < SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES {
        "emergency"
    } else if host_available_bytes < SHARED_VM_HOST_MEMORY_RESERVE_BYTES {
        "elevated"
    } else {
        "normal"
    }
}

#[cfg(target_os = "macos")]
fn shared_vm_memory_pressure_state_before(
    host_available_bytes: u64,
    guest_available_bytes: Option<u64>,
) -> &'static str {
    if host_available_bytes < SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES {
        "emergency"
    } else if host_available_bytes < SHARED_VM_HOST_MEMORY_RESERVE_BYTES {
        "host_reclaim"
    } else if guest_available_bytes
        .map(|value| value < SHARED_VM_GUEST_MEMORY_GROW_THRESHOLD_BYTES)
        .unwrap_or(false)
    {
        "guest_protected"
    } else {
        "balanced"
    }
}

#[cfg(target_os = "macos")]
fn shared_vm_memory_pressure_state_after(
    action: &SharedVmMemoryBalloonAction,
    reason: &str,
    host_available_bytes: u64,
    guest_available_bytes: Option<u64>,
) -> &'static str {
    match action {
        SharedVmMemoryBalloonAction::Reclaim {
            aggressive: true, ..
        }
        | SharedVmMemoryBalloonAction::EmergencyStop { .. } => "emergency",
        SharedVmMemoryBalloonAction::Reclaim { .. } => "host_reclaim",
        SharedVmMemoryBalloonAction::Grow { .. } => "guest_protected",
        SharedVmMemoryBalloonAction::NoAction => {
            if matches!(
                reason,
                "host_memory_emergency" | "host_memory_emergency_at_floor"
            ) {
                "emergency"
            } else if matches!(
                reason,
                "host_memory_below_reserve"
                    | "at_floor_while_host_below_reserve"
                    | "host_growth_budget_exhausted"
            ) {
                "host_reclaim"
            } else {
                shared_vm_memory_pressure_state_before(host_available_bytes, guest_available_bytes)
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn append_shared_vm_memory_controller_decision_event(
    data_root: &Path,
    resource_state: &mut SharedVmResourceState,
    guest_probe_ready: bool,
    current_target_bytes: u64,
    host_available_bytes: u64,
    guest_available_bytes: Option<u64>,
    action: &SharedVmMemoryBalloonAction,
) {
    let ceiling_bytes =
        align_down_to_mebibyte(resource_state.memory_ceiling_bytes.max(MEBIBYTE_BYTES));
    let floor_bytes = align_down_to_mebibyte(resource_state.memory_floor_bytes.max(MEBIBYTE_BYTES))
        .min(ceiling_bytes);
    let current_target_bytes =
        align_down_to_mebibyte(current_target_bytes).clamp(floor_bytes, ceiling_bytes);
    let reason = shared_vm_memory_controller_decision_reason(
        action,
        guest_probe_ready,
        current_target_bytes,
        ceiling_bytes,
        floor_bytes,
        guest_available_bytes,
        host_available_bytes,
    );
    let (action_name, new_target_bytes, aggressive) = match action {
        SharedVmMemoryBalloonAction::NoAction => ("no_action", None, None),
        SharedVmMemoryBalloonAction::Reclaim {
            new_target_bytes,
            aggressive,
            ..
        } => ("reclaim", Some(*new_target_bytes), Some(*aggressive)),
        SharedVmMemoryBalloonAction::Grow {
            new_target_bytes, ..
        } => ("grow", Some(*new_target_bytes), None),
        SharedVmMemoryBalloonAction::EmergencyStop {
            current_target_bytes,
            ..
        } => ("emergency_stop", Some(*current_target_bytes), None),
    };
    let target_bytes_after = new_target_bytes.unwrap_or(current_target_bytes);
    let sequence = resource_state.memory_controller_decision_trace.next_sequence();
    let new_target_bytes = new_target_bytes
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let reason_codes = shared_vm_memory_controller_reason_codes(reason)
        .iter()
        .map(|code| format!("\"{code}\""))
        .collect::<Vec<_>>()
        .join(",");
    let pressure_state_before =
        shared_vm_memory_pressure_state_before(host_available_bytes, guest_available_bytes);
    let pressure_state_after =
        shared_vm_memory_pressure_state_after(action, reason, host_available_bytes, guest_available_bytes);
    let host_pressure_state = shared_vm_host_pressure_state_name(host_available_bytes);
    let guest_available_bytes = guest_available_bytes
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let aggressive = aggressive
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let event = format!(
        concat!(
            "{{\"event\":\"ControllerDecisionEvent\",\"version\":1,",
            "\"schema_version\":1,",
            "\"trace_id\":\"{}\",\"epoch\":{},\"epoch_id\":{},\"sequence\":{},\"decision_seq\":{},",
            "\"action\":\"{}\",\"reason\":\"{}\",\"reason_codes\":[{}],",
            "\"target_bytes_before\":{},\"target_bytes_after\":{},",
            "\"pressure_state_before\":\"{}\",\"pressure_state_after\":\"{}\",",
            "\"host\":{{\"pressure_state\":\"{}\",\"available_bytes\":{}}},",
            "\"guest\":{{\"available_bytes\":{}}},",
            "\"context\":{{",
            "\"current_target_bytes\":{},\"new_target_bytes\":{},",
            "\"floor_bytes\":{},\"ceiling_bytes\":{},",
            "\"host_available_bytes\":{},\"guest_available_bytes\":{},",
            "\"guest_probe_ready\":{},\"aggressive\":{},",
            "\"host_memory_reserve_bytes\":{},\"host_memory_emergency_bytes\":{},",
            "\"guest_memory_grow_threshold_bytes\":{},\"memory_balloon_step_bytes\":{}",
            "}}}}"
        ),
        resource_state.memory_controller_decision_trace.trace_id,
        resource_state.memory_controller_decision_trace.epoch_millis,
        resource_state.memory_controller_decision_trace.epoch_millis,
        sequence,
        sequence,
        action_name,
        reason,
        reason_codes,
        current_target_bytes,
        target_bytes_after,
        pressure_state_before,
        pressure_state_after,
        host_pressure_state,
        host_available_bytes,
        guest_available_bytes,
        current_target_bytes,
        new_target_bytes,
        floor_bytes,
        ceiling_bytes,
        host_available_bytes,
        guest_available_bytes,
        guest_probe_ready,
        aggressive,
        SHARED_VM_HOST_MEMORY_RESERVE_BYTES,
        SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES,
        SHARED_VM_GUEST_MEMORY_GROW_THRESHOLD_BYTES,
        SHARED_VM_MEMORY_BALLOON_STEP_BYTES,
    );
    let _ = append_shared_vm_log_line(data_root, &event);
}

#[cfg(target_os = "macos")]
pub(super) fn maybe_grow_shared_vm_data_disk(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
    data_root: &Path,
    resource_state: &mut SharedVmResourceState,
) -> Result<()> {
    if !shared_vm_owner_guest_probe_ready(data_root) {
        return Ok(());
    }
    let now = std::time::Instant::now();
    if now < resource_state.next_data_disk_check_at {
        return Ok(());
    }
    resource_state.next_data_disk_check_at = now + SHARED_VM_DATA_DISK_POLL_INTERVAL;

    let data_disk_path = shared_vm_data_disk_path(data_root);
    let current_size_bytes = fs::metadata(&data_disk_path)
        .with_context(|| format!("reading {}", data_disk_path.display()))?
        .len();
    let guest_free_bytes = guest_mount_available_bytes(queue, virtual_machine, "/ctx")?;
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
            let device_path = guest_data_disk_device_path(queue, virtual_machine)?;
            grow_guest_data_disk_filesystem(queue, virtual_machine, &device_path)?;
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
pub(super) fn maybe_adjust_shared_vm_memory(
    queue: &DispatchQueue,
    virtual_machine: &Retained<VZVirtualMachine>,
    data_root: &Path,
    resource_state: &mut SharedVmResourceState,
) -> Result<()> {
    let virtual_machine_ptr = &**virtual_machine as *const VZVirtualMachine;
    let guest_probe_ready = shared_vm_owner_guest_probe_ready(data_root);
    let now = std::time::Instant::now();
    if now < resource_state.next_memory_check_at {
        return Ok(());
    }
    resource_state.next_memory_check_at = now + SHARED_VM_MEMORY_POLL_INTERVAL;

    let current_target_bytes = shared_vm_memory_target_bytes_on_queue(queue, virtual_machine_ptr)?;
    let host_available_bytes = host_available_memory_bytes(resource_state.host_port)?;
    let guest_available_bytes =
        if !guest_probe_ready || host_available_bytes < SHARED_VM_HOST_MEMORY_RESERVE_BYTES {
            None
        } else {
            guest_memory_available_bytes(queue, virtual_machine).ok()
        };
    let action = resolve_shared_vm_memory_balloon_action(
        current_target_bytes,
        resource_state.memory_ceiling_bytes,
        resource_state.memory_floor_bytes,
        guest_available_bytes,
        host_available_bytes,
    );
    append_shared_vm_memory_controller_decision_event(
        data_root,
        resource_state,
        guest_probe_ready,
        current_target_bytes,
        host_available_bytes,
        guest_available_bytes,
        &action,
    );

    match action {
        SharedVmMemoryBalloonAction::NoAction => Ok(()),
        SharedVmMemoryBalloonAction::Reclaim {
            new_target_bytes,
            available_host_bytes,
            aggressive,
        } => {
            compact_guest_memory_best_effort(queue, virtual_machine);
            request_shared_vm_memory_target_bytes_on_queue(
                queue,
                virtual_machine_ptr,
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
                virtual_machine_ptr,
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
