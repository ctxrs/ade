use super::*;

#[cfg(target_os = "macos")]
fn build_shared_data_root_device(
    data_root: &Path,
) -> Result<Retained<VZVirtioFileSystemDeviceConfiguration>> {
    let shared_dir_url = file_url_for_path(data_root);
    let shared_dir = unsafe {
        VZSharedDirectory::initWithURL_readOnly(VZSharedDirectory::alloc(), &shared_dir_url, false)
    };
    let share = unsafe {
        VZSingleDirectoryShare::initWithDirectory(VZSingleDirectoryShare::alloc(), &shared_dir)
    };
    let device = unsafe {
        VZVirtioFileSystemDeviceConfiguration::initWithTag(
            VZVirtioFileSystemDeviceConfiguration::alloc(),
            &NSString::from_str(SHARED_VM_DATA_ROOT_SHARE_TAG),
        )
    };
    unsafe {
        device.setShare(Some(share.as_super()));
    }
    Ok(device)
}

const MEBIBYTE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResolvedAvfVmSizing {
    pub(super) cpu_count: usize,
    pub(super) memory_size_bytes: u64,
    pub(super) policy_note: String,
}

fn align_down_to_mebibyte(bytes: u64) -> u64 {
    bytes - (bytes % MEBIBYTE_BYTES)
}

fn align_up_to_mebibyte(bytes: u64) -> u64 {
    if bytes == 0 {
        return 0;
    }
    let remainder = bytes % MEBIBYTE_BYTES;
    if remainder == 0 {
        bytes
    } else {
        bytes + (MEBIBYTE_BYTES - remainder)
    }
}

pub(super) fn resolve_avf_vm_sizing(
    min_cpu: usize,
    max_cpu: usize,
    min_memory: u64,
    max_memory: u64,
    host_cpu_count: usize,
    host_memory_bytes: u64,
    cpu_override: Option<usize>,
    memory_override_bytes: Option<u64>,
) -> ResolvedAvfVmSizing {
    let min_cpu = min_cpu.max(1);
    let max_cpu = max_cpu.max(min_cpu);
    let requested_cpu = cpu_override.unwrap_or_else(|| host_cpu_count.max(1));
    let cpu_count = requested_cpu.clamp(min_cpu, max_cpu);

    let min_memory = align_up_to_mebibyte(min_memory.max(MEBIBYTE_BYTES));
    let max_memory = align_down_to_mebibyte(max_memory).max(min_memory);
    let default_memory_bytes = host_memory_bytes
        .saturating_sub(SHARED_VM_HOST_MEMORY_RESERVE_BYTES)
        .max(SHARED_VM_MIN_DEFAULT_MEMORY_BYTES);
    let requested_memory = memory_override_bytes
        .unwrap_or(default_memory_bytes)
        .max(SHARED_VM_MIN_DEFAULT_MEMORY_BYTES);
    let memory_size_bytes = align_down_to_mebibyte(requested_memory).clamp(min_memory, max_memory);

    let cpu_policy = if cpu_override.is_some() {
        format!("{SHARED_VM_CPU_COUNT_ENV} override")
    } else {
        "host logical CPU count".to_string()
    };
    let memory_policy = if memory_override_bytes.is_some() {
        format!("{SHARED_VM_MEMORY_CEILING_BYTES_ENV} override")
    } else {
        format!(
            "host RAM minus {} MiB reserve",
            SHARED_VM_HOST_MEMORY_RESERVE_BYTES / MEBIBYTE_BYTES
        )
    };

    ResolvedAvfVmSizing {
        cpu_count,
        memory_size_bytes,
        policy_note: format!(
            "vm sizing policy: cpu={cpu_policy}, bounded by AVF limits; memory ceiling={memory_policy}, bounded by AVF limits and a {} MiB floor",
            SHARED_VM_MIN_DEFAULT_MEMORY_BYTES / MEBIBYTE_BYTES
        ),
    }
}

#[cfg(target_os = "macos")]
fn read_optional_env_usize(name: &str) -> Result<Option<usize>> {
    match std::env::var(name) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                bail!("{name} is set but empty");
            }
            let value = trimmed
                .parse::<usize>()
                .with_context(|| format!("parsing {name} as an integer"))?;
            if value == 0 {
                bail!("{name} must be greater than zero");
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{name} is not valid UTF-8"),
    }
}

