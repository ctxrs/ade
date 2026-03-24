mod avf_linux_exec_protocol;

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(all(target_os = "macos", unix))]
use std::os::fd::FromRawFd;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use dispatch2::{DispatchQueue, DispatchQueueAttr};
use flate2::read::GzDecoder;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::{AnyThread, ClassType};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSData, NSError, NSString, NSURL};
#[cfg(target_os = "macos")]
use objc2_virtualization::{
    VZDiskImageCachingMode, VZDiskImageStorageDeviceAttachment, VZDiskImageSynchronizationMode,
    VZFileSerialPortAttachment, VZGenericMachineIdentifier, VZGenericPlatformConfiguration,
    VZLinuxBootLoader, VZMACAddress, VZMemoryBalloonDeviceConfiguration,
    VZNATNetworkDeviceAttachment, VZNetworkDeviceConfiguration, VZSerialPortConfiguration,
    VZSocketDeviceConfiguration, VZStorageDeviceConfiguration, VZVirtioBlockDeviceConfiguration,
    VZVirtioConsoleDeviceSerialPortConfiguration, VZVirtioNetworkDeviceConfiguration,
    VZVirtioSocketConnection, VZVirtioSocketDevice, VZVirtioSocketDeviceConfiguration,
    VZVirtioTraditionalMemoryBalloonDeviceConfiguration, VZVirtualMachine,
    VZVirtualMachineConfiguration, VZVirtualMachineState,
};
use portable_pty::{CommandBuilder as PtyCommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use self::avf_linux_exec_protocol::{
    read_exec_frame, write_exec_frame, AvfLinuxExecError, AvfLinuxExecExit, AvfLinuxExecFrame,
    AvfLinuxExecRequest, AvfLinuxExecResize,
};

const HELPER_PROTOCOL_VERSION: u32 = 1;
const HELPER_PROTOCOL_SCHEMA: &str = "ctx.avf_linux_helper.v1";
const SHARED_VM_STATE_FILE: &str = "shared-vm-state.json";
const SHARED_VM_LOG_FILE: &str = "shared-vm.log";
const SHARED_VM_CONTROL_SOCKET_FILE: &str = "shared-vm-control.sock";
const SHARED_VM_GUEST_AGENT_SOCKET_FILE: &str = "shared-vm-guest-agent.sock";
const SHARED_VM_KERNEL_CMDLINE_FILE: &str = "kernel-cmdline";
const SHARED_VM_SAVED_STATE_FILE: &str = "saved-machine-state.vzvmsave";
const SHARED_VM_SHUTDOWN_REQUEST_FILE: &str = "shutdown-request";
const GUEST_WORKTREES_DIR: &str = "worktrees";
const GUEST_WORKTREE_METADATA_FILE: &str = "worktree.json";
const GUEST_WORKTREE_SHADOW_DIR: &str = "shadow-root";
const GUEST_WORKTREES_ROOT: &str = "/ctx/ws/worktrees";
const GUEST_WORKSPACE_HOMES_ROOT: &str = "/ctx/home";
const GUEST_WORKSPACE_CACHE_ROOT: &str = "/ctx/cache";
const GUEST_WORKSPACE_TMP_ROOT: &str = "/ctx/tmp";
const GUEST_WORKSPACE_USER_PREFIX: &str = "ctx-ws-";
const AVF_LINUX_GUEST_AGENT_HELPER: &str = "guest-agent";
const AVF_LINUX_EGRESS_PROXY_HELPER: &str = "egress-proxy";
const SHARED_VM_CLOUD_INIT_DIR: &str = "cloud-init";
const SHARED_VM_CLOUD_INIT_META_DATA_FILE: &str = "meta-data";
const SHARED_VM_CLOUD_INIT_USER_DATA_FILE: &str = "user-data";
const SHARED_VM_CLOUD_INIT_NETWORK_CONFIG_FILE: &str = "network-config";
const SHARED_VM_CLOUD_INIT_IMAGE_FILE: &str = "cidata.img";
const SHARED_VM_GUEST_AGENT_SERVICE_NAME: &str = "ctx-avf-linux-guest-agent.service";
const SHARED_VM_ROOTFS_LABEL: &str = "ctx-avf-linux";
const SHARED_VM_BOOT_DIR: &str = "boot";
const SHARED_VM_BOOT_KERNEL_FILE: &str = "kernel";
const SHARED_VM_DISK_DIR: &str = "disk";
const SHARED_VM_ROOTFS_FILE: &str = "rootfs.raw";
const SHARED_VM_MACHINE_IDENTIFIER_FILE: &str = "machine-identifier.bin";
const SHARED_VM_MAC_ADDRESS_FILE: &str = "mac-address.txt";
const SHARED_VM_GUEST_CONSOLE_LOG_FILE: &str = "guest-console.log";
#[cfg(target_os = "macos")]
const SHARED_VM_GUEST_CONTROL_VSOCK_PORT: u32 = 47001;
#[cfg(target_os = "macos")]
const SHARED_VM_CONTROL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
const GUEST_EXEC_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const GUEST_EXEC_CONNECT_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
const GUEST_EXEC_TTY_RESIZE_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
const SHARED_VM_SHUTDOWN_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_PTY_COLS: u16 = 80;
const DEFAULT_PTY_ROWS: u16 = 24;
const REQUIRED_SHARED_VM_KERNEL_CMDLINE_TOKENS: &[&str] =
    &["systemd.mask=systemd-networkd-wait-online.service"];

#[derive(Debug, Clone, Serialize)]
struct AvfLinuxHelperProbe {
    protocol_version: u32,
    protocol_schema: &'static str,
    helper_version: &'static str,
    host_os: &'static str,
    host_arch: &'static str,
    supported: bool,
    save_restore_supported: bool,
    rosetta_supported: bool,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AvfLinuxSharedVmLifecycleState {
    Missing,
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AvfLinuxRuntimeLayoutStatus {
    Prepared,
    AlreadyPresent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AvfLinuxSharedVmTransitionStatus {
    Scaffolded,
    Stopped,
    AlreadyStopped,
    Missing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AvfLinuxGuestWorktreeStatus {
    Prepared,
    AlreadyPresent,
}

#[derive(Debug, Clone, Serialize)]
struct AvfLinuxRuntimeLayout {
    protocol_version: u32,
    protocol_schema: &'static str,
    vm_root: PathBuf,
    logs_root: PathBuf,
    state_path: PathBuf,
    layout_status: AvfLinuxRuntimeLayoutStatus,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AvfLinuxSharedVmStateResponse {
    protocol_version: u32,
    protocol_schema: &'static str,
    state: AvfLinuxSharedVmLifecycleState,
    vm_root: PathBuf,
    logs_root: PathBuf,
    state_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    log_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    saved_state_path: Option<PathBuf>,
    saved_state_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_root: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rootfs_image: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kernel_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initrd_path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_saved_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_stopped_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transition_status: Option<AvfLinuxSharedVmTransitionStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    relay_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guest_agent_pid: Option<u32>,
    simulated: bool,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSharedVmState {
    state: AvfLinuxSharedVmLifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rootfs_image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kernel_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    initrd_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_saved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_stopped_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    transition_status: Option<AvfLinuxSharedVmTransitionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    relay_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guest_agent_pid: Option<u32>,
    #[serde(default)]
    simulated: bool,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct AvfLinuxGuestWorktreeResponse {
    protocol_version: u32,
    protocol_schema: &'static str,
    workspace_id: String,
    worktree_id: String,
    guest_root: PathBuf,
    guest_user: String,
    host_shadow_root: PathBuf,
    metadata_path: PathBuf,
    status: AvfLinuxGuestWorktreeStatus,
    simulated: bool,
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedGuestWorktreeState {
    workspace_id: String,
    worktree_id: String,
    host_workspace_root: PathBuf,
    guest_root: PathBuf,
    #[serde(default)]
    guest_user: String,
    host_shadow_root: PathBuf,
    base_commit_sha: String,
    branch_name: String,
    updated_at: String,
    simulated: bool,
    notes: Vec<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("probe") if args.next().is_none() => write_json(&build_probe()),
        Some("prepare-runtime-layout") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            ensure_no_extra_args(args)?;
            write_json(&prepare_runtime_layout(&data_root)?)
        }
        Some("shared-vm-state") | Some("workspace-vm-state") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            ensure_no_extra_args(args)?;
            write_json(&shared_vm_state(&data_root)?)
        }
        Some("start-shared-vm") | Some("start-workspace-vm") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            let runtime_root = required_path_arg(args.next(), "runtime_root")?;
            let rootfs_image = required_path_arg(args.next(), "rootfs_image")?;
            let kernel_path = required_path_arg(args.next(), "kernel_path")?;
            let initrd_path = required_path_arg(args.next(), "initrd_path")?;
            let runtime_version = required_string_arg(args.next(), "runtime_version")?;
            ensure_no_extra_args(args)?;
            write_json(&start_shared_vm(
                &data_root,
                &runtime_root,
                &rootfs_image,
                &kernel_path,
                &initrd_path,
                runtime_version,
            )?)
        }
        Some("stop-shared-vm") | Some("stop-workspace-vm") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            ensure_no_extra_args(args)?;
            write_json(&stop_shared_vm(&data_root)?)
        }
        Some("prepare-guest-worktree") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            let workspace_id = required_string_arg(args.next(), "workspace_id")?;
            let worktree_id = required_string_arg(args.next(), "worktree_id")?;
            let host_workspace_root = required_path_arg(args.next(), "host_workspace_root")?;
            let base_commit_sha = required_string_arg(args.next(), "base_commit_sha")?;
            let branch_name = required_string_arg(args.next(), "branch_name")?;
            ensure_no_extra_args(args)?;
            write_json(&prepare_guest_worktree(
                &data_root,
                &workspace_id,
                &worktree_id,
                &host_workspace_root,
                &base_commit_sha,
                &branch_name,
            )?)
        }
        Some("serve-shared-vm") | Some("serve-workspace-vm") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            ensure_no_extra_args(args)?;
            serve_shared_vm(&data_root)
        }
        Some("serve-guest-agent") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            ensure_no_extra_args(args)?;
            serve_guest_agent(&data_root)
        }
        Some("run-shared-vm") | Some("run-workspace-vm") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            ensure_no_extra_args(args)?;
            run_shared_vm(&data_root)
        }
        Some("guest-exec") => {
            let mut data_root = None;
            let mut workspace_id = None;
            let mut worktree_id = None;
            let mut cwd = None;
            let mut command = None;
            let mut guest_env = Vec::new();
            let mut user = None;
            let mut pty = false;
            let mut passthrough_args = Vec::new();
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--data-root" => data_root = args.next().map(PathBuf::from),
                    "--workspace-id" => workspace_id = args.next(),
                    "--worktree-id" => worktree_id = args.next(),
                    "--cwd" => cwd = args.next().map(PathBuf::from),
                    "--command" => command = args.next(),
                    "--env" => guest_env.push(required_string_arg(args.next(), "env")?),
                    "--user" => user = args.next(),
                    "--pty" => pty = true,
                    "--" => {
                        passthrough_args.extend(args);
                        break;
                    }
                    other => bail!("unknown guest-exec argument `{other}`"),
                }
            }
            let exit_code = guest_exec(
                &required_path_arg(
                    data_root.map(|path| path.to_string_lossy().to_string()),
                    "data_root",
                )?,
                &required_string_arg(workspace_id, "workspace_id")?,
                &required_string_arg(worktree_id, "worktree_id")?,
                &required_path_arg(cwd.map(|path| path.to_string_lossy().to_string()), "cwd")?,
                &required_string_arg(command, "command")?,
                &guest_env,
                user.as_deref(),
                pty,
                &passthrough_args,
            )?;
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            Ok(())
        }
        Some(other) => bail!("unsupported ctx-avf-linux-helper command: {other}"),
        None => bail!(
            "usage: ctx-avf-linux-helper <probe|prepare-runtime-layout|workspace-vm-state|start-workspace-vm|stop-workspace-vm|prepare-guest-worktree|serve-workspace-vm|serve-guest-agent|run-workspace-vm|guest-exec> ..."
        ),
    }
}

fn build_probe() -> AvfLinuxHelperProbe {
    let mut notes = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(version) = macos_product_version() {
            notes.push(format!("host macOS {version}"));
        }
        notes.push(
            "shared AVF Linux VM backend is enabled for managed guest artifact prefetch"
                .to_string(),
        );
    } else {
        notes.push("AVF Linux helper is only usable on macOS hosts".to_string());
    }
    if cfg!(target_arch = "aarch64") {
        notes.push(
            "Apple silicon host detected; save/restore and Rosetta-backed Linux guests are available when the runtime and VM configuration support them".to_string(),
        );
    } else {
        notes.push(
            "Intel Mac host detected; save/restore is expected to remain unavailable".to_string(),
        );
    }

    AvfLinuxHelperProbe {
        protocol_version: HELPER_PROTOCOL_VERSION,
        protocol_schema: HELPER_PROTOCOL_SCHEMA,
        helper_version: env!("CARGO_PKG_VERSION"),
        host_os: std::env::consts::OS,
        host_arch: std::env::consts::ARCH,
        supported: cfg!(target_os = "macos"),
        save_restore_supported: shared_vm_save_restore_supported(),
        rosetta_supported: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        notes,
    }
}

fn shared_vm_save_restore_supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64")) && macos_major_version_at_least(14)
}

fn macos_major_version_at_least(required_major: u64) -> bool {
    let Some(version) = macos_product_version() else {
        return false;
    };
    let Some(major) = version
        .split('.')
        .next()
        .and_then(|segment| segment.parse::<u64>().ok())
    else {
        return false;
    };
    major >= required_major
}

