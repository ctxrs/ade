mod avf_linux_exec_protocol;
#[path = "avf_linux_helper/cloud_init.rs"]
mod cloud_init;
#[path = "avf_linux_helper/control.rs"]
mod control;
#[path = "avf_linux_helper/guest_exec.rs"]
mod guest_exec;
#[path = "avf_linux_helper/guest_worktree.rs"]
mod guest_worktree;
#[path = "avf_linux_helper/paths.rs"]
mod paths;
#[path = "avf_linux_helper/probe.rs"]
mod probe;
#[path = "avf_linux_helper/real_vm_runtime.rs"]
mod real_vm_runtime;
#[path = "avf_linux_helper/runtime_artifacts.rs"]
mod runtime_artifacts;
#[path = "avf_linux_helper/shared_vm_lifecycle.rs"]
mod shared_vm_lifecycle;
#[path = "avf_linux_helper/simulated_exec.rs"]
mod simulated_exec;
#[path = "avf_linux_helper/state.rs"]
mod state;
#[cfg(test)]
#[path = "avf_linux_helper/tests.rs"]
mod tests;
#[path = "avf_linux_helper/virtualization.rs"]
mod virtualization;

use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
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
    VZDirectorySharingDeviceConfiguration, VZDiskImageCachingMode,
    VZDiskImageStorageDeviceAttachment, VZDiskImageSynchronizationMode, VZFileSerialPortAttachment,
    VZGenericMachineIdentifier, VZGenericPlatformConfiguration, VZLinuxBootLoader, VZMACAddress,
    VZMemoryBalloonDeviceConfiguration, VZNATNetworkDeviceAttachment, VZNetworkDeviceConfiguration,
    VZSerialPortConfiguration, VZSharedDirectory, VZSingleDirectoryShare,
    VZSocketDeviceConfiguration, VZStorageDeviceConfiguration, VZVirtioBlockDeviceConfiguration,
    VZVirtioConsoleDeviceSerialPortConfiguration, VZVirtioFileSystemDeviceConfiguration,
    VZVirtioNetworkDeviceConfiguration, VZVirtioSocketDeviceConfiguration,
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
use self::cloud_init::*;
use self::control::*;
use self::guest_exec::*;
use self::guest_worktree::*;
use self::paths::*;
use self::probe::*;
use self::real_vm_runtime::*;
use self::runtime_artifacts::*;
use self::shared_vm_lifecycle::*;
use self::simulated_exec::*;
use self::state::*;
use self::virtualization::*;

const HELPER_PROTOCOL_VERSION: u32 = 1;
const HELPER_PROTOCOL_SCHEMA: &str = "ctx.avf_linux_helper.v1";
const SHARED_VM_STATE_FILE: &str = "shared-vm-state.json";
const SHARED_VM_LOG_FILE: &str = "shared-vm.log";
const SHARED_VM_CONTROL_SOCKET_FILE: &str = "shared-vm-control.sock";
const SHARED_VM_GUEST_AGENT_SOCKET_FILE: &str = "shared-vm-guest-agent.sock";
const SHARED_VM_KERNEL_CMDLINE_FILE: &str = "kernel-cmdline";
const SHARED_VM_SAVED_STATE_FILE: &str = "saved-machine-state.vzvmsave";
const SHARED_VM_SHUTDOWN_REQUEST_FILE: &str = "shutdown-request";
const SHARED_VM_MEMORY_PRESSURE_REQUEST_FILE: &str = "memory-pressure-request";
const SHARED_VM_START_LOCK_FILE: &str = "start.lock";
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
const AVF_LINUX_CONTAINER_STACK_FILE: &str = "container-stack.tar.gz";
const SHARED_VM_CLOUD_INIT_DIR: &str = "cloud-init";
const SHARED_VM_CLOUD_INIT_META_DATA_FILE: &str = "meta-data";
const SHARED_VM_CLOUD_INIT_USER_DATA_FILE: &str = "user-data";
const SHARED_VM_CLOUD_INIT_NETWORK_CONFIG_FILE: &str = "network-config";
const SHARED_VM_CLOUD_INIT_IMAGE_FILE: &str = "cidata.img";
const SHARED_VM_GUEST_AGENT_SERVICE_NAME: &str = "ctx-avf-linux-guest-agent.service";
const SHARED_VM_ROOTFS_LABEL: &str = "cloudimg-rootfs";
const SHARED_VM_BOOT_DIR: &str = "boot";
const SHARED_VM_BOOT_KERNEL_FILE: &str = "kernel";
const SHARED_VM_DISK_DIR: &str = "disk";
const SHARED_VM_ROOTFS_FILE: &str = "rootfs.raw";
const SHARED_VM_DATA_DISK_FILE: &str = "data.raw";
const SHARED_VM_MACHINE_IDENTIFIER_FILE: &str = "machine-identifier.bin";
const SHARED_VM_MAC_ADDRESS_FILE: &str = "mac-address.txt";
const SHARED_VM_GUEST_CONSOLE_LOG_FILE: &str = "guest-console.log";
const SHARED_VM_GUEST_CONTROL_READY_FILE: &str = "guest-control-ready";
const SHARED_VM_DATA_ROOT_SHARE_TAG: &str = "ctx-data-root";
const SHARED_VM_HOST_DATA_SERVICE_NAME: &str = "ctx-avf-host-data.service";
const SHARED_VM_DATA_DISK_LABEL: &str = "ctx-avf-data";
const SHARED_VM_DATA_DISK_SERVICE_NAME: &str = "ctx-avf-data-disk.service";
const SHARED_VM_DATA_DISK_INSTALL_PATH: &str = "/usr/local/lib/ctx/ctx-avf-data-disk.sh";
const SHARED_VM_CONTAINERD_SERVICE_NAME: &str = "containerd.service";
const SHARED_VM_BUILDKIT_SERVICE_NAME: &str = "buildkit.service";
const SHARED_VM_PAYLOADS_DIR: &str = "payloads";
const SHARED_VM_GUEST_CONTAINER_STACK_INSTALL_PATH: &str =
    "/usr/local/lib/ctx/ctx-avf-install-container-stack.sh";