#[cfg(target_os = "macos")]
fn read_optional_env_u64(name: &str) -> Result<Option<u64>> {
    match std::env::var(name) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                bail!("{name} is set but empty");
            }
            let value = trimmed
                .parse::<u64>()
                .with_context(|| format!("parsing {name} as an integer"))?;
            if value == 0 {
                bail!("{name} must be greater than zero");
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => bail!("{name} is not valid UTF-8"),
    }
}

#[cfg(target_os = "macos")]
fn read_sysctl_u64(name: &str) -> Result<u64> {
    let name_cstr =
        std::ffi::CString::new(name).with_context(|| format!("building sysctl name {name}"))?;
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let status = unsafe {
        libc::sysctlbyname(
            name_cstr.as_ptr(),
            (&mut value as *mut u64).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if status != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("reading sysctl {name}"));
    }
    if size == 0 {
        bail!("sysctl {name} returned no data");
    }
    Ok(value)
}

#[cfg(target_os = "macos")]
fn host_logical_cpu_count() -> Result<usize> {
    let cpu_count = read_sysctl_u64("hw.logicalcpu")? as usize;
    if cpu_count == 0 {
        bail!("hw.logicalcpu reported zero logical CPUs");
    }
    Ok(cpu_count)
}

#[cfg(target_os = "macos")]
fn host_memory_size_bytes() -> Result<u64> {
    let memory_bytes = read_sysctl_u64("hw.memsize")?;
    if memory_bytes == 0 {
        bail!("hw.memsize reported zero bytes");
    }
    Ok(memory_bytes)
}

#[cfg(target_os = "macos")]
pub(super) fn resolved_avf_vm_sizing_for_host(
    min_cpu: usize,
    max_cpu: usize,
    min_memory: u64,
    max_memory: u64,
) -> Result<ResolvedAvfVmSizing> {
    let host_cpu_count = host_logical_cpu_count()?;
    let host_memory_bytes = host_memory_size_bytes()?;
    let cpu_override = read_optional_env_usize(SHARED_VM_CPU_COUNT_ENV)?;
    let memory_override_bytes = read_optional_env_u64(SHARED_VM_MEMORY_CEILING_BYTES_ENV)?;
    Ok(resolve_avf_vm_sizing(
        min_cpu,
        max_cpu,
        min_memory,
        max_memory,
        host_cpu_count,
        host_memory_bytes,
        cpu_override,
        memory_override_bytes,
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn validate_real_avf_linux_vm_configuration(
    data_root: &Path,
    rootfs_image: &Path,
    data_disk_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    kernel_cmdline: &str,
) -> Result<String> {
    if !unsafe { VZVirtualMachine::isSupported() } {
        bail!("Virtualization.framework reported that virtualization is unavailable on this host");
    }

    let kernel_url = file_url_for_path(kernel_path);
    let initrd_url = file_url_for_path(initrd_path);
    let rootfs_url = file_url_for_path(rootfs_image);
    let data_disk_url = file_url_for_path(data_disk_image);

    let boot_loader =
        unsafe { VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url) };
    unsafe {
        boot_loader.setCommandLine(&NSString::from_str(kernel_cmdline));
        boot_loader.setInitialRamdiskURL(Some(&initrd_url));
    }

    let root_storage_attachment = unsafe {
        VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &rootfs_url,
            false,
            VZDiskImageCachingMode::Automatic,
            VZDiskImageSynchronizationMode::Fsync,
        )
    }
    .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    let root_storage_device = unsafe {
        VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            root_storage_attachment.as_super(),
        )
    };
    let data_storage_attachment = unsafe {
        VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &data_disk_url,
            false,
            VZDiskImageCachingMode::Automatic,
            VZDiskImageSynchronizationMode::Fsync,
        )
    }
    .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    let data_storage_device = unsafe {
        VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            data_storage_attachment.as_super(),
        )
    };
    let storage_devices: Retained<NSArray<VZStorageDeviceConfiguration>> =
        NSArray::from_slice(&[root_storage_device.as_super(), data_storage_device.as_super()]);

    let nat_attachment = unsafe { VZNATNetworkDeviceAttachment::new() };
    let network_device = unsafe { VZVirtioNetworkDeviceConfiguration::new() };
    unsafe {
        network_device.setAttachment(Some(nat_attachment.as_super()));
    }
    let network_devices: Retained<NSArray<VZNetworkDeviceConfiguration>> =
        NSArray::from_slice(&[network_device.as_super()]);

    let socket_device = unsafe { VZVirtioSocketDeviceConfiguration::new() };
    let socket_devices: Retained<NSArray<VZSocketDeviceConfiguration>> =
        NSArray::from_slice(&[socket_device.as_super()]);
    let balloon_device = unsafe { VZVirtioTraditionalMemoryBalloonDeviceConfiguration::new() };
    let balloon_devices: Retained<NSArray<VZMemoryBalloonDeviceConfiguration>> =
        NSArray::from_slice(&[balloon_device.as_super()]);
    let shared_data_root_device = build_shared_data_root_device(data_root)?;
    let directory_sharing_devices: Retained<NSArray<VZDirectorySharingDeviceConfiguration>> =
        NSArray::from_slice(&[shared_data_root_device.as_super()]);

    let configuration = unsafe { VZVirtualMachineConfiguration::new() };
    let platform = unsafe { VZGenericPlatformConfiguration::new() };
    let min_cpu = unsafe { VZVirtualMachineConfiguration::minimumAllowedCPUCount() };
    let max_cpu = unsafe { VZVirtualMachineConfiguration::maximumAllowedCPUCount() };
    let min_memory = unsafe { VZVirtualMachineConfiguration::minimumAllowedMemorySize() };
    let max_memory = unsafe { VZVirtualMachineConfiguration::maximumAllowedMemorySize() };
    let sizing = resolved_avf_vm_sizing_for_host(min_cpu, max_cpu, min_memory, max_memory)?;
    unsafe {
        configuration.setBootLoader(Some(boot_loader.as_super()));
        configuration.setPlatform(platform.as_super());
        configuration.setCPUCount(sizing.cpu_count);
        configuration.setMemorySize(sizing.memory_size_bytes);
        configuration.setStorageDevices(&storage_devices);
        configuration.setNetworkDevices(&network_devices);
        configuration.setSocketDevices(&socket_devices);
        configuration.setMemoryBalloonDevices(&balloon_devices);
        configuration.setDirectorySharingDevices(&directory_sharing_devices);
        configuration
            .validateWithError()
            .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    }

    let save_restore_note = if shared_vm_save_restore_supported() {
        #[cfg(target_arch = "aarch64")]
        {
            match unsafe { configuration.validateSaveRestoreSupportWithError() } {
                Ok(()) => "save/restore supported".to_string(),
                Err(err) => format!(
                    "save/restore unavailable for this VM configuration: {}",
                    format_nserror(&err)
                ),
            }
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            "save/restore unavailable on this host".to_string()
        }
    } else {
        "save/restore unavailable on this host".to_string()
    };

    Ok(format!(
        "native AVF configuration validated (cpu={}, memory={} MiB; {}; {})",
        sizing.cpu_count,
        sizing.memory_size_bytes / MEBIBYTE_BYTES,
        sizing.policy_note,
        save_restore_note,
    ))
}