#[cfg(target_os = "macos")]
fn file_url_for_path(path: &Path) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(path.to_string_lossy().as_ref()))
}

#[cfg(target_os = "macos")]
fn format_nserror(error: &NSError) -> String {
    format!(
        "{} (domain: {}, code: {})",
        error.localizedDescription(),
        error.domain(),
        error.code()
    )
}

fn default_shared_vm_kernel_cmdline() -> String {
    ensure_required_shared_vm_kernel_cmdline_tokens(format!(
        "console=hvc0 root=LABEL={SHARED_VM_ROOTFS_LABEL} rootwait rw"
    ))
}

fn ensure_required_shared_vm_kernel_cmdline_tokens(mut cmdline: String) -> String {
    for token in REQUIRED_SHARED_VM_KERNEL_CMDLINE_TOKENS {
        if cmdline.split_whitespace().any(|part| part == *token) {
            continue;
        }
        if !cmdline.is_empty() {
            cmdline.push(' ');
        }
        cmdline.push_str(token);
    }
    cmdline
}

fn shared_vm_boot_root(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_BOOT_DIR)
}

fn shared_vm_boot_kernel_path(data_root: &Path) -> PathBuf {
    shared_vm_boot_root(data_root).join(SHARED_VM_BOOT_KERNEL_FILE)
}

fn shared_vm_disk_root(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_DISK_DIR)
}

fn shared_vm_rootfs_path(data_root: &Path) -> PathBuf {
    shared_vm_disk_root(data_root).join(SHARED_VM_ROOTFS_FILE)
}

fn path_has_gzip_magic(path: &Path) -> Result<bool> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut magic = [0_u8; 2];
    let read = file
        .read(&mut magic)
        .with_context(|| format!("reading {}", path.display()))?;
    Ok(read == 2 && magic == [0x1f, 0x8b])
}

fn materialize_bootable_kernel_image(
    data_root: &Path,
    kernel_path: &Path,
) -> Result<(PathBuf, Option<String>)> {
    if !path_has_gzip_magic(kernel_path)? {
        return Ok((kernel_path.to_path_buf(), None));
    }

    let boot_root = shared_vm_boot_root(data_root);
    fs::create_dir_all(&boot_root).with_context(|| format!("creating {}", boot_root.display()))?;
    let staged_kernel_path = shared_vm_boot_kernel_path(data_root);
    let staged_kernel_tmp_path = staged_kernel_path.with_extension("tmp");

    let source_file =
        File::open(kernel_path).with_context(|| format!("opening {}", kernel_path.display()))?;
    let mut decoder = GzDecoder::new(source_file);
    let mut staged_file = File::create(&staged_kernel_tmp_path)
        .with_context(|| format!("creating {}", staged_kernel_tmp_path.display()))?;
    std::io::copy(&mut decoder, &mut staged_file).with_context(|| {
        format!(
            "decompressing AVF Linux kernel {} into {}",
            kernel_path.display(),
            staged_kernel_tmp_path.display()
        )
    })?;
    staged_file
        .flush()
        .with_context(|| format!("flushing {}", staged_kernel_tmp_path.display()))?;
    drop(staged_file);
    fs::rename(&staged_kernel_tmp_path, &staged_kernel_path).with_context(|| {
        format!(
            "moving {} into {}",
            staged_kernel_tmp_path.display(),
            staged_kernel_path.display()
        )
    })?;

    Ok((
        staged_kernel_path,
        Some(format!(
            "staged a decompressed AVF boot kernel from {} because the bundled kernel artifact is gzip-compressed",
            kernel_path.display()
        )),
    ))
}

fn clone_or_copy_rootfs_image(source_rootfs: &Path, staged_rootfs_tmp: &Path) -> Result<String> {
    let clone_attempt = Command::new("cp")
        .arg("-c")
        .arg(source_rootfs)
        .arg(staged_rootfs_tmp)
        .status();
    if let Ok(status) = clone_attempt {
        if status.success() {
            return Ok(format!(
                "staged a writable shared-VM rootfs clone from {} into {}",
                source_rootfs.display(),
                staged_rootfs_tmp.display()
            ));
        }
    }

    fs::copy(source_rootfs, staged_rootfs_tmp).with_context(|| {
        format!(
            "copying writable shared-VM rootfs from {} into {}",
            source_rootfs.display(),
            staged_rootfs_tmp.display()
        )
    })?;
    Ok(format!(
        "staged a writable shared-VM rootfs copy from {} into {}",
        source_rootfs.display(),
        staged_rootfs_tmp.display()
    ))
}

fn materialize_writable_rootfs_image(
    data_root: &Path,
    source_rootfs: &Path,
) -> Result<(PathBuf, Option<String>)> {
    let staged_rootfs = shared_vm_rootfs_path(data_root);
    if staged_rootfs == source_rootfs {
        return Ok((staged_rootfs, None));
    }
    if staged_rootfs.exists() {
        return Ok((
            staged_rootfs.clone(),
            Some(format!(
                "reusing writable shared-VM rootfs at {}",
                staged_rootfs.display()
            )),
        ));
    }

    let disk_root = shared_vm_disk_root(data_root);
    fs::create_dir_all(&disk_root).with_context(|| format!("creating {}", disk_root.display()))?;
    let staged_rootfs_tmp = staged_rootfs.with_extension("tmp");
    fs::remove_file(&staged_rootfs_tmp).ok();
    let note = clone_or_copy_rootfs_image(source_rootfs, &staged_rootfs_tmp)?;
    fs::rename(&staged_rootfs_tmp, &staged_rootfs).with_context(|| {
        format!(
            "moving {} into {}",
            staged_rootfs_tmp.display(),
            staged_rootfs.display()
        )
    })?;
    Ok((staged_rootfs, Some(note)))
}

fn load_shared_vm_kernel_cmdline(runtime_root: &Path) -> Result<String> {
    let path = shared_vm_kernel_cmdline_path(runtime_root);
    if !path.is_file() {
        return Ok(default_shared_vm_kernel_cmdline());
    }
    let contents =
        fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Ok(default_shared_vm_kernel_cmdline());
    }
    Ok(ensure_required_shared_vm_kernel_cmdline_tokens(
        trimmed.to_string(),
    ))
}

#[cfg(target_os = "macos")]
fn validate_real_avf_linux_vm_configuration(
    rootfs_image: &Path,
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

    let boot_loader =
        unsafe { VZLinuxBootLoader::initWithKernelURL(VZLinuxBootLoader::alloc(), &kernel_url) };
    unsafe {
        boot_loader.setCommandLine(&NSString::from_str(kernel_cmdline));
        boot_loader.setInitialRamdiskURL(Some(&initrd_url));
    }

    let storage_attachment = unsafe {
        VZDiskImageStorageDeviceAttachment::initWithURL_readOnly_cachingMode_synchronizationMode_error(
            VZDiskImageStorageDeviceAttachment::alloc(),
            &rootfs_url,
            false,
            VZDiskImageCachingMode::Automatic,
            VZDiskImageSynchronizationMode::Fsync,
        )
    }
    .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    let storage_device = unsafe {
        VZVirtioBlockDeviceConfiguration::initWithAttachment(
            VZVirtioBlockDeviceConfiguration::alloc(),
            storage_attachment.as_super(),
        )
    };
    let storage_devices: Retained<NSArray<VZStorageDeviceConfiguration>> =
        NSArray::from_slice(&[storage_device.as_super()]);

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

    let configuration = unsafe { VZVirtualMachineConfiguration::new() };
    let platform = unsafe { VZGenericPlatformConfiguration::new() };
    let min_cpu = unsafe { VZVirtualMachineConfiguration::minimumAllowedCPUCount() };
    let max_cpu = unsafe { VZVirtualMachineConfiguration::maximumAllowedCPUCount() };
    let cpu_count = min_cpu.max(2).min(max_cpu);
    let min_memory = unsafe { VZVirtualMachineConfiguration::minimumAllowedMemorySize() };
    let max_memory = unsafe { VZVirtualMachineConfiguration::maximumAllowedMemorySize() };
    let target_memory = (4 * 1024 * 1024 * 1024_u64).max(min_memory).min(max_memory);
    unsafe {
        configuration.setBootLoader(Some(boot_loader.as_super()));
        configuration.setPlatform(platform.as_super());
        configuration.setCPUCount(cpu_count);
        configuration.setMemorySize(target_memory);
        configuration.setStorageDevices(&storage_devices);
        configuration.setNetworkDevices(&network_devices);
        configuration.setSocketDevices(&socket_devices);
        configuration.setMemoryBalloonDevices(&balloon_devices);
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
        "native AVF configuration validated (cpu={}, memory={} MiB; {})",
        cpu_count,
        target_memory / (1024 * 1024),
        save_restore_note,
    ))
}