const SHARED_VM_GUEST_CONTAINER_STACK_MARKER_PATH: &str =
    "/usr/local/lib/ctx/container-stack.sha256";
const SHARED_VM_GUEST_NERDCTL_BIN: &str = "/usr/local/bin/nerdctl";
const SHARED_VM_GUEST_BUILDKITCTL_BIN: &str = "/usr/local/bin/buildctl";
const SHARED_VM_GUEST_BUILDKIT_SOCKET: &str = "unix:///run/buildkit/buildkitd.sock";
const SHARED_VM_CPU_COUNT_ENV: &str = "CTX_AVF_LINUX_CPU_COUNT";
const SHARED_VM_MEMORY_CEILING_BYTES_ENV: &str = "CTX_AVF_LINUX_MEMORY_CEILING_BYTES";
const SHARED_VM_HOST_MEMORY_RESERVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const SHARED_VM_MIN_DEFAULT_MEMORY_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const SHARED_VM_INITIAL_DATA_DISK_BYTES: u64 = 12 * 1024 * 1024 * 1024;
const SHARED_VM_HOST_DISK_RESERVE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const SHARED_VM_DATA_DISK_GROWTH_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SHARED_VM_DATA_DISK_CRITICAL_FREE_BYTES: u64 = 512 * 1024 * 1024;
const SHARED_VM_DATA_DISK_GROWTH_STEP_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const SHARED_VM_MEMORY_BALLOON_STEP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SHARED_VM_GUEST_MEMORY_GROW_THRESHOLD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const SHARED_VM_HOST_MEMORY_EMERGENCY_BYTES: u64 = 1024 * 1024 * 1024;
const SHARED_VM_MEMORY_WATCHDOG_CONFIRMATION_POLLS: u32 = 2;
#[cfg(target_os = "macos")]
const SHARED_VM_MEMORY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(target_os = "macos")]
const SHARED_VM_MEMORY_WATCHDOG_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(2);
const SHARED_VM_MEMORY_WATCHDOG_EXIT_GRACE: Duration = Duration::from_secs(8);
const SHARED_VM_START_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SHARED_VM_READINESS_PHASE_TIMEOUT_SECONDS: u64 = 10;
const SHARED_VM_READINESS_GUEST_EXEC_IO_TIMEOUT: Duration =
    Duration::from_secs(SHARED_VM_READINESS_PHASE_TIMEOUT_SECONDS + 5);
#[cfg(target_os = "macos")]
const SHARED_VM_GUEST_CONTROL_VSOCK_PORT: u32 = 47001;
#[cfg(target_os = "macos")]
const SHARED_VM_CONTROL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);
#[cfg(target_os = "macos")]
const SHARED_VM_DATA_DISK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const GUEST_EXEC_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
const GUEST_EXEC_CONNECT_RETRY_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(100);
// Real shared-VM guest control transport still truncates streamed stdin around the
// 4 KiB-class frame budget during live `tar -xf -` imports, so keep payload chunks
// well below that empirical limit on both sides of the relay.
const AVF_EXEC_STREAM_FRAME_MAX_PAYLOAD: usize = 1024;
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
    runtime_shape_digest: Option<String>,
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
    runtime_shape_digest: Option<String>,
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
        Some("watch-shared-vm-memory") | Some("watch-workspace-vm-memory") => {
            let data_root = required_path_arg(args.next(), "data_root")?;
            let owner_pid = required_u32_arg(args.next(), "owner_pid")?;
            ensure_no_extra_args(args)?;
            run_shared_vm_memory_watchdog(&data_root, owner_pid)
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
        Some("shared-vm-exec") => {
            let mut data_root = None;
            let mut cwd = None;
            let mut command = None;
            let mut guest_env = Vec::new();
            let mut user = None;
            let mut pty = false;
            let mut passthrough_args = Vec::new();
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--data-root" => data_root = args.next().map(PathBuf::from),
                    "--cwd" => cwd = args.next().map(PathBuf::from),
                    "--command" => command = args.next(),
                    "--env" => guest_env.push(required_string_arg(args.next(), "env")?),
                    "--user" => user = args.next(),
                    "--pty" => pty = true,
                    "--" => {
                        passthrough_args.extend(args);
                        break;
                    }
                    other => bail!("unknown shared-vm-exec argument `{other}`"),
                }
            }
            let exit_code = shared_vm_exec(
                &required_path_arg(
                    data_root.map(|path| path.to_string_lossy().to_string()),
                    "data_root",
                )?,
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
            "usage: ctx-avf-linux-helper <probe|prepare-runtime-layout|workspace-vm-state|start-workspace-vm|stop-workspace-vm|prepare-guest-worktree|serve-workspace-vm|serve-guest-agent|run-workspace-vm|watch-workspace-vm-memory|guest-exec|shared-vm-exec> ..."
        ),
    }
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

fn required_u32_arg(value: Option<String>, name: &str) -> Result<u32> {
    let raw = required_string_arg(value, name)?;
    raw.parse::<u32>()
        .with_context(|| format!("parsing argument `{name}` as a u32"))
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