#[cfg(target_os = "macos")]
pub(super) fn load_or_create_shared_vm_machine_identifier(
    data_root: &Path,
) -> Result<Retained<VZGenericMachineIdentifier>> {
    let path = shared_vm_machine_identifier_path(data_root);
    if path.is_file() {
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let data = NSData::from_vec(bytes);
        return unsafe {
            VZGenericMachineIdentifier::initWithDataRepresentation(
                VZGenericMachineIdentifier::alloc(),
                &data,
            )
        }
        .ok_or_else(|| anyhow::anyhow!("invalid AVF machine identifier at {}", path.display()));
    }

    let identifier = unsafe { VZGenericMachineIdentifier::new() };
    let data = unsafe { identifier.dataRepresentation() };
    fs::write(&path, data.to_vec()).with_context(|| format!("writing {}", path.display()))?;
    Ok(identifier)
}

#[cfg(target_os = "macos")]
pub(super) fn load_or_create_shared_vm_mac_address(
    data_root: &Path,
) -> Result<Retained<VZMACAddress>> {
    let path = shared_vm_mac_address_path(data_root);
    if path.is_file() {
        let raw =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let value = raw.trim();
        let ns_value = NSString::from_str(value);
        return unsafe { VZMACAddress::initWithString(VZMACAddress::alloc(), &ns_value) }
            .ok_or_else(|| {
                anyhow::anyhow!("invalid AVF MAC address `{value}` at {}", path.display())
            });
    }

    let address = unsafe { VZMACAddress::randomLocallyAdministeredAddress() };
    let address_string = unsafe { address.string() }.to_string();
    fs::write(&path, address_string).with_context(|| format!("writing {}", path.display()))?;
    Ok(address)
}