#[cfg(target_os = "macos")]
fn load_or_create_shared_vm_machine_identifier(
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
fn load_or_create_shared_vm_mac_address(data_root: &Path) -> Result<Retained<VZMACAddress>> {
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
fn build_real_avf_linux_vm_configuration(
    data_root: &Path,
    rootfs_image: &Path,
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
    let mut storage_devices_owned = vec![root_storage_device];
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
    let cpu_count = min_cpu.max(2).min(max_cpu);
    let min_memory = unsafe { VZVirtualMachineConfiguration::minimumAllowedMemorySize() };
    let max_memory = unsafe { VZVirtualMachineConfiguration::maximumAllowedMemorySize() };
    let target_memory = (4 * 1024 * 1024 * 1024_u64).max(min_memory).min(max_memory);
    unsafe {
        configuration.setBootLoader(Some(boot_loader.as_super()));
        platform.setMachineIdentifier(&machine_identifier);
        configuration.setPlatform(platform.as_super());
        configuration.setCPUCount(cpu_count);
        configuration.setMemorySize(target_memory);
        configuration.setStorageDevices(&storage_devices);
        configuration.setNetworkDevices(&network_devices);
        configuration.setSerialPorts(&serial_ports);
        configuration.setSocketDevices(&socket_devices);
        configuration.setMemoryBalloonDevices(&balloon_devices);
        configuration
            .validateWithError()
            .map_err(|err| anyhow::anyhow!(format_nserror(&err)))?;
    }
    Ok(configuration)
}

#[cfg(target_os = "macos")]
fn build_real_avf_linux_virtual_machine(
    data_root: &Path,
    rootfs_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    seed_image: Option<&Path>,
    kernel_cmdline: &str,
    queue: &DispatchQueue,
) -> Result<Retained<VZVirtualMachine>> {
    let configuration = build_real_avf_linux_vm_configuration(
        data_root,
        rootfs_image,
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
fn exec_on_dispatch_queue<T, F>(queue: &DispatchQueue, label: &str, work: F) -> Result<T>
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
fn start_virtual_machine_on_queue(
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
fn run_vm_completion_on_queue<F>(queue: &DispatchQueue, label: &str, invoke: F) -> Result<()>
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
fn pause_virtual_machine_on_queue(
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
fn resume_virtual_machine_on_queue(
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
fn stop_virtual_machine_on_queue(
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
fn virtual_machine_state_on_queue(
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
fn virtual_machine_can_stop_on_queue(
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
fn save_virtual_machine_state_on_queue(
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
fn save_virtual_machine_state_on_queue(
    _queue: &DispatchQueue,
    _virtual_machine: *const VZVirtualMachine,
    _save_path: &Path,
) -> Result<()> {
    bail!("AVF Linux VM save/restore requires an Apple silicon macOS host");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn restore_virtual_machine_state_on_queue(
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
fn restore_virtual_machine_state_on_queue(
    _queue: &DispatchQueue,
    _virtual_machine: *const VZVirtualMachine,
    _save_path: &Path,
) -> Result<()> {
    bail!("AVF Linux VM save/restore requires an Apple silicon macOS host");
}

#[cfg(not(target_os = "macos"))]
fn validate_real_avf_linux_vm_configuration(
    _rootfs_image: &Path,
    _kernel_path: &Path,
    _initrd_path: &Path,
    _kernel_cmdline: &str,
) -> Result<String> {
    bail!("AVF Linux VM validation requires macOS")
}

#[cfg(not(target_os = "macos"))]
fn build_real_avf_linux_virtual_machine(
    _rootfs_image: &Path,
    _kernel_path: &Path,
    _initrd_path: &Path,
    _seed_image: Option<&Path>,
    _kernel_cmdline: &str,
    _queue: &(),
) -> Result<()> {
    bail!("AVF Linux VM launch requires macOS")
}

fn prepare_runtime_layout(data_root: &Path) -> Result<AvfLinuxRuntimeLayout> {
    let vm_root = shared_vm_root(data_root);
    let logs_root = shared_vm_logs_root(data_root);
    let state_path = shared_vm_state_path(data_root);
    let existed = vm_root.exists() && logs_root.exists() && state_path.exists();

    fs::create_dir_all(&vm_root).with_context(|| format!("creating {}", vm_root.display()))?;
    fs::create_dir_all(&logs_root).with_context(|| format!("creating {}", logs_root.display()))?;

    if !state_path.exists() {
        let state = PersistedSharedVmState {
            state: AvfLinuxSharedVmLifecycleState::Stopped,
            runtime_root: None,
            rootfs_image: None,
            kernel_path: None,
            initrd_path: None,
            runtime_version: None,
            updated_at: Some(now_timestamp_string()),
            last_started_at: None,
            last_saved_at: None,
            last_stopped_at: None,
            transition_status: None,
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

fn shared_vm_state(data_root: &Path) -> Result<AvfLinuxSharedVmStateResponse> {
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
            state.state = AvfLinuxSharedVmLifecycleState::Stopped;
            state.updated_at = Some(now_timestamp_string());
            state.last_stopped_at = state.updated_at.clone();
            state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Stopped);
            state.notes = vec![if state.simulated {
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
        vm_root,
        logs_root,
        state_path,
        log_path,
    ))
}

fn shared_vm_guest_agent_helper_path(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("helpers")
        .join(AVF_LINUX_GUEST_AGENT_HELPER)
}

fn shared_vm_egress_proxy_helper_path(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("helpers")
        .join(AVF_LINUX_EGRESS_PROXY_HELPER)
}

fn shared_vm_runtime_supports_real_guest_exec(runtime_root: &Path) -> (bool, String) {
    let guest_agent_path = shared_vm_guest_agent_helper_path(runtime_root);
    if guest_agent_path.is_file() {
        return (
            true,
            format!(
                "runtime includes a guest-agent payload at {}; enabling real AVF VM ownership",
                guest_agent_path.display()
            ),
        );
    }
    if std::env::var_os("CTX_AVF_LINUX_FORCE_REAL_VM").is_some() {
        return (
            true,
            format!(
                "forcing real AVF VM ownership without a staged guest-agent payload because CTX_AVF_LINUX_FORCE_REAL_VM is set (expected helper path: {})",
                guest_agent_path.display()
            ),
        );
    }
    (
        false,
        format!(
            "runtime is missing a baked guest-agent payload at {}; keeping the shared VM on the simulated relay path",
            guest_agent_path.display()
        ),
    )
}

fn start_shared_vm(
    data_root: &Path,
    runtime_root: &Path,
    rootfs_image: &Path,
    kernel_path: &Path,
    initrd_path: &Path,
    runtime_version: String,
) -> Result<AvfLinuxSharedVmStateResponse> {
    for path in [runtime_root, rootfs_image, kernel_path, initrd_path] {
        if !path.exists() {
            bail!(
                "required AVF Linux runtime path is missing: {}",
                path.display()
            );
        }
    }
    let kernel_cmdline = load_shared_vm_kernel_cmdline(runtime_root)?;
    let (boot_kernel_path, kernel_materialization_note) =
        materialize_bootable_kernel_image(data_root, kernel_path)?;
    let (staged_rootfs_image, rootfs_materialization_note) =
        materialize_writable_rootfs_image(data_root, rootfs_image)?;

    let native_validation_note = if cfg!(test) {
        None
    } else {
        Some(
            validate_real_avf_linux_vm_configuration(
                &staged_rootfs_image,
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

    let _ = prepare_runtime_layout(data_root)?;
    clear_shared_vm_shutdown_request(data_root);
    let state_path = shared_vm_state_path(data_root);
    let mut state = load_state(&state_path)?.unwrap_or_else(default_stopped_state);
    let saved_state_path = shared_vm_saved_state_path(data_root);
    let runtime_shape_changed = state.runtime_version.as_deref() != Some(runtime_version.as_str())
        || state.rootfs_image.as_ref() != Some(&staged_rootfs_image)
        || state.kernel_path.as_ref() != Some(&boot_kernel_path)
        || state.initrd_path.as_ref().map(PathBuf::as_path) != Some(initrd_path);
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
    let owner_alive = state.relay_pid.is_some_and(shared_vm_server_process_alive);
    let guest_alive = state
        .guest_agent_pid
        .is_some_and(shared_vm_server_process_alive);
    let already_running = if state.simulated {
        owner_alive && guest_alive
    } else {
        owner_alive
    };
    if already_running {
        state.state = AvfLinuxSharedVmLifecycleState::Running;
        state.runtime_root = Some(runtime_root.to_path_buf());
        state.rootfs_image = Some(staged_rootfs_image.clone());
        state.kernel_path = Some(boot_kernel_path.clone());
        state.initrd_path = Some(initrd_path.to_path_buf());
        state.runtime_version = Some(runtime_version);
        state.updated_at = Some(now_timestamp_string());
        state.last_started_at = state.updated_at.clone();
        if stale_saved_state_note.is_some() {
            state.last_saved_at = None;
        }
        state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Scaffolded);
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
        state.notes.push(real_vm_support_note.clone());
        if let Some(note) = native_validation_note.clone() {
            state.notes.push(note);
        }
        if let Some(note) = stale_saved_state_note.clone() {
            state.notes.push(note);
        }
        persist_state(&state_path, &state)?;
        return shared_vm_state(data_root);
    }
    if let Some(pid) = state.relay_pid.take() {
        stop_shared_vm_server(pid);
    }
    if let Some(pid) = state.guest_agent_pid.take() {
        stop_shared_vm_server(pid);
    }
    state.runtime_root = Some(runtime_root.to_path_buf());
    state.rootfs_image = Some(staged_rootfs_image.clone());
    state.kernel_path = Some(boot_kernel_path.clone());
    state.initrd_path = Some(initrd_path.to_path_buf());
    state.runtime_version = Some(runtime_version.clone());
    state.updated_at = Some(now_timestamp_string());
    state.last_started_at = None;
    if stale_saved_state_note.is_some() {
        state.last_saved_at = None;
    }
    state.transition_status = None;
    state.relay_pid = None;
    state.guest_agent_pid = None;
    state.notes =
        vec!["persisting AVF runtime paths before starting the shared VM owner".to_string()];
    persist_state(&state_path, &state)?;

    let (relay_pid, guest_agent_pid, simulated, mut notes) = if cfg!(test) {
        (
            None,
            None,
            true,
            vec!["shared VM start was requested in test mode; state is simulated until actual AVF guest boot is implemented".to_string()],
        )
    } else if real_vm_supported {
        let relay_pid = spawn_real_shared_vm_owner(data_root)?;
        (
            Some(relay_pid),
            None,
            false,
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
    state.updated_at = Some(now_timestamp_string());
    state.last_started_at = state.updated_at.clone();
    state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Scaffolded);
    state.relay_pid = relay_pid;
    state.guest_agent_pid = guest_agent_pid;
    state.simulated = simulated;
    if let Some(note) = kernel_materialization_note {
        notes.push(note);
    }
    if let Some(note) = rootfs_materialization_note {
        notes.push(note);
    }
    if let Some(note) = native_validation_note {
        notes.push(note);
    }
    if let Some(note) = stale_saved_state_note {
        notes.push(note);
    }
    state.notes = notes;
    persist_state(&state_path, &state)?;
    shared_vm_state(data_root)
}

fn stop_shared_vm(data_root: &Path) -> Result<AvfLinuxSharedVmStateResponse> {
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
            updated_at: Some(now_timestamp_string()),
            last_started_at: None,
            last_saved_at: None,
            last_stopped_at: None,
            transition_status: Some(AvfLinuxSharedVmTransitionStatus::Missing),
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
                let control_socket = shared_vm_control_socket_path(data_root);
                if control_socket.exists() {
                    let _ = fs::remove_file(&control_socket);
                }
                let guest_agent_socket = shared_vm_guest_agent_socket_path(data_root);
                if guest_agent_socket.exists() {
                    let _ = fs::remove_file(&guest_agent_socket);
                }
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
    let control_socket = shared_vm_control_socket_path(data_root);
    if control_socket.exists() {
        let _ = fs::remove_file(&control_socket);
    }
    let guest_agent_socket = shared_vm_guest_agent_socket_path(data_root);
    if guest_agent_socket.exists() {
        let _ = fs::remove_file(&guest_agent_socket);
    }
    state.state = AvfLinuxSharedVmLifecycleState::Stopped;
    state.updated_at = Some(now_timestamp_string());
    state.last_stopped_at = state.updated_at.clone();
    state.transition_status = Some(AvfLinuxSharedVmTransitionStatus::Stopped);
    state.notes = vec![if state.simulated {
        "shared VM lifecycle scaffold is stopped".to_string()
    } else {
        "real shared AVF Linux VM is stopped".to_string()
    }];
    persist_state(&state_path, &state)?;
    shared_vm_state(data_root)
}

fn serve_shared_vm(data_root: &Path) -> Result<()> {
    let listener = bind_shared_vm_control_listener(data_root)?;
    loop {
        let (stream, _) = listener.accept().with_context(|| {
            format!(
                "accepting {}",
                shared_vm_control_socket_path(data_root).display()
            )
        })?;
        let data_root = data_root.to_path_buf();
        std::thread::spawn(move || {
            if let Err(err) = handle_shared_vm_control_connection(&data_root, stream) {
                let _ = append_shared_vm_log_line(
                    &data_root,
                    &format!("shared VM control connection failed: {err:#}"),
                );
            }
        });
    }
}

fn serve_guest_agent(data_root: &Path) -> Result<()> {
    let listener = bind_guest_agent_control_listener(data_root)?;
    loop {
        let (stream, _) = listener.accept().with_context(|| {
            format!(
                "accepting {}",
                shared_vm_guest_agent_socket_path(data_root).display()
            )
        })?;
        let data_root = data_root.to_path_buf();
        std::thread::spawn(move || {
            if let Err(err) = handle_guest_agent_control_connection(&data_root, stream) {
                let _ = append_shared_vm_log_line(
                    &data_root,
                    &format!("guest-agent control connection failed: {err:#}"),
                );
            }
        });
    }
}

#[cfg(unix)]
fn bind_shared_vm_control_listener(data_root: &Path) -> Result<UnixListener> {
    let socket_path = shared_vm_control_socket_path(data_root);
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if socket_path.exists() {
        fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding {}", socket_path.display()))?;
    Ok(listener)
}

#[cfg(unix)]
fn bind_guest_agent_control_listener(data_root: &Path) -> Result<UnixListener> {
    let socket_path = shared_vm_guest_agent_socket_path(data_root);
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    if socket_path.exists() {
        fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding {}", socket_path.display()))?;
    Ok(listener)
}

#[cfg(not(unix))]
fn bind_shared_vm_control_listener(_data_root: &Path) -> Result<()> {
    bail!("shared VM control listener requires unix domain sockets")
}

#[cfg(not(unix))]
fn bind_guest_agent_control_listener(_data_root: &Path) -> Result<()> {
    bail!("guest-agent control listener requires unix domain sockets")
}

#[cfg(unix)]
fn handle_shared_vm_control_connection(data_root: &Path, mut stream: UnixStream) -> Result<()> {
    let request = match read_exec_frame(&mut stream).context("reading shared VM exec request")? {
        Some(AvfLinuxExecFrame::Request(request)) => request,
        Some(other) => {
            write_exec_frame(
                &mut stream,
                &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                    code: "invalid_request".to_string(),
                    message: format!("expected request frame, received {other:?}"),
                }),
            )
            .ok();
            bail!("expected request frame, received {other:?}");
        }
        None => bail!("shared VM control socket closed before request frame"),
    };

    let resolved = resolve_simulated_exec_request(data_root, &request)?;
    let forwarded_request = AvfLinuxExecRequest::new(
        resolved.command,
        resolved.args,
        resolved.cwd.display().to_string(),
        request.user,
        resolved.env,
        request.pty,
    );
    proxy_request_to_guest_agent(data_root, stream, forwarded_request)
}

#[cfg(not(unix))]
fn handle_shared_vm_control_connection(_data_root: &Path, _stream: ()) -> Result<()> {
    bail!("shared VM control connections require unix domain sockets")
}

#[cfg(unix)]
fn handle_guest_agent_control_connection(_data_root: &Path, mut stream: UnixStream) -> Result<()> {
    let request = match read_exec_frame(&mut stream).context("reading guest-agent exec request")? {
        Some(AvfLinuxExecFrame::Request(request)) => request,
        Some(other) => {
            write_exec_frame(
                &mut stream,
                &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                    code: "invalid_request".to_string(),
                    message: format!("expected request frame, received {other:?}"),
                }),
            )
            .ok();
            bail!("expected request frame, received {other:?}");
        }
        None => bail!("guest-agent control socket closed before request frame"),
    };
    run_guest_agent_exec_request(stream, request)
}

#[cfg(not(unix))]
fn handle_guest_agent_control_connection(_data_root: &Path, _stream: ()) -> Result<()> {
    bail!("guest-agent control connections require unix domain sockets")
}

#[cfg(unix)]
fn proxy_request_to_guest_agent(
    data_root: &Path,
    client_stream: UnixStream,
    request: AvfLinuxExecRequest,
) -> Result<()> {
    let mut agent_stream =
        connect_guest_agent_control_socket(&shared_vm_guest_agent_socket_path(data_root))?;
    write_exec_frame(&mut agent_stream, &AvfLinuxExecFrame::Request(request))
        .context("writing guest-agent exec request")?;

    let mut client_reader = client_stream;
    let mut agent_reader = agent_stream;
    let client_writer = Arc::new(Mutex::new(
        client_reader
            .try_clone()
            .context("cloning shared VM client stream")?,
    ));
    let agent_writer = Arc::new(Mutex::new(
        agent_reader
            .try_clone()
            .context("cloning guest-agent control stream")?,
    ));

    let _stdin_forwarder = {
        let agent_writer = Arc::clone(&agent_writer);
        std::thread::spawn(move || loop {
            match read_exec_frame(&mut client_reader) {
                Ok(Some(
                    frame @ (AvfLinuxExecFrame::Stdin(_)
                    | AvfLinuxExecFrame::CloseStdin
                    | AvfLinuxExecFrame::Resize(_)),
                )) => {
                    let Ok(mut guard) = agent_writer.lock() else {
                        return;
                    };
                    if write_exec_frame(&mut *guard, &frame).is_err() {
                        return;
                    }
                }
                Ok(Some(_)) | Ok(None) => {
                    let Ok(mut guard) = agent_writer.lock() else {
                        return;
                    };
                    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                    return;
                }
                Err(_) => {
                    let Ok(mut guard) = agent_writer.lock() else {
                        return;
                    };
                    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                    return;
                }
            }
        })
    };

    loop {
        match read_exec_frame(&mut agent_reader).context("reading guest-agent response frame")? {
            Some(frame) => {
                let terminal = matches!(
                    frame,
                    AvfLinuxExecFrame::Exit(_) | AvfLinuxExecFrame::Error(_)
                );
                let mut guard = client_writer
                    .lock()
                    .map_err(|_| anyhow::anyhow!("shared VM client writer mutex poisoned"))?;
                write_exec_frame(&mut *guard, &frame)
                    .context("writing proxied guest-agent frame")?;
                drop(guard);
                if terminal {
                    return Ok(());
                }
            }
            None => {
                bail!("guest-agent control socket closed before sending an exit frame");
            }
        }
    }
}

#[cfg(unix)]
fn run_guest_agent_exec_request(
    mut stream: UnixStream,
    request: AvfLinuxExecRequest,
) -> Result<()> {
    if request.command.trim().is_empty() {
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                code: "invalid_request".to_string(),
                message: "guest exec command must not be empty".to_string(),
            }),
        )
        .ok();
        bail!("guest exec command must not be empty");
    }
    if request.cwd.trim().is_empty() {
        write_exec_frame(
            &mut stream,
            &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                code: "invalid_request".to_string(),
                message: "guest exec cwd must not be empty".to_string(),
            }),
        )
        .ok();
        bail!("guest exec cwd must not be empty");
    }

    let resolved = SimulatedExecRequest {
        command: request.command,
        args: request.args,
        cwd: PathBuf::from(request.cwd),
        env: request.env,
    };
    if request.pty {
        return handle_shared_vm_pty_connection(stream, resolved);
    }

    let mut child = Command::new(&resolved.command);
    child.args(&resolved.args);
    child.current_dir(&resolved.cwd);
    child.stdin(Stdio::piped());
    child.stdout(Stdio::piped());
    child.stderr(Stdio::piped());
    for (key, value) in &resolved.env {
        child.env(key, value);
    }

    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(err) => {
            write_exec_frame(
                &mut stream,
                &AvfLinuxExecFrame::Error(AvfLinuxExecError {
                    code: "spawn_failed".to_string(),
                    message: err.to_string(),
                }),
            )
            .ok();
            return Err(err).with_context(|| {
                format!(
                    "spawning guest-agent command `{}` in {}",
                    resolved.command,
                    resolved.cwd.display()
                )
            });
        }
    };

    let writer = Arc::new(Mutex::new(
        stream.try_clone().context("cloning guest-agent stream")?,
    ));
    let mut stdin_reader = stream;
    let stdin_thread = if let Some(mut child_stdin) = child.stdin.take() {
        Some(std::thread::spawn(move || loop {
            match read_exec_frame(&mut stdin_reader) {
                Ok(Some(AvfLinuxExecFrame::Stdin(bytes))) => {
                    if child_stdin.write_all(&bytes).is_err() {
                        return;
                    }
                    let _ = child_stdin.flush();
                }
                Ok(Some(AvfLinuxExecFrame::CloseStdin)) | Ok(None) => return,
                Ok(Some(_)) => return,
                Err(_) => return,
            }
        }))
    } else {
        None
    };

    let stdout_thread = child.stdout.take().map(|mut stdout| {
        let writer = Arc::clone(&writer);
        std::thread::spawn(move || {
            relay_child_output(&mut stdout, writer, true);
        })
    });
    let stderr_thread = child.stderr.take().map(|mut stderr| {
        let writer = Arc::clone(&writer);
        std::thread::spawn(move || {
            relay_child_output(&mut stderr, writer, false);
        })
    });

    let status = child.wait().context("waiting for guest-agent child")?;
    if let Some(handle) = stdin_thread {
        let _ = handle.join();
    }
    if let Some(handle) = stdout_thread {
        let _ = handle.join();
    }
    if let Some(handle) = stderr_thread {
        let _ = handle.join();
    }
    let exit_code = status.code().unwrap_or(1);
    write_exec_frame(
        &mut *writer
            .lock()
            .map_err(|_| anyhow::anyhow!("guest-agent writer mutex poisoned"))?,
        &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code }),
    )
    .context("writing guest-agent exit frame")?;
    Ok(())
}

fn relay_child_output(reader: &mut impl Read, writer: Arc<Mutex<UnixStream>>, stdout: bool) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let frame = if stdout {
                    AvfLinuxExecFrame::Stdout(buf[..n].to_vec())
                } else {
                    AvfLinuxExecFrame::Stderr(buf[..n].to_vec())
                };
                let Ok(mut guard) = writer.lock() else {
                    return;
                };
                if write_exec_frame(&mut *guard, &frame).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

fn relay_pty_output(reader: &mut impl Read, writer: Arc<Mutex<UnixStream>>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let Ok(mut guard) = writer.lock() else {
                    return;
                };
                if write_exec_frame(&mut *guard, &AvfLinuxExecFrame::Stdout(buf[..n].to_vec()))
                    .is_err()
                {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

fn handle_shared_vm_pty_connection(
    stream: UnixStream,
    resolved: SimulatedExecRequest,
) -> Result<()> {
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: DEFAULT_PTY_ROWS,
            cols: DEFAULT_PTY_COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("opening simulated shared VM PTY")?;
    let mut cmd = PtyCommandBuilder::new(resolved.command.clone());
    for arg in &resolved.args {
        cmd.arg(arg);
    }
    cmd.cwd(&resolved.cwd);
    for (key, value) in &resolved.env {
        cmd.env(key, value);
    }

    let child = pair.slave.spawn_command(cmd).with_context(|| {
        format!(
            "spawning simulated shared VM PTY command `{}` in {}",
            resolved.command,
            resolved.cwd.display()
        )
    })?;
    drop(pair.slave);

    let mut pty_reader = pair
        .master
        .try_clone_reader()
        .context("cloning simulated shared VM PTY reader")?;
    let mut pty_writer = pair
        .master
        .take_writer()
        .context("taking simulated shared VM PTY writer")?;
    let master = Arc::new(Mutex::new(pair.master));

    let writer = Arc::new(Mutex::new(
        stream
            .try_clone()
            .context("cloning shared VM stream for PTY output")?,
    ));
    let mut stdin_reader = stream;
    let resize_master = Arc::clone(&master);
    let _stdin_thread = std::thread::spawn(move || loop {
        match read_exec_frame(&mut stdin_reader) {
            Ok(Some(AvfLinuxExecFrame::Stdin(bytes))) => {
                if pty_writer.write_all(&bytes).is_err() {
                    return;
                }
                let _ = pty_writer.flush();
            }
            Ok(Some(AvfLinuxExecFrame::Resize(AvfLinuxExecResize { cols, rows }))) => {
                let Ok(master) = resize_master.lock() else {
                    return;
                };
                if master
                    .resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .is_err()
                {
                    return;
                }
            }
            Ok(Some(AvfLinuxExecFrame::CloseStdin)) | Ok(None) => return,
            Ok(Some(_)) => return,
            Err(_) => return,
        }
    });
    let output_writer = Arc::clone(&writer);
    let stdout_thread = std::thread::spawn(move || {
        relay_pty_output(&mut pty_reader, output_writer);
    });

    let child = Arc::new(Mutex::new(child));
    let exit_code = loop {
        let exit = {
            let mut child = child
                .lock()
                .map_err(|_| anyhow::anyhow!("shared VM PTY child mutex poisoned"))?;
            child.try_wait().ok().flatten()
        };
        if let Some(status) = exit {
            break i32::try_from(status.exit_code()).unwrap_or(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };

    let _ = stdout_thread.join();
    write_exec_frame(
        &mut *writer
            .lock()
            .map_err(|_| anyhow::anyhow!("shared VM PTY writer mutex poisoned"))?,
        &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code }),
    )
    .context("writing shared VM PTY exit frame")?;
    Ok(())
}

#[derive(Debug)]
struct SimulatedExecRequest {
    command: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: HashMap<String, String>,
}

struct GuestExecCaptureResult {
    exit_code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn resolve_simulated_exec_request(
    data_root: &Path,
    request: &AvfLinuxExecRequest,
) -> Result<SimulatedExecRequest> {
    let cwd = resolve_host_path_for_guest_path(data_root, Path::new(&request.cwd))?;
    let command = rewrite_bundled_path_for_host_simulation(&request.command);
    let args = request
        .args
        .iter()
        .map(|arg| rewrite_bundled_path_for_host_simulation(arg))
        .collect::<Vec<_>>();
    let env = request
        .env
        .iter()
        .map(|(key, value)| {
            let rewritten = if key == "PATH" {
                rewrite_path_list_for_host_simulation(value)
            } else if key.ends_with("_PATH") || key.ends_with("_ROOT") || key.ends_with("_DIR") {
                rewrite_bundled_path_for_host_simulation(value)
            } else {
                value.to_string()
            };
            (key.clone(), rewritten)
        })
        .collect::<HashMap<_, _>>();
    Ok(SimulatedExecRequest {
        command,
        args,
        cwd,
        env,
    })
}

fn resolve_host_path_for_guest_path(data_root: &Path, guest_path: &Path) -> Result<PathBuf> {
    if guest_path == Path::new("/") {
        return Ok(shared_vm_root(data_root));
    }
    let worktrees_root = shared_vm_worktrees_root(data_root);
    if !worktrees_root.exists() {
        bail!(
            "shared VM worktree metadata root is missing at {}",
            worktrees_root.display()
        );
    }
    for workspace_dir in fs::read_dir(&worktrees_root)
        .with_context(|| format!("reading {}", worktrees_root.display()))?
    {
        let workspace_dir = workspace_dir?.path();
        if !workspace_dir.is_dir() {
            continue;
        }
        for worktree_dir in fs::read_dir(&workspace_dir)
            .with_context(|| format!("reading {}", workspace_dir.display()))?
        {
            let worktree_dir = worktree_dir?.path();
            let metadata_path = worktree_dir.join(GUEST_WORKTREE_METADATA_FILE);
            let Some(metadata) = load_guest_worktree_state(&metadata_path)? else {
                continue;
            };
            let guest_root = metadata.guest_root;
            if guest_path == guest_root {
                return Ok(metadata.host_shadow_root);
            }
            if guest_path.starts_with(&guest_root) {
                let relative = guest_path
                    .strip_prefix(&guest_root)
                    .with_context(|| format!("mapping {}", guest_path.display()))?;
                return Ok(join_relative_path(&metadata.host_shadow_root, relative));
            }
        }
    }
    bail!(
        "no helper-staged worktree metadata found for guest path {}",
        guest_path.display()
    );
}

fn join_relative_path(root: &Path, relative: &Path) -> PathBuf {
    let mut out = root.to_path_buf();
    if relative != Path::new("") {
        out.push(relative);
    }
    out
}

fn rewrite_path_list_for_host_simulation(value: &str) -> String {
    if value.trim().is_empty() {
        return value.to_string();
    }
    let rewritten = std::env::split_paths(std::ffi::OsStr::new(value))
        .map(|entry| {
            PathBuf::from(rewrite_bundled_path_for_host_simulation(
                entry.to_string_lossy().as_ref(),
            ))
        })
        .collect::<Vec<_>>();
    std::env::join_paths(rewritten)
        .map(|joined| joined.to_string_lossy().to_string())
        .unwrap_or_else(|_| value.to_string())
}

fn rewrite_bundled_path_for_host_simulation(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return value.to_string();
    }
    let sep = if trimmed.contains('\\') { '\\' } else { '/' };
    for marker in ["providers", "runtimes"] {
        let needle = format!("{sep}{marker}{sep}");
        let Some(idx) = trimmed.find(&needle) else {
            continue;
        };
        let rest = &trimmed[idx + needle.len()..];
        let mut parts = rest.split(sep);
        let Some(id) = parts.next() else {
            continue;
        };
        let Some(os) = parts.next() else {
            continue;
        };
        let Some(_arch) = parts.next() else {
            continue;
        };
        if os != "linux" {
            return value.to_string();
        }
        let tail = parts.collect::<Vec<_>>().join(&sep.to_string());
        let prefix = &trimmed[..idx];
        let candidate = format!(
            "{prefix}{needle}{id}{sep}macos{sep}{}{sep}{tail}",
            std::env::consts::ARCH
        );
        if Path::new(&candidate).exists() {
            return candidate;
        }
    }
    value.to_string()
}

#[cfg(unix)]
fn io_error_is_benign(err: &std::io::Error) -> bool {
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
fn relay_socket_copy(mut reader: impl Read, mut writer: impl Write) -> Result<()> {
    match std::io::copy(&mut reader, &mut writer) {
        Ok(_) => Ok(()),
        Err(err) if io_error_is_benign(&err) => Ok(()),
        Err(err) => Err(err).context("relaying socket bytes"),
    }
}

#[cfg(all(target_os = "macos", unix))]
fn connect_shared_vm_guest_control_socket(
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
fn relay_shared_vm_control_client(client: UnixStream, guest: File) -> Result<()> {
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
fn service_real_shared_vm_control_clients(
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
fn shutdown_real_shared_vm_for_exit(
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

fn wrap_cloud_init_base64(bytes: &[u8]) -> String {
    let encoded = BASE64_STANDARD.encode(bytes);
    let mut wrapped = String::new();
    for chunk in encoded.as_bytes().chunks(76) {
        if !wrapped.is_empty() {
            wrapped.push('\n');
        }
        wrapped.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    wrapped
}

fn indent_cloud_init_block(content: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    content
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_shared_vm_guest_agent_service() -> String {
    format!(
        "[Unit]\nDescription=ctx AVF Linux Guest Agent\n\n[Service]\nType=simple\nEnvironment=RUST_BACKTRACE=1\nExecStartPre=/bin/sh -lc 'echo \"[ctx-avf-linux] starting guest-agent\" >/dev/hvc0'\nExecStart=/bin/sh -lc 'exec /usr/local/bin/ctx-avf-linux-guest-agent'\nStandardOutput=journal+console\nStandardError=journal+console\nRestart=always\nRestartSec=1\n\n[Install]\nWantedBy=multi-user.target\n# {}\n",
        SHARED_VM_GUEST_AGENT_SERVICE_NAME
    )
}

fn hash_shared_vm_seed_component(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn render_shared_vm_cloud_init_meta_data(
    guest_agent_bytes: &[u8],
    egress_proxy_bytes: Option<&[u8]>,
) -> String {
    let mut seed_material = Vec::with_capacity(guest_agent_bytes.len() + 256);
    seed_material.extend_from_slice(guest_agent_bytes);
    if let Some(egress_proxy_bytes) = egress_proxy_bytes {
        seed_material.extend_from_slice(egress_proxy_bytes);
    }
    seed_material.extend_from_slice(render_shared_vm_guest_agent_service().as_bytes());
    let seed_hash = hash_shared_vm_seed_component(&seed_material);
    format!("instance-id: ctx-avf-linux-{seed_hash}\nlocal-hostname: ctx-avf-linux\n")
}

fn render_shared_vm_cloud_init_user_data(
    guest_agent_bytes: &[u8],
    egress_proxy_bytes: Option<&[u8]>,
) -> String {
    let guest_agent_b64 = indent_cloud_init_block(&wrap_cloud_init_base64(guest_agent_bytes), 6);
    let service = indent_cloud_init_block(&render_shared_vm_guest_agent_service(), 6);
    let egress_proxy_block = egress_proxy_bytes.map(|bytes| {
        let egress_proxy_b64 = indent_cloud_init_block(&wrap_cloud_init_base64(bytes), 6);
        format!(
            "  - path: /usr/local/bin/ctx-egress-proxy\n    permissions: '0755'\n    encoding: b64\n    content: |\n{egress_proxy_b64}\n"
        )
    });
    format!(
        "#cloud-config\nwrite_files:\n  - path: /usr/local/bin/ctx-avf-linux-guest-agent\n    permissions: '0755'\n    encoding: b64\n    content: |\n{guest_agent_b64}\n{egress_proxy_block}  - path: /etc/systemd/system/{service_name}\n    permissions: '0644'\n    content: |\n{service}\nruncmd:\n  - [ sh, -lc, 'echo \"[ctx-avf-linux] preparing {service_name}\" >/dev/hvc0; ls -l /usr/local/bin/ctx-avf-linux-guest-agent >/dev/hvc0 2>&1; ls -l /etc/systemd/system/{service_name} >/dev/hvc0 2>&1' ]\n  - [ systemctl, daemon-reload ]\n  - [ sh, -lc, 'systemctl enable --now {service_name} >/dev/hvc0 2>&1 || (systemctl status {service_name} --no-pager >/dev/hvc0 2>&1; exit 1)' ]\n",
        service_name = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        egress_proxy_block = egress_proxy_block.unwrap_or_default(),
    )
}

fn render_shared_vm_cloud_init_network_config() -> &'static str {
    "version: 2\nethernets:\n  default:\n    match:\n      name: \"en*\"\n    dhcp4: true\n    optional: true\n"
}

fn stage_shared_vm_cloud_init_seed(
    data_root: &Path,
    runtime_root: &Path,
    preserve_existing_image: bool,
) -> Result<Option<PathBuf>> {
    let guest_agent_path = shared_vm_guest_agent_helper_path(runtime_root);
    if !guest_agent_path.is_file() {
        return Ok(None);
    }
    let egress_proxy_path = shared_vm_egress_proxy_helper_path(runtime_root);
    let image_path = shared_vm_cloud_init_image_path(data_root);
    if preserve_existing_image && image_path.is_file() {
        return Ok(Some(image_path));
    }

    let seed_root = shared_vm_cloud_init_root(data_root);
    fs::remove_dir_all(&seed_root).ok();
    fs::create_dir_all(&seed_root).with_context(|| format!("creating {}", seed_root.display()))?;
    let guest_agent_bytes = fs::read(&guest_agent_path)
        .with_context(|| format!("reading {}", guest_agent_path.display()))?;
    let egress_proxy_bytes = if egress_proxy_path.is_file() {
        Some(
            fs::read(&egress_proxy_path)
                .with_context(|| format!("reading {}", egress_proxy_path.display()))?,
        )
    } else {
        None
    };
    fs::write(
        shared_vm_cloud_init_meta_data_path(data_root),
        render_shared_vm_cloud_init_meta_data(&guest_agent_bytes, egress_proxy_bytes.as_deref()),
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_meta_data_path(data_root).display()
        )
    })?;
    fs::write(
        shared_vm_cloud_init_user_data_path(data_root),
        render_shared_vm_cloud_init_user_data(&guest_agent_bytes, egress_proxy_bytes.as_deref()),
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_user_data_path(data_root).display()
        )
    })?;
    fs::write(
        shared_vm_cloud_init_network_config_path(data_root),
        render_shared_vm_cloud_init_network_config(),
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_network_config_path(data_root).display()
        )
    })?;

    fs::remove_file(&image_path).ok();
    let image = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&image_path)
        .with_context(|| format!("creating {}", image_path.display()))?;
    image
        .set_len(16 * 1024 * 1024)
        .with_context(|| format!("sizing {}", image_path.display()))?;
    drop(image);

    let raw_device = attach_raw_disk_image_nomount(&image_path)?;
    let mut raw_device_attached = true;
    let mut mounted_device: Option<String> = None;
    let mut mounted_volume: Option<PathBuf> = None;
    let result = (|| -> Result<()> {
        run_command(
            "diskutil",
            &[
                "partitionDisk",
                &raw_device,
                "MBR",
                "MS-DOS",
                "CIDATA",
                "100%",
            ],
            "partitioning raw cloud-init image",
        )?;
        detach_disk_image_device(&raw_device)?;
        raw_device_attached = false;
        let (device, volume_path) = attach_raw_disk_image_with_mount(&image_path)?;
        mounted_device = Some(device);
        mounted_volume = Some(volume_path.clone());
        fs::copy(
            shared_vm_cloud_init_meta_data_path(data_root),
            volume_path.join("meta-data"),
        )
        .context("copying cloud-init meta-data into mounted seed volume")?;
        fs::copy(
            shared_vm_cloud_init_user_data_path(data_root),
            volume_path.join("user-data"),
        )
        .context("copying cloud-init user-data into mounted seed volume")?;
        fs::copy(
            shared_vm_cloud_init_network_config_path(data_root),
            volume_path.join("network-config"),
        )
        .context("copying cloud-init network-config into mounted seed volume")?;
        Ok(())
    })();

    if let Some(device) = mounted_device.as_deref() {
        let _ = detach_disk_image_device(device);
    }
    if raw_device_attached {
        let _ = detach_disk_image_device(&raw_device);
    }
    result?;

    Ok(Some(image_path))
}

fn run_command(program: &str, args: &[&str], context_label: &str) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("spawning {program} for {context_label}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let details = [stdout, stderr]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if details.is_empty() {
        bail!("{context_label} failed with status {}", output.status);
    }
    bail!(
        "{context_label} failed with status {}:\n{details}",
        output.status
    );
}

fn attach_raw_disk_image_nomount(image_path: &Path) -> Result<String> {
    let output = run_command(
        "hdiutil",
        &[
            "attach",
            "-nomount",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
            &image_path.display().to_string(),
        ],
        "attaching raw cloud-init image without mounting",
    )?;
    output
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .filter(|value| value.starts_with("/dev/"))
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("could not determine raw device from hdiutil output"))
}

fn attach_raw_disk_image_with_mount(image_path: &Path) -> Result<(String, PathBuf)> {
    let output = run_command(
        "hdiutil",
        &[
            "attach",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
            &image_path.display().to_string(),
        ],
        "attaching raw cloud-init image with mount",
    )?;
    for line in output.lines() {
        let parts = line
            .split('\t')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }
        let device = parts[0];
        let mount = parts[2];
        if device.starts_with("/dev/") && mount.starts_with("/Volumes/") {
            return Ok((device.to_string(), PathBuf::from(mount)));
        }
    }
    bail!("could not determine mounted cloud-init volume from hdiutil output");
}

fn detach_disk_image_device(device: &str) -> Result<()> {
    let _ = run_command(
        "hdiutil",
        &["detach", device],
        "detaching raw cloud-init image",
    )?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_shared_vm(data_root: &Path) -> Result<()> {
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
fn run_shared_vm(_data_root: &Path) -> Result<()> {
    bail!("real shared AVF Linux VM ownership requires macOS")
}

fn spawn_real_shared_vm_owner(data_root: &Path) -> Result<u32> {
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

fn spawn_shared_vm_server(data_root: &Path) -> Result<u32> {
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

fn spawn_guest_agent_server(data_root: &Path) -> Result<u32> {
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

fn wait_for_control_socket(data_root: &Path) -> Result<()> {
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

fn wait_for_guest_agent_socket(data_root: &Path) -> Result<()> {
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
fn wait_for_real_guest_exec_ready(data_root: &Path) -> Result<()> {
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
fn wait_for_real_guest_exec_ready(_data_root: &Path) -> Result<()> {
    Ok(())
}

fn request_shared_vm_shutdown(data_root: &Path) -> Result<()> {
    let path = shared_vm_shutdown_request_path(data_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, now_timestamp_string()).with_context(|| format!("writing {}", path.display()))
}

fn clear_shared_vm_shutdown_request(data_root: &Path) {
    let path = shared_vm_shutdown_request_path(data_root);
    if path.exists() {
        let _ = fs::remove_file(path);
    }
}

fn shared_vm_shutdown_requested(data_root: &Path) -> bool {
    shared_vm_shutdown_request_path(data_root).exists()
}

fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
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
fn shared_vm_server_process_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    err.raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn shared_vm_server_process_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(unix)]
fn stop_shared_vm_server(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn stop_shared_vm_server(pid: u32) {
    let _ = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
}

fn append_shared_vm_log_line(data_root: &Path, line: &str) -> Result<()> {
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

fn prepare_guest_worktree(
    data_root: &Path,
    workspace_id: &str,
    worktree_id: &str,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<AvfLinuxGuestWorktreeResponse> {
    let shared_vm = shared_vm_state(data_root)?;
    if !matches!(shared_vm.state, AvfLinuxSharedVmLifecycleState::Running) {
        bail!(
            "shared AVF Linux VM must be running before preparing guest worktrees (state={:?})",
            shared_vm.state
        );
    }
    if !host_workspace_root.is_dir() {
        bail!(
            "host workspace root does not exist: {}",
            host_workspace_root.display()
        );
    }

    let guest_root = guest_worktree_root(worktree_id);
    let guest_user = guest_workspace_user(workspace_id);
    let host_shadow_root = shared_vm_worktree_shadow_root(data_root, workspace_id, worktree_id);
    let metadata_path = shared_vm_worktree_metadata_path(data_root, workspace_id, worktree_id);

    if let Some(existing) = load_guest_worktree_state(&metadata_path)? {
        let existing_guest_user = if existing.guest_user.trim().is_empty() {
            guest_user.clone()
        } else {
            existing.guest_user.clone()
        };
        let matches_request = existing.workspace_id == workspace_id
            && existing.worktree_id == worktree_id
            && existing.host_workspace_root == host_workspace_root
            && existing.base_commit_sha == base_commit_sha
            && existing.branch_name == branch_name
            && existing.host_shadow_root == host_shadow_root
            && existing.guest_root == guest_root
            && host_shadow_root.join(".git").exists();
        if matches_request {
            if !shared_vm.simulated {
                ensure_guest_workspace_user(data_root, &existing_guest_user)?;
                if guest_directory_exists(data_root, &guest_root)? {
                    finalize_guest_worktree_permissions(
                        data_root,
                        &guest_root,
                        &existing_guest_user,
                    )?;
                    return Ok(map_guest_worktree_response(
                        workspace_id,
                        worktree_id,
                        guest_root,
                        existing_guest_user,
                        host_shadow_root,
                        metadata_path,
                        AvfLinuxGuestWorktreeStatus::AlreadyPresent,
                        existing.simulated,
                        existing.notes,
                    ));
                }

                materialize_guest_worktree_from_shadow_root(
                    data_root,
                    &host_shadow_root,
                    &guest_root,
                )?;
                finalize_guest_worktree_permissions(data_root, &guest_root, &existing_guest_user)?;

                let mut notes = existing.notes;
                notes.push(format!(
                    "guest worktree was rematerialized at {} because the prior guest path was missing after VM restart",
                    guest_root.display()
                ));
                let persisted = PersistedGuestWorktreeState {
                    workspace_id: workspace_id.to_string(),
                    worktree_id: worktree_id.to_string(),
                    host_workspace_root: host_workspace_root.to_path_buf(),
                    guest_root: guest_root.clone(),
                    guest_user: existing_guest_user.clone(),
                    host_shadow_root: host_shadow_root.clone(),
                    base_commit_sha: base_commit_sha.to_string(),
                    branch_name: branch_name.to_string(),
                    updated_at: now_timestamp_string(),
                    simulated: false,
                    notes: notes.clone(),
                };
                persist_guest_worktree_state(&metadata_path, &persisted)?;

                return Ok(map_guest_worktree_response(
                    workspace_id,
                    worktree_id,
                    guest_root,
                    existing_guest_user,
                    host_shadow_root,
                    metadata_path,
                    AvfLinuxGuestWorktreeStatus::Prepared,
                    false,
                    notes,
                ));
            }
            return Ok(map_guest_worktree_response(
                workspace_id,
                worktree_id,
                guest_root,
                existing_guest_user,
                host_shadow_root,
                metadata_path,
                AvfLinuxGuestWorktreeStatus::AlreadyPresent,
                existing.simulated,
                existing.notes,
            ));
        }
    }

    best_effort_remove_git_worktree(host_workspace_root, &host_shadow_root);
    if host_shadow_root.exists() {
        fs::remove_dir_all(&host_shadow_root)
            .with_context(|| format!("removing {}", host_shadow_root.display()))?;
    }
    if let Some(parent) = host_shadow_root.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    run_git_worktree_add(
        host_workspace_root,
        &host_shadow_root,
        base_commit_sha,
        branch_name,
    )?;

    let (simulated, notes) = if shared_vm.simulated {
        (
            true,
            vec![
                "guest worktree is staged through the helper-owned shadow root until full AVF guest filesystem import lands".to_string(),
                format!("guest root planned at {}", guest_root.display()),
                format!("guest worktree user reserved as {guest_user}"),
            ],
        )
    } else {
        ensure_guest_workspace_user(data_root, &guest_user)?;
        materialize_guest_worktree_from_shadow_root(data_root, &host_shadow_root, &guest_root)?;
        finalize_guest_worktree_permissions(data_root, &guest_root, &guest_user)?;
        (
            false,
            vec![
                format!(
                    "guest worktree imported from helper shadow root {}",
                    host_shadow_root.display()
                ),
                format!("guest root materialized at {}", guest_root.display()),
                format!("guest worktree user ensured as {guest_user}"),
            ],
        )
    };
    let persisted = PersistedGuestWorktreeState {
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        host_workspace_root: host_workspace_root.to_path_buf(),
        guest_root: guest_root.clone(),
        guest_user: guest_user.clone(),
        host_shadow_root: host_shadow_root.clone(),
        base_commit_sha: base_commit_sha.to_string(),
        branch_name: branch_name.to_string(),
        updated_at: now_timestamp_string(),
        simulated,
        notes: notes.clone(),
    };
    persist_guest_worktree_state(&metadata_path, &persisted)?;

    Ok(map_guest_worktree_response(
        workspace_id,
        worktree_id,
        guest_root,
        guest_user,
        host_shadow_root,
        metadata_path,
        AvfLinuxGuestWorktreeStatus::Prepared,
        simulated,
        notes,
    ))
}

fn guest_directory_exists(data_root: &Path, guest_path: &Path) -> Result<bool> {
    let result = run_guest_exec_capture(
        &shared_vm_control_socket_path(data_root),
        Path::new("/"),
        "/usr/bin/test",
        &[String::from("-d"), guest_path.display().to_string()],
        Some("root"),
        HashMap::new(),
        None,
    )
    .with_context(|| {
        format!(
            "checking whether guest path {} exists",
            guest_path.display()
        )
    })?;
    Ok(result.exit_code == 0)
}

fn guest_exec(
    data_root: &Path,
    workspace_id: &str,
    worktree_id: &str,
    cwd: &Path,
    command: &str,
    env: &[String],
    user: Option<&str>,
    pty: bool,
    args: &[String],
) -> Result<i32> {
    let shared_vm = shared_vm_state(data_root)?;
    if !matches!(shared_vm.state, AvfLinuxSharedVmLifecycleState::Running) {
        bail!(
            "shared AVF Linux VM must be running before guest exec (state={:?})",
            shared_vm.state
        );
    }

    let metadata_path = shared_vm_worktree_metadata_path(data_root, workspace_id, worktree_id);
    let Some(worktree) = load_guest_worktree_state(&metadata_path)? else {
        bail!(
            "guest worktree metadata is missing for workspace {} worktree {}",
            workspace_id,
            worktree_id
        );
    };
    if !cwd.starts_with(&worktree.guest_root) {
        bail!(
            "guest exec cwd {} must stay under guest worktree root {}",
            cwd.display(),
            worktree.guest_root.display()
        );
    }

    let control_socket = shared_vm_control_socket_path(data_root);
    let guest_env = parse_guest_exec_env(env)?;
    let guest_user = user
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            if worktree.guest_user.trim().is_empty() {
                Some(guest_workspace_user(workspace_id))
            } else {
                Some(worktree.guest_user.clone())
            }
        });
    run_guest_exec_process(
        &control_socket,
        cwd,
        command,
        args,
        guest_user.as_deref(),
        guest_env,
        pty,
    )
    .with_context(|| {
        format!(
            "running AVF Linux guest exec for workspace {} worktree {}",
            workspace_id, worktree_id
        )
    })
}

fn parse_guest_exec_env(env: &[String]) -> Result<HashMap<String, String>> {
    let mut parsed = HashMap::new();
    for entry in env {
        let Some((key, value)) = entry.split_once('=') else {
            bail!("guest exec env entry must be KEY=VALUE, got `{entry}`");
        };
        let key = key.trim();
        if key.is_empty() {
            bail!("guest exec env key must not be empty");
        }
        if key.starts_with("CTX_AVF_") {
            bail!("guest exec env key `{key}` is reserved for helper control state");
        }
        parsed.insert(key.to_string(), value.to_string());
    }
    Ok(parsed)
}

fn guest_workspace_user(workspace_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(workspace_id.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{GUEST_WORKSPACE_USER_PREFIX}{}", &digest[..12])
}

fn guest_workspace_home(guest_user: &str) -> PathBuf {
    PathBuf::from(GUEST_WORKSPACE_HOMES_ROOT).join(guest_user)
}

fn ensure_guest_workspace_user(data_root: &Path, guest_user: &str) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    ensure_guest_exec_success(
        "creating guest workspace account roots",
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/mkdir",
            &[
                String::from("-p"),
                GUEST_WORKSPACE_HOMES_ROOT.to_string(),
                GUEST_WORKSPACE_CACHE_ROOT.to_string(),
                GUEST_WORKSPACE_TMP_ROOT.to_string(),
            ],
            None,
            HashMap::new(),
            None,
        )?,
    )?;

    let user_exists = run_guest_exec_capture(
        &control_socket,
        Path::new("/"),
        "/usr/bin/id",
        &[String::from("-u"), guest_user.to_string()],
        None,
        HashMap::new(),
        None,
    )?
    .exit_code
        == 0;
    if !user_exists {
        ensure_guest_exec_success(
            &format!("creating guest workspace user {guest_user}"),
            run_guest_exec_capture(
                &control_socket,
                Path::new("/"),
                "/usr/sbin/useradd",
                &[
                    String::from("--create-home"),
                    String::from("--home-dir"),
                    guest_workspace_home(guest_user).display().to_string(),
                    String::from("--shell"),
                    String::from("/bin/bash"),
                    guest_user.to_string(),
                ],
                None,
                HashMap::new(),
                None,
            )?,
        )?;
    }

    ensure_guest_exec_success(
        &format!("ensuring guest workspace directories for {guest_user}"),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/usr/bin/install",
            &[
                String::from("-d"),
                String::from("-o"),
                guest_user.to_string(),
                String::from("-g"),
                guest_user.to_string(),
                String::from("-m"),
                String::from("700"),
                guest_workspace_home(guest_user).display().to_string(),
                PathBuf::from(GUEST_WORKSPACE_CACHE_ROOT)
                    .join(guest_user)
                    .display()
                    .to_string(),
                PathBuf::from(GUEST_WORKSPACE_TMP_ROOT)
                    .join(guest_user)
                    .display()
                    .to_string(),
            ],
            None,
            HashMap::new(),
            None,
        )?,
    )
}

fn finalize_guest_worktree_permissions(
    data_root: &Path,
    guest_root: &Path,
    guest_user: &str,
) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    ensure_guest_exec_success(
        &format!(
            "setting guest worktree ownership on {}",
            guest_root.display()
        ),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/chown",
            &[
                String::from("-R"),
                format!("{guest_user}:{guest_user}"),
                guest_root.display().to_string(),
            ],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    ensure_guest_exec_success(
        &format!("restricting guest worktree root {}", guest_root.display()),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/chmod",
            &[String::from("700"), guest_root.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )
}

#[cfg(unix)]
fn run_guest_exec_capture(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    stdin_reader: Option<&mut dyn Read>,
) -> Result<GuestExecCaptureResult> {
    if command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if cwd.as_os_str().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let mut stream = connect_shared_vm_control_socket(control_socket)?;
    let request = AvfLinuxExecRequest::new(
        command,
        args.to_vec(),
        cwd.display().to_string(),
        user.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        env,
        false,
    );
    write_exec_frame(&mut stream, &AvfLinuxExecFrame::Request(request))
        .context("writing AVF Linux guest exec capture request")?;

    if let Some(reader) = stdin_reader {
        let mut buf = [0u8; 8192];
        loop {
            let read = reader
                .read(&mut buf)
                .context("reading staged guest exec stdin")?;
            if read == 0 {
                break;
            }
            write_exec_frame(&mut stream, &AvfLinuxExecFrame::Stdin(buf[..read].to_vec()))
                .context("writing AVF Linux guest exec stdin frame")?;
        }
    }
    write_exec_frame(&mut stream, &AvfLinuxExecFrame::CloseStdin)
        .context("closing AVF Linux guest exec stdin")?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    loop {
        match read_exec_frame(&mut stream).context("reading AVF Linux guest exec capture frame")? {
            Some(AvfLinuxExecFrame::Stdout(bytes)) => stdout.extend_from_slice(&bytes),
            Some(AvfLinuxExecFrame::Stderr(bytes)) => stderr.extend_from_slice(&bytes),
            Some(AvfLinuxExecFrame::Exit(exit)) => {
                return Ok(GuestExecCaptureResult {
                    exit_code: exit.exit_code,
                    stdout,
                    stderr,
                });
            }
            Some(AvfLinuxExecFrame::Error(error)) => {
                let stderr_text = String::from_utf8_lossy(&stderr).trim().to_string();
                let stdout_text = String::from_utf8_lossy(&stdout).trim().to_string();
                let extra = [stderr_text, stdout_text]
                    .into_iter()
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                if extra.is_empty() {
                    bail!("guest exec failed: {} ({})", error.message, error.code);
                }
                bail!(
                    "guest exec failed: {} ({})\n{}",
                    error.message,
                    error.code,
                    extra
                );
            }
            Some(
                AvfLinuxExecFrame::Request(_)
                | AvfLinuxExecFrame::Stdin(_)
                | AvfLinuxExecFrame::CloseStdin
                | AvfLinuxExecFrame::Resize(_),
            ) => bail!("received unexpected frame while waiting for guest exec result"),
            None => bail!("shared VM control socket closed before guest exec exit"),
        }
    }
}

#[cfg(not(unix))]
fn run_guest_exec_capture(
    _control_socket: &Path,
    _cwd: &Path,
    _command: &str,
    _args: &[String],
    _user: Option<&str>,
    _env: HashMap<String, String>,
    _stdin_reader: Option<&mut dyn Read>,
) -> Result<GuestExecCaptureResult> {
    bail!("programmatic AVF guest exec capture requires unix domain sockets")
}

fn format_guest_exec_output(output: &[u8]) -> String {
    String::from_utf8_lossy(output).trim().to_string()
}

fn ensure_guest_exec_success(context: &str, result: GuestExecCaptureResult) -> Result<()> {
    if result.exit_code == 0 {
        return Ok(());
    }
    let stdout = format_guest_exec_output(&result.stdout);
    let stderr = format_guest_exec_output(&result.stderr);
    let joined = [stderr, stdout]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if joined.is_empty() {
        bail!("{context} failed with exit code {}", result.exit_code);
    }
    bail!(
        "{context} failed with exit code {}:\n{}",
        result.exit_code,
        joined
    );
}

fn stage_guest_worktree_archive_path(worktree_id: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "ctx-avf-linux-worktree-{worktree_id}-{}-{millis}.tar",
        std::process::id()
    ))
}

fn materialize_guest_worktree_from_shadow_root(
    data_root: &Path,
    host_shadow_root: &Path,
    guest_root: &Path,
) -> Result<()> {
    let control_socket = shared_vm_control_socket_path(data_root);
    let guest_root_parent = guest_root.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "guest worktree root has no parent: {}",
            guest_root.display()
        )
    })?;

    ensure_guest_exec_success(
        &format!(
            "creating guest worktree parent {}",
            guest_root_parent.display()
        ),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/mkdir",
            &[String::from("-p"), guest_root_parent.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    ensure_guest_exec_success(
        &format!("clearing guest worktree root {}", guest_root.display()),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/rm",
            &[String::from("-rf"), guest_root.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    ensure_guest_exec_success(
        &format!("creating guest worktree root {}", guest_root.display()),
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/bin/mkdir",
            &[String::from("-p"), guest_root.display().to_string()],
            None,
            HashMap::new(),
            None,
        )?,
    )?;
    if !guest_directory_exists(data_root, guest_root)? {
        bail!(
            "guest worktree root {} is still missing immediately after creation",
            guest_root.display()
        );
    }

    let archive_path = stage_guest_worktree_archive_path(
        guest_root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("worktree"),
    );
    let archive_status = Command::new("tar")
        .env("COPYFILE_DISABLE", "1")
        .arg("-C")
        .arg(host_shadow_root)
        .arg("-cf")
        .arg(&archive_path)
        .arg(".")
        .status()
        .with_context(|| {
            format!(
                "creating guest worktree archive from {}",
                host_shadow_root.display()
            )
        })?;
    if !archive_status.success() {
        bail!(
            "creating guest worktree archive from {} failed with status {}",
            host_shadow_root.display(),
            archive_status
        );
    }

    let import_result = (|| -> Result<GuestExecCaptureResult> {
        let mut archive_file = std::fs::File::open(&archive_path)
            .with_context(|| format!("opening {}", archive_path.display()))?;
        run_guest_exec_capture(
            &control_socket,
            Path::new("/"),
            "/usr/bin/tar",
            &[
                String::from("-xpf"),
                String::from("-"),
                String::from("-C"),
                guest_root.display().to_string(),
            ],
            None,
            HashMap::new(),
            Some(&mut archive_file),
        )
    })();
    let _ = fs::remove_file(&archive_path);
    ensure_guest_exec_success(
        &format!(
            "importing staged worktree {} into guest root {}",
            host_shadow_root.display(),
            guest_root.display()
        ),
        import_result?,
    )?;
    if !guest_directory_exists(data_root, guest_root)? {
        bail!(
            "guest worktree root {} disappeared after importing staged worktree {}",
            guest_root.display(),
            host_shadow_root.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn connect_shared_vm_control_socket(socket_path: &Path) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + GUEST_EXEC_CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    || err.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if std::time::Instant::now() >= deadline {
                    return Err(err).with_context(|| {
                        format!(
                            "connecting to shared VM control socket {}",
                            socket_path.display()
                        )
                    });
                }
                std::thread::sleep(GUEST_EXEC_CONNECT_RETRY_INTERVAL);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "connecting to shared VM control socket {}",
                        socket_path.display()
                    )
                });
            }
        }
    }
}

#[cfg(unix)]
fn connect_guest_agent_control_socket(socket_path: &Path) -> Result<UnixStream> {
    let deadline = std::time::Instant::now() + GUEST_EXEC_CONNECT_TIMEOUT;
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => return Ok(stream),
            Err(err)
                if err.kind() == std::io::ErrorKind::NotFound
                    || err.kind() == std::io::ErrorKind::ConnectionRefused =>
            {
                if std::time::Instant::now() >= deadline {
                    return Err(err).with_context(|| {
                        format!(
                            "connecting to guest-agent control socket {}",
                            socket_path.display()
                        )
                    });
                }
                std::thread::sleep(GUEST_EXEC_CONNECT_RETRY_INTERVAL);
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "connecting to guest-agent control socket {}",
                        socket_path.display()
                    )
                });
            }
        }
    }
}

#[cfg(unix)]
fn run_guest_exec_process(
    control_socket: &Path,
    cwd: &Path,
    command: &str,
    args: &[String],
    user: Option<&str>,
    env: HashMap<String, String>,
    pty: bool,
) -> Result<i32> {
    if command.trim().is_empty() {
        bail!("guest exec command must not be empty");
    }
    if cwd.as_os_str().is_empty() {
        bail!("guest exec cwd must not be empty");
    }

    let mut stream = connect_shared_vm_control_socket(control_socket)?;
    let request = AvfLinuxExecRequest::new(
        command,
        args.to_vec(),
        cwd.display().to_string(),
        user.map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        env,
        pty,
    );
    write_exec_frame(&mut stream, &AvfLinuxExecFrame::Request(request))
        .context("writing AVF Linux guest exec request")?;

    let writer =
        Arc::new(Mutex::new(stream.try_clone().context(
            "cloning shared VM control stream for stdin forwarding",
        )?));
    if pty {
        spawn_terminal_resize_forwarder(Arc::clone(&writer));
    }
    let stdin_writer = Arc::clone(&writer);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 8192];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => {
                    let Ok(mut guard) = stdin_writer.lock() else {
                        return;
                    };
                    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                    return;
                }
                Ok(n) => {
                    let Ok(mut guard) = stdin_writer.lock() else {
                        return;
                    };
                    if write_exec_frame(&mut *guard, &AvfLinuxExecFrame::Stdin(buf[..n].to_vec()))
                        .is_err()
                    {
                        return;
                    }
                }
                Err(_) => {
                    let Ok(mut guard) = stdin_writer.lock() else {
                        return;
                    };
                    let _ = write_exec_frame(&mut *guard, &AvfLinuxExecFrame::CloseStdin);
                    return;
                }
            }
        }
    });

    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    loop {
        match read_exec_frame(&mut stream).context("reading AVF Linux guest exec response")? {
            Some(AvfLinuxExecFrame::Stdout(bytes)) => {
                stdout
                    .write_all(&bytes)
                    .and_then(|_| stdout.flush())
                    .context("writing guest stdout")?;
            }
            Some(AvfLinuxExecFrame::Stderr(bytes)) => {
                stderr
                    .write_all(&bytes)
                    .and_then(|_| stderr.flush())
                    .context("writing guest stderr")?;
            }
            Some(AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code })) => return Ok(exit_code),
            Some(AvfLinuxExecFrame::Error(AvfLinuxExecError { code, message })) => {
                bail!("{code}: {message}");
            }
            Some(other) => {
                bail!("received unexpected AVF Linux exec frame: {other:?}");
            }
            None => {
                bail!(
                    "shared VM control socket {} closed before sending an exit frame",
                    control_socket.display()
                );
            }
        }
    }
}

#[cfg(unix)]
fn spawn_terminal_resize_forwarder(writer: Arc<Mutex<UnixStream>>) {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let fd = stdin.as_raw_fd();
        let mut last_size = None;
        loop {
            let Some((cols, rows)) = current_terminal_size(fd) else {
                return;
            };
            if last_size != Some((cols, rows)) {
                let Ok(mut guard) = writer.lock() else {
                    return;
                };
                if write_exec_frame(
                    &mut *guard,
                    &AvfLinuxExecFrame::Resize(AvfLinuxExecResize { cols, rows }),
                )
                .is_err()
                {
                    return;
                }
                last_size = Some((cols, rows));
            }
            std::thread::sleep(GUEST_EXEC_TTY_RESIZE_POLL_INTERVAL);
        }
    });
}

#[cfg(unix)]
fn current_terminal_size(fd: std::os::fd::RawFd) -> Option<(u16, u16)> {
    unsafe {
        if libc::isatty(fd) != 1 {
            return None;
        }
        let mut winsize = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut winsize) != 0 {
            return None;
        }
        if winsize.ws_col == 0 || winsize.ws_row == 0 {
            return None;
        }
        Some((winsize.ws_col, winsize.ws_row))
    }
}

#[cfg(not(unix))]
fn run_guest_exec_process(
    _control_socket: &Path,
    _cwd: &Path,
    _command: &str,
    _args: &[String],
    _user: Option<&str>,
    _env: HashMap<String, String>,
    _pty: bool,
) -> Result<i32> {
    bail!("AVF Linux guest exec relay requires unix domain sockets")
}