#[cfg(target_os = "macos")]
pub(super) fn build_real_avf_linux_vm_configuration(
    data_root: &Path,
    rootfs_image: &Path,
    data_disk_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    seed_image: Option<&Path>,
    kernel_cmdline: &str,
) -> Result<Retained<VZVirtualMachineConfiguration>> {
    if !unsafe { VZVirtualMachine::isSupported() } {
        bail!("Virtualization.framework reported that virtualization is unavailable on this host");
    }

    let kernel_url = file_url_for_path(kernel_path);
    let initrd_url = file_url_for_path(initrd_path);
    let rootfs_url = file_url_for_path(rootfs_image);
    let data_disk_url = file_url_for_path(data_disk_image);

    let boot_loader =
        unsafe { VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url) };
    unsafe {
        boot_loader.setCommandLine(&NSString::from_str(kernel_cmdline));
        boot_loader.setInitialRamdiskURL(Some(&initrd_url));
    }

    let root_storage_attachment = unsafe {
        VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &rootfs_url,
            false,
            VZDiskImageCachingMode::Automatic,
            VZDiskImageSynchronizationMode::Fsync,
        )
    }
    .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    let root_storage_device = unsafe {
        VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            root_storage_attachment.as_super(),
        )
    };
    let data_storage_attachment = unsafe {
        VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &data_disk_url,
            false,
            VZDiskImageCachingMode::Automatic,
            VZDiskImageSynchronizationMode::Fsync,
        )
    }
    .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    let data_storage_device = unsafe {
        VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            data_storage_attachment.as_super(),
        )
    };
    let mut storage_devices_owned = vec![root_storage_device, data_storage_device];
    if let Some(seed_image) = seed_image {
        let seed_url = file_url_for_path(seed_image);
        let seed_storage_attachment = unsafe {
            VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
                VZDiskImageStorageDeviceAttachment::alloc(),
                &seed_url,
                true,
                VZDiskImageCachingMode::Automatic,
                VZDiskImageSynchronizationMode::Fsync,
            )
        }
        .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
        let seed_storage_device = unsafe {
            VZVirtioBlockDeviceConfiguration::initWithAttachment(
                VZVirtioBlockDeviceConfiguration::alloc(),
                seed_storage_attachment.as_super(),
            )
        };
        storage_devices_owned.push(seed_storage_device);
    }
    let storage_device_refs = storage_devices_owned
        .iter()
        .map(|device| device.as_super())
        .collect::<Vec<_>>();
    let storage_devices: Retained<NSArray<VZStorageDeviceConfiguration>> =
        NSArray::from_slice(&storage_device_refs);

    let nat_attachment = unsafe { VZNATNetworkDeviceAttachment::new() };
    let network_device = unsafe { VZVirtioNetworkDeviceConfiguration::new() };
    let mac_address = load_or_create_shared_vm_mac_address(data_root)?;
    unsafe {
        network_device.setAttachment(Some(nat_attachment.as_super()));
        network_device.setMACAddress(&mac_address);
    }
    let network_devices: Retained<NSArray<VZNetworkDeviceConfiguration>> =
        NSArray::from_slice(&[network_device.as_super()]);

    let socket_device = unsafe { VZVirtioSocketDeviceConfiguration::new() };
    let socket_devices: Retained<NSArray<VZSocketDeviceConfiguration>> =
        NSArray::from_slice(&[socket_device.as_super()]);
    let balloon_device = unsafe { VZVirtioTraditionalMemoryBalloonDeviceConfiguration::new() };
    let balloon_devices: Retained<NSArray<VZMemoryBalloonDeviceConfiguration>> =
        NSArray::from_slice(&[balloon_device.as_super()]);
    let shared_data_root_device = build_shared_data_root_device(data_root)?;
    let directory_sharing_devices: Retained<NSArray<VZDirectorySharingDeviceConfiguration>> =
        NSArray::from_slice(&[shared_data_root_device.as_super()]);
    let guest_console_log_path = shared_vm_guest_console_log_path(data_root);
    if let Some(parent) = guest_console_log_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&guest_console_log_path, b"")
        .with_context(|| format!("resetting {}", guest_console_log_path.display()))?;
    let guest_console_url = file_url_for_path(&guest_console_log_path);
    let guest_console_attachment = unsafe {
        VZFileSerialPortAttachment::initWithURL_append_error(
            VZFileSerialPortAttachment::alloc(),
            &guest_console_url,
            true,
        )
    }
    .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    let serial_port = unsafe { VZVirtioConsoleDeviceSerialPortConfiguration::new() };
    unsafe {
        serial_port.setAttachment(Some(guest_console_attachment.as_super()));
    }
    let serial_ports: Retained<NSArray<VZSerialPortConfiguration>> =
        NSArray::from_slice(&[serial_port.as_super()]);

    let configuration = unsafe { VZVirtualMachineConfiguration::new() };
    let platform = unsafe { VZGenericPlatformConfiguration::new() };
    let machine_identifier = load_or_create_shared_vm_machine_identifier(data_root)?;
    let min_cpu = unsafe { VZVirtualMachineConfiguration::minimumAllowedCPUCount() };
    let max_cpu = unsafe { VZVirtualMachineConfiguration::maximumAllowedCPUCount() };
    let min_memory = unsafe { VZVirtualMachineConfiguration::minimumAllowedMemorySize() };
    let max_memory = unsafe { VZVirtualMachineConfiguration::maximumAllowedMemorySize() };
    let sizing = resolved_avf_vm_sizing_for_host(min_cpu, max_cpu, min_memory, max_memory)?;
    unsafe {
        configuration.setBootLoader(Some(boot_loader.as_super()));
        platform.setMachineIdentifier(&machine_identifier);
        configuration.setPlatform(platform.as_super());
        configuration.setCPUCount(sizing.cpu_count);
        configuration.setMemorySize(sizing.memory_size_bytes);
        configuration.setStorageDevices(&storage_devices);
        configuration.setNetworkDevices(&network_devices);
        configuration.setSerialPorts(&serial_ports);
        configuration.setSocketDevices(&socket_devices);
        configuration.setMemoryBalloonDevices(&balloon_devices);
        configuration.setDirectorySharingDevices(&directory_sharing_devices);
        configuration
            .validateWithError()
            .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    }
    Ok(configuration)
}