fn map_state_response(
    persisted: Option<&PersistedSharedVmState>,
    vm_root: PathBuf,
    logs_root: PathBuf,
    state_path: PathBuf,
    log_path: PathBuf,
) -> AvfLinuxSharedVmStateResponse {
    let state = persisted
        .map(|state| state.state)
        .unwrap_or(AvfLinuxSharedVmLifecycleState::Missing);
    let saved_state_path = vm_root.join(SHARED_VM_SAVED_STATE_FILE);
    AvfLinuxSharedVmStateResponse {
        protocol_version: HELPER_PROTOCOL_VERSION,
        protocol_schema: HELPER_PROTOCOL_SCHEMA,
        state,
        vm_root,
        logs_root,
        state_path,
        log_path: Some(log_path),
        saved_state_path: Some(saved_state_path.clone()),
        saved_state_exists: saved_state_path.exists(),
        runtime_root: persisted.and_then(|state| state.runtime_root.clone()),
        rootfs_image: persisted.and_then(|state| state.rootfs_image.clone()),
        kernel_path: persisted.and_then(|state| state.kernel_path.clone()),
        initrd_path: persisted.and_then(|state| state.initrd_path.clone()),
        runtime_version: persisted.and_then(|state| state.runtime_version.clone()),
        updated_at: persisted.and_then(|state| state.updated_at.clone()),
        last_started_at: persisted.and_then(|state| state.last_started_at.clone()),
        last_saved_at: persisted.and_then(|state| state.last_saved_at.clone()),
        last_stopped_at: persisted.and_then(|state| state.last_stopped_at.clone()),
        transition_status: persisted.and_then(|state| state.transition_status),
        relay_pid: persisted.and_then(|state| state.relay_pid),
        guest_agent_pid: persisted.and_then(|state| state.guest_agent_pid),
        simulated: persisted.map(|state| state.simulated).unwrap_or(true),
        notes: persisted
            .map(|state| state.notes.clone())
            .unwrap_or_else(|| vec!["shared VM state has not been initialized yet".to_string()]),
    }
}

fn default_stopped_state() -> PersistedSharedVmState {
    PersistedSharedVmState {
        state: AvfLinuxSharedVmLifecycleState::Stopped,
        runtime_root: None,
        rootfs_image: None,
        kernel_path: None,
        initrd_path: None,
        runtime_version: None,
        updated_at: Some(now_timestamp_string()),
        last_started_at: None,
        last_saved_at: None,
        last_stopped_at: None,
        transition_status: None,
        relay_pid: None,
        guest_agent_pid: None,
        simulated: true,
        notes: vec![
            "shared VM lifecycle scaffold is present, but actual AVF guest boot is not implemented yet".to_string(),
        ],
    }
}

fn load_state(path: &Path) -> Result<Option<PersistedSharedVmState>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(parsed))
}

fn persist_state(path: &Path, state: &PersistedSharedVmState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(state).context("serializing shared VM state")?;
    fs::write(path, raw).with_context(|| format!("writing {}", path.display()))
}

fn load_guest_worktree_state(path: &Path) -> Result<Option<PersistedGuestWorktreeState>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(parsed))
}

fn persist_guest_worktree_state(path: &Path, state: &PersistedGuestWorktreeState) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_vec_pretty(state).context("serializing guest worktree state")?;
    fs::write(path, raw).with_context(|| format!("writing {}", path.display()))
}