#[cfg(target_os = "macos")]
pub(super) fn build_real_avf_linux_virtual_machine(
    data_root: &Path,
    rootfs_image: &Path,
    data_disk_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    seed_image: Option<&Path>,
    kernel_cmdline: &str,
    queue: &DispatchQueue,
) -> Result<Retained<VZVirtualMachine>> {
    let configuration = build_real_avf_linux_vm_configuration(
        data_root,
        rootfs_image,
        data_disk_image,
        kernel_path,
        initrd_path,
        seed_image,
        kernel_cmdline,
    )?;
    Ok(unsafe {
        VZVirtualMachine::initWithConfiguration_queue(
            VZVirtualMachine::alloc(),
            &configuration,
            queue,
        )
    })
}

#[cfg(target_os = "macos")]
pub(super) fn exec_on_dispatch_queue<T, F>(queue: &DispatchQueue, label: &str, work: F) -> Result<T>
where
    T: Send + 'static,
    F: Send + FnOnce() -> T + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    queue.exec_async(move || {
        let _ = sender.send(work());
    });
    match receiver.recv_timeout(GUEST_EXEC_CONNECT_TIMEOUT) {
        Ok(value) => Ok(value),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            bail!("{label} timed out waiting for dispatch queue execution")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("{label} dispatch queue disconnected unexpectedly")
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn start_virtual_machine_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    let (sender, receiver) = mpsc::sync_channel(1);
    queue.exec_async(move || {
        let completion = RcBlock::new(move |error: *mut NSError| {
            let result = if error.is_null() {
                Ok(())
            } else {
                let error = unsafe { &*error };
                Err(anyhow::anyhow!(format_nserror(error)))
            };
            let _ = sender.send(result);
        });
        unsafe {
            let virtual_machine = virtual_machine_addr as *const VZVirtualMachine;
            (&*virtual_machine).startWithCompletionHandler(&completion);
        }
    });
    match receiver.recv_timeout(GUEST_EXEC_CONNECT_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err).context("shared AVF Linux VM start"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            bail!("shared AVF Linux VM start timed out waiting for completion")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("shared AVF Linux VM start completion handler disconnected unexpectedly")
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn run_vm_completion_on_queue<F>(
    queue: &DispatchQueue,
    label: &str,
    invoke: F,
) -> Result<()>
where
    F: Send + FnOnce(&RcBlock<dyn Fn(*mut NSError)>) + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    queue.exec_async(move || {
        let completion = RcBlock::new(move |error: *mut NSError| {
            let result = if error.is_null() {
                Ok(())
            } else {
                let error = unsafe { &*error };
                Err(anyhow::anyhow!(format_nserror(error)))
            };
            let _ = sender.send(result);
        });
        invoke(&completion);
    });
    match receiver.recv_timeout(GUEST_EXEC_CONNECT_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err).context(label.to_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            bail!("{label} timed out waiting for completion")
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            bail!("{label} completion handler disconnected unexpectedly")
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn pause_virtual_machine_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    run_vm_completion_on_queue(queue, "shared AVF Linux VM pause", move |completion| {
        let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
        unsafe {
            virtual_machine.pauseWithCompletionHandler(completion);
        }
    })
}

#[cfg(target_os = "macos")]
pub(super) fn resume_virtual_machine_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    run_vm_completion_on_queue(queue, "shared AVF Linux VM resume", move |completion| {
        let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
        unsafe {
            virtual_machine.resumeWithCompletionHandler(completion);
        }
    })
}

#[cfg(target_os = "macos")]
pub(super) fn stop_virtual_machine_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    run_vm_completion_on_queue(queue, "shared AVF Linux VM stop", move |completion| {
        let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
        unsafe {
            virtual_machine.stopWithCompletionHandler(completion);
        }
    })
}