fn shared_vm_root(data_root: &Path) -> PathBuf {
    data_root
        .join("managed")
        .join("vms")
        .join("avf-linux")
        .join(std::env::consts::OS)
        .join(std::env::consts::ARCH)
        .join("shared")
}

fn shared_vm_logs_root(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join("logs")
}

fn shared_vm_state_path(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_STATE_FILE)
}

fn shared_vm_log_path(data_root: &Path) -> PathBuf {
    shared_vm_logs_root(data_root).join(SHARED_VM_LOG_FILE)
}

fn shared_vm_saved_state_path(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_SAVED_STATE_FILE)
}

fn shared_vm_machine_identifier_path(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_MACHINE_IDENTIFIER_FILE)
}

fn shared_vm_mac_address_path(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_MAC_ADDRESS_FILE)
}

fn shared_vm_shutdown_request_path(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_SHUTDOWN_REQUEST_FILE)
}

fn shared_vm_guest_console_log_path(data_root: &Path) -> PathBuf {
    shared_vm_logs_root(data_root).join(SHARED_VM_GUEST_CONSOLE_LOG_FILE)
}

fn shared_vm_kernel_cmdline_path(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("helpers")
        .join(SHARED_VM_KERNEL_CMDLINE_FILE)
}

fn shared_vm_cloud_init_root(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(SHARED_VM_CLOUD_INIT_DIR)
}

fn shared_vm_cloud_init_meta_data_path(data_root: &Path) -> PathBuf {
    shared_vm_cloud_init_root(data_root).join(SHARED_VM_CLOUD_INIT_META_DATA_FILE)
}

fn shared_vm_cloud_init_user_data_path(data_root: &Path) -> PathBuf {
    shared_vm_cloud_init_root(data_root).join(SHARED_VM_CLOUD_INIT_USER_DATA_FILE)
}

fn shared_vm_cloud_init_network_config_path(data_root: &Path) -> PathBuf {
    shared_vm_cloud_init_root(data_root).join(SHARED_VM_CLOUD_INIT_NETWORK_CONFIG_FILE)
}

fn shared_vm_cloud_init_image_path(data_root: &Path) -> PathBuf {
    shared_vm_cloud_init_root(data_root).join(SHARED_VM_CLOUD_INIT_IMAGE_FILE)
}

fn shared_vm_control_socket_path(data_root: &Path) -> PathBuf {
    shared_vm_control_socket_root().join(format!(
        "{}-{}",
        short_hash(data_root),
        SHARED_VM_CONTROL_SOCKET_FILE
    ))
}

fn shared_vm_guest_agent_socket_path(data_root: &Path) -> PathBuf {
    shared_vm_control_socket_root().join(format!(
        "{}-{}",
        short_hash(data_root),
        SHARED_VM_GUEST_AGENT_SOCKET_FILE
    ))
}

fn shared_vm_control_socket_root() -> PathBuf {
    PathBuf::from("/tmp").join("ctxavf")
}

fn short_hash(path: &Path) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn shared_vm_worktrees_root(data_root: &Path) -> PathBuf {
    shared_vm_root(data_root).join(GUEST_WORKTREES_DIR)
}

fn shared_vm_worktree_root(data_root: &Path, workspace_id: &str, worktree_id: &str) -> PathBuf {
    shared_vm_worktrees_root(data_root)
        .join(workspace_id)
        .join(worktree_id)
}

fn shared_vm_worktree_shadow_root(
    data_root: &Path,
    workspace_id: &str,
    worktree_id: &str,
) -> PathBuf {
    shared_vm_worktree_root(data_root, workspace_id, worktree_id).join(GUEST_WORKTREE_SHADOW_DIR)
}

fn shared_vm_worktree_metadata_path(
    data_root: &Path,
    workspace_id: &str,
    worktree_id: &str,
) -> PathBuf {
    shared_vm_worktree_root(data_root, workspace_id, worktree_id).join(GUEST_WORKTREE_METADATA_FILE)
}

fn guest_worktree_root(worktree_id: &str) -> PathBuf {
    PathBuf::from(GUEST_WORKTREES_ROOT).join(worktree_id)
}

fn map_guest_worktree_response(
    workspace_id: &str,
    worktree_id: &str,
    guest_root: PathBuf,
    guest_user: String,
    host_shadow_root: PathBuf,
    metadata_path: PathBuf,
    status: AvfLinuxGuestWorktreeStatus,
    simulated: bool,
    notes: Vec<String>,
) -> AvfLinuxGuestWorktreeResponse {
    AvfLinuxGuestWorktreeResponse {
        protocol_version: HELPER_PROTOCOL_VERSION,
        protocol_schema: HELPER_PROTOCOL_SCHEMA,
        workspace_id: workspace_id.to_string(),
        worktree_id: worktree_id.to_string(),
        guest_root,
        guest_user,
        host_shadow_root,
        metadata_path,
        status,
        simulated,
        notes,
    }
}

fn best_effort_remove_git_worktree(host_workspace_root: &Path, host_shadow_root: &Path) {
    let _ = Command::new("git")
        .arg("-C")
        .arg(host_workspace_root)
        .arg("worktree")
        .arg("remove")
        .arg("--force")
        .arg(host_shadow_root)
        .output();
    let _ = Command::new("git")
        .arg("-C")
        .arg(host_workspace_root)
        .arg("worktree")
        .arg("prune")
        .output();
}

fn run_git_worktree_add(
    host_workspace_root: &Path,
    host_shadow_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(host_workspace_root)
        .arg("worktree")
        .arg("add")
        .arg("--force")
        .arg("-B")
        .arg(branch_name)
        .arg(host_shadow_root)
        .arg(base_commit_sha)
        .output()
        .with_context(|| {
            format!(
                "spawning git worktree add for {} -> {}",
                host_workspace_root.display(),
                host_shadow_root.display()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let combined = format!("{stderr}\n{stdout}").trim().to_string();
    if combined.is_empty() {
        bail!(
            "git worktree add failed for {} (status: {})",
            host_shadow_root.display(),
            output.status
        );
    }
    bail!(
        "git worktree add failed for {}: {}",
        host_shadow_root.display(),
        combined
    )
}

fn write_json<T>(value: &T) -> Result<()>
where
    T: Serialize,
{
    serde_json::to_writer(std::io::stdout(), value).context("writing AVF Linux helper response")?;
    println!();
    Ok(())
}

fn required_path_arg(value: Option<String>, name: &str) -> Result<PathBuf> {
    let raw = required_string_arg(value, name)?;
    Ok(PathBuf::from(raw))
}

fn required_string_arg(value: Option<String>, name: &str) -> Result<String> {
    let Some(value) = value else {
        bail!("missing required argument `{name}`");
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("argument `{name}` is empty");
    }
    Ok(trimmed.to_string())
}

fn ensure_no_extra_args(mut args: impl Iterator<Item = String>) -> Result<()> {
    if let Some(extra) = args.next() {
        bail!("unexpected extra argument: {extra}");
    }
    Ok(())
}

fn now_timestamp_string() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

fn macos_product_version() -> Option<String> {
    let output = Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8(output.stdout).ok()?;
    let trimmed = version.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_guest_exec_env_rejects_reserved_helper_keys() {
        let err = parse_guest_exec_env(&["CTX_AVF_SECRET=1".to_string()])
            .expect_err("reserved helper keys should be rejected");
        assert!(err.to_string().contains("reserved"));
    }

    #[test]
    fn runtime_without_guest_agent_stays_simulated() {
        let temp = PathBuf::from("/tmp").join(format!(
            "ctxavf-capability-{}-{}",
            std::process::id(),
            now_timestamp_string()
        ));
        if temp.exists() {
            fs::remove_dir_all(&temp).expect("clear tempdir");
        }
        fs::create_dir_all(temp.join("helpers")).expect("create helpers dir");
        let (enabled, note) = shared_vm_runtime_supports_real_guest_exec(&temp);
        assert!(!enabled);
        assert!(note.contains("missing"));
        fs::remove_dir_all(&temp).expect("cleanup tempdir");
    }

    #[test]
    fn runtime_with_guest_agent_can_enable_real_vm_path() {
        let temp = PathBuf::from("/tmp").join(format!(
            "ctxavf-capability-agent-{}-{}",
            std::process::id(),
            now_timestamp_string()
        ));
        if temp.exists() {
            fs::remove_dir_all(&temp).expect("clear tempdir");
        }
        let helper_path = temp.join("helpers").join(AVF_LINUX_GUEST_AGENT_HELPER);
        fs::create_dir_all(helper_path.parent().expect("helper parent")).expect("helpers dir");
        fs::write(&helper_path, b"guest-agent").expect("guest-agent helper");
        let (enabled, note) = shared_vm_runtime_supports_real_guest_exec(&temp);
        assert!(enabled);
        assert!(note.contains("guest-agent payload"));
        fs::remove_dir_all(&temp).expect("cleanup tempdir");
    }

    #[test]
    fn cloud_init_user_data_embeds_guest_agent_and_service() {
        let user_data =
            render_shared_vm_cloud_init_user_data(b"guest-agent", Some(b"egress-proxy"));
        assert!(user_data.contains("#cloud-config"));
        assert!(user_data.contains("/usr/local/bin/ctx-avf-linux-guest-agent"));
        assert!(user_data.contains("/usr/local/bin/ctx-egress-proxy"));
        assert!(user_data.contains(SHARED_VM_GUEST_AGENT_SERVICE_NAME));
        assert!(user_data.contains("systemctl enable --now ctx-avf-linux-guest-agent.service"));
        assert!(user_data.contains("StandardOutput=journal+console"));
        assert!(user_data.contains("starting guest-agent"));
        assert!(user_data.contains("preparing ctx-avf-linux-guest-agent.service"));
        assert!(user_data.contains("systemctl status ctx-avf-linux-guest-agent.service --no-pager"));
        let content_lines = user_data
            .lines()
            .skip_while(|line| *line != "    content: |")
            .skip(1)
            .take_while(|line| !line.starts_with("  - path: "))
            .collect::<Vec<_>>();
        assert!(!content_lines.is_empty());
        assert!(content_lines.iter().all(|line| line.starts_with("      ")));
    }

    #[test]
    fn cloud_init_meta_data_changes_when_guest_payload_changes() {
        let first =
            render_shared_vm_cloud_init_meta_data(b"guest-agent-a", Some(b"egress-proxy-a"));
        let second =
            render_shared_vm_cloud_init_meta_data(b"guest-agent-b", Some(b"egress-proxy-b"));
        assert!(first.contains("instance-id: ctx-avf-linux-"));
        assert_ne!(first, second);
    }

    #[cfg(unix)]
    #[test]
    fn guest_exec_relays_request_over_shared_vm_control_socket() {
        use std::os::unix::net::UnixListener;
        use std::thread;

        let temp = PathBuf::from("/tmp").join(format!(
            "ctxavf-{}-{}",
            std::process::id(),
            now_timestamp_string()
        ));
        if temp.exists() {
            fs::remove_dir_all(&temp).expect("clear tempdir");
        }
        fs::create_dir_all(&temp).expect("create tempdir");
        let runtime_root = temp.join("runtime");
        fs::create_dir_all(&runtime_root).expect("runtime dir");
        let rootfs = runtime_root.join("rootfs.img");
        let kernel = runtime_root.join("kernel");
        let initrd = runtime_root.join("initrd");
        fs::write(&rootfs, b"rootfs").expect("rootfs");
        fs::write(&kernel, b"kernel").expect("kernel");
        fs::write(&initrd, b"initrd").expect("initrd");
        start_shared_vm(
            &temp,
            &runtime_root,
            &rootfs,
            &kernel,
            &initrd,
            "test".into(),
        )
        .expect("start shared vm");

        let metadata_path = shared_vm_worktree_metadata_path(&temp, "ws-123", "wt-456");
        persist_guest_worktree_state(
            &metadata_path,
            &PersistedGuestWorktreeState {
                workspace_id: "ws-123".to_string(),
                worktree_id: "wt-456".to_string(),
                host_workspace_root: temp.join("repo"),
                guest_root: PathBuf::from("/ctx/ws/worktrees/wt-456"),
                host_shadow_root: temp.join("shadow-root"),
                guest_user: "ctx-ws-test".to_string(),
                base_commit_sha: "abc123".to_string(),
                branch_name: "ctx/ws-123/wt-456".to_string(),
                updated_at: now_timestamp_string(),
                simulated: true,
                notes: vec![],
            },
        )
        .expect("persist guest worktree state");

        let socket_path = shared_vm_control_socket_path(&temp);
        if let Some(parent) = socket_path.parent() {
            fs::create_dir_all(parent).expect("socket dir");
        }
        if socket_path.exists() {
            fs::remove_file(&socket_path).expect("remove stale socket");
        }
        let listener = UnixListener::bind(&socket_path).expect("bind control socket");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept control socket");
            let request = read_exec_frame(&mut stream)
                .expect("read request")
                .expect("request frame");
            let request = match request {
                AvfLinuxExecFrame::Request(request) => request,
                other => panic!("expected request frame, got {other:?}"),
            };
            assert_eq!(request.command, "/usr/bin/env");
            assert_eq!(request.args, vec!["--version".to_string()]);
            assert_eq!(request.cwd, "/ctx/ws/worktrees/wt-456/src");
            assert_eq!(request.user.as_deref(), Some("ctxagent"));
            assert!(request.pty);
            assert_eq!(
                request.env.get("TERM").map(String::as_str),
                Some("xterm-256color")
            );
            write_exec_frame(
                &mut stream,
                &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 7 }),
            )
            .expect("write exit frame");
        });

        let exit_code = guest_exec(
            &temp,
            "ws-123",
            "wt-456",
            Path::new("/ctx/ws/worktrees/wt-456/src"),
            "/usr/bin/env",
            &["TERM=xterm-256color".to_string()],
            Some("ctxagent"),
            true,
            &["--version".to_string()],
        )
        .expect("guest exec should succeed");

        assert_eq!(exit_code, 7);
        server.join().expect("server thread");
        fs::remove_dir_all(&temp).expect("cleanup tempdir");
    }

    #[cfg(unix)]
    #[test]
    fn shared_vm_control_connection_proxies_to_guest_agent() {
        use std::os::unix::net::UnixListener;
        use std::thread;

        let temp = PathBuf::from("/tmp").join(format!(
            "ctxavf-proxy-{}-{}",
            std::process::id(),
            now_timestamp_string()
        ));
        if temp.exists() {
            fs::remove_dir_all(&temp).expect("clear tempdir");
        }
        fs::create_dir_all(&temp).expect("create tempdir");

        let metadata_path = shared_vm_worktree_metadata_path(&temp, "ws-123", "wt-456");
        let host_shadow_root = temp.join("shadow-root");
        fs::create_dir_all(host_shadow_root.join("src")).expect("shadow root");
        persist_guest_worktree_state(
            &metadata_path,
            &PersistedGuestWorktreeState {
                workspace_id: "ws-123".to_string(),
                worktree_id: "wt-456".to_string(),
                host_workspace_root: temp.join("repo"),
                guest_root: PathBuf::from("/ctx/ws/worktrees/wt-456"),
                host_shadow_root: host_shadow_root.clone(),
                guest_user: "ctx-ws-test".to_string(),
                base_commit_sha: "abc123".to_string(),
                branch_name: "ctx/ws-123/wt-456".to_string(),
                updated_at: now_timestamp_string(),
                simulated: true,
                notes: vec![],
            },
        )
        .expect("persist guest worktree state");

        let agent_socket = shared_vm_guest_agent_socket_path(&temp);
        if let Some(parent) = agent_socket.parent() {
            fs::create_dir_all(parent).expect("socket dir");
        }
        if agent_socket.exists() {
            fs::remove_file(&agent_socket).expect("remove stale guest-agent socket");
        }
        let listener = UnixListener::bind(&agent_socket).expect("bind guest-agent socket");
        let agent = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept guest-agent connection");
            let request = read_exec_frame(&mut stream)
                .expect("read proxied request")
                .expect("request frame");
            let request = match request {
                AvfLinuxExecFrame::Request(request) => request,
                other => panic!("expected proxied request frame, got {other:?}"),
            };
            assert_eq!(request.command, "/usr/bin/env");
            assert_eq!(
                request.cwd,
                host_shadow_root.join("src").display().to_string()
            );
            assert_eq!(request.args, vec!["--version".to_string()]);
            assert_eq!(
                request.env.get("TERM").map(String::as_str),
                Some("xterm-256color")
            );
            write_exec_frame(
                &mut stream,
                &AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 13 }),
            )
            .expect("write exit frame");
        });

        let (mut client, server) = UnixStream::pair().expect("unix stream pair");
        let temp_for_server = temp.clone();
        let relay = thread::spawn(move || {
            handle_shared_vm_control_connection(&temp_for_server, server)
                .expect("proxy shared vm control connection");
        });

        write_exec_frame(
            &mut client,
            &AvfLinuxExecFrame::Request(AvfLinuxExecRequest::new(
                "/usr/bin/env",
                vec!["--version".to_string()],
                "/ctx/ws/worktrees/wt-456/src",
                Some("ctxagent".to_string()),
                HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]),
                false,
            )),
        )
        .expect("write client request");

        let frame = read_exec_frame(&mut client)
            .expect("read proxied exit")
            .expect("exit frame");
        assert_eq!(
            frame,
            AvfLinuxExecFrame::Exit(AvfLinuxExecExit { exit_code: 13 })
        );

        relay.join().expect("relay thread");
        agent.join().expect("agent thread");
        fs::remove_dir_all(&temp).expect("cleanup tempdir");
    }
}