#[cfg(target_os = "macos")]
pub(super) fn virtual_machine_state_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<VZVirtualMachineState> {
    let virtual_machine_addr = virtual_machine as usize;
    exec_on_dispatch_queue(
        queue,
        "shared AVF Linux VM state dispatch",
        move || unsafe {
            let virtual_machine = &*(virtual_machine_addr as *const VZVirtualMachine);
            virtual_machine.state()
        },
    )
}

#[cfg(target_os = "macos")]
pub(super) fn virtual_machine_can_stop_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
) -> Result<bool> {
    let virtual_machine_addr = virtual_machine as usize;
    exec_on_dispatch_queue(
        queue,
        "shared AVF Linux VM canStop dispatch",
        move || unsafe {
            let virtual_machine = &*(virtual_machine_addr as *const VZVirtualMachine);
            virtual_machine.canStop()
        },
    )
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(super) fn save_virtual_machine_state_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
    save_path: &Path,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    let save_path = save_path.to_path_buf();
    run_vm_completion_on_queue(queue, "shared AVF Linux VM save", move |completion| {
        let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
        let save_url = file_url_for_path(&save_path);
        unsafe {
            virtual_machine.saveMachineStateToURL_completionHandler(&save_url, completion);
        }
    })
}

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
pub(super) fn save_virtual_machine_state_on_queue(
    _queue: &DispatchQueue,
    _virtual_machine: *const VZVirtualMachine,
    _save_path: &Path,
) -> Result<()> {
    bail!("AVF Linux VM save/restore requires an Apple silicon macOS host");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub(super) fn restore_virtual_machine_state_on_queue(
    queue: &DispatchQueue,
    virtual_machine: *const VZVirtualMachine,
    save_path: &Path,
) -> Result<()> {
    let virtual_machine_addr = virtual_machine as usize;
    let save_path = save_path.to_path_buf();
    run_vm_completion_on_queue(queue, "shared AVF Linux VM restore", move |completion| {
        let virtual_machine = unsafe { &*(virtual_machine_addr as *const VZVirtualMachine) };
        let save_url = file_url_for_path(&save_path);
        unsafe {
            virtual_machine.restoreMachineStateFromURL_completionHandler(&save_url, completion);
        }
    })
}

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
pub(super) fn restore_virtual_machine_state_on_queue(
    _queue: &DispatchQueue,
    _virtual_machine: *const VZVirtualMachine,
    _save_path: &Path,
) -> Result<()> {
    bail!("AVF Linux VM save/restore requires an Apple silicon macOS host");
}

#[cfg(not(target_os = "macos"))]
pub(super) fn validate_real_avf_linux_vm_configuration(
    _data_root: &Path,
    _rootfs_image: &Path,
    _data_disk_image: &Path,
    _kernel_path: &Path,
    _initrd_path: &Path,
    _kernel_cmdline: &str,
) -> Result<String> {
    bail!("AVF Linux VM validation requires macOS")
}

#[cfg(not(target_os = "macos"))]
pub(super) fn build_real_avf_linux_virtual_machine(
    _data_root: &Path,
    _rootfs_image: &Path,
    _data_disk_image: &Path,
    _kernel_path: &Path,
    _initrd_path: &Path,
    _seed_image: Option<&Path>,
    _kernel_cmdline: &str,
    _queue: &(),
) -> Result<()> {
    bail!("AVF Linux VM launch requires macOS")
}
