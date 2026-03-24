use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Mutex as StdMutex;
use std::sync::OnceLock;

use anyhow::{bail, Context, Result};
use ctx_core::ids::{WorkspaceId, WorktreeId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::sync::Mutex;

use super::machine::archive::{extract_archive_to_dir, resolve_single_extracted_root};
use super::machine::downloads::{
    acquire_managed_artifact_file_lock, download_managed_artifact,
    finalize_managed_artifact_download, managed_artifact_lock_path, managed_artifact_partial_path,
    ManagedArtifactDownloadReporter, ManagedDownloadAggregate,
};
use super::{
    observe_log, observe_phase, ContainerExecutionSettings, HarnessSetupLogLevel,
    HarnessSetupObserver, HarnessSetupPhase,
};
use crate::bundled_assets;
use crate::updates;

pub(crate) const AVF_LINUX_HELPER_PATH_ENV: &str = "CTX_AVF_LINUX_HELPER_PATH";
const AVF_LINUX_GUEST_RUNTIME_ID: &str = "avf-linux-guest";
const AVF_LINUX_RUNTIME_READY_MARKER: &str = ".ctx-managed-ready";
const AVF_LINUX_ROOTFS_LABEL: &str = "Ubuntu guest runtime";
const AVF_LINUX_KERNEL_HELPER: &str = "kernel";
const AVF_LINUX_INITRD_HELPER: &str = "initrd";
const AVF_LINUX_GUEST_AGENT_HELPER: &str = "guest-agent";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AvfLinuxHelperProbe {
    pub protocol_version: u32,
    pub protocol_schema: String,
    pub helper_version: String,
    pub host_os: String,
    pub host_arch: String,
    pub supported: bool,
    pub save_restore_supported: bool,
    pub rosetta_supported: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvfLinuxSharedVmLifecycleState {
    Missing,
    Stopped,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvfLinuxRuntimeLayoutStatus {
    Prepared,
    AlreadyPresent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvfLinuxSharedVmTransitionStatus {
    Scaffolded,
    Stopped,
    AlreadyStopped,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AvfLinuxRuntimeLayout {
    pub protocol_version: u32,
    pub protocol_schema: String,
    pub vm_root: PathBuf,
    pub logs_root: PathBuf,
    pub state_path: PathBuf,
    pub layout_status: AvfLinuxRuntimeLayoutStatus,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AvfLinuxSharedVmState {
    pub protocol_version: u32,
    pub protocol_schema: String,
    pub state: AvfLinuxSharedVmLifecycleState,
    pub vm_root: PathBuf,
    pub logs_root: PathBuf,
    pub state_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootfs_image: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initrd_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_stopped_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_status: Option<AvfLinuxSharedVmTransitionStatus>,
    #[serde(default)]
    pub simulated: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AvfLinuxGuestWorktreeStatus {
    Prepared,
    AlreadyPresent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AvfLinuxGuestWorktree {
    pub protocol_version: u32,
    pub protocol_schema: String,
    pub workspace_id: String,
    pub worktree_id: String,
    pub guest_root: PathBuf,
    pub host_shadow_root: PathBuf,
    pub metadata_path: PathBuf,
    pub status: AvfLinuxGuestWorktreeStatus,
    #[serde(default)]
    pub simulated: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct AvfLinuxGuestRuntime {
    pub runtime_root: PathBuf,
    pub rootfs_image: PathBuf,
    pub kernel_path: PathBuf,
    pub initrd_path: PathBuf,
    pub guest_agent_path: Option<PathBuf>,
    pub version: String,
    pub managed: bool,
}

impl AvfLinuxGuestRuntime {
    fn from_source(
        data_root: &Path,
        source: &bundled_assets::ManagedRuntimeSource,
    ) -> Result<Self> {
        let runtime_root = managed_avf_linux_runtime_root(data_root, source);
        let rootfs_image = runtime_root.join(source.bin.trim());
        let kernel_path = managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_KERNEL_HELPER)
            .ok_or_else(|| {
                anyhow::anyhow!("managed AVF Linux runtime is missing a kernel helper")
            })?;
        let initrd_path = managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_INITRD_HELPER)
            .ok_or_else(|| {
                anyhow::anyhow!("managed AVF Linux runtime is missing an initrd helper")
            })?;
        let guest_agent_path =
            managed_avf_linux_helper_path(&runtime_root, AVF_LINUX_GUEST_AGENT_HELPER);
        Ok(Self {
            runtime_root,
            rootfs_image,
            kernel_path,
            initrd_path,
            guest_agent_path,
            version: source.version.trim().to_string(),
            managed: true,
        })
    }

    fn from_bundled(paths: bundled_assets::BundledRuntimePaths) -> Self {
        Self {
            kernel_path: paths.root.join("helpers").join(AVF_LINUX_KERNEL_HELPER),
            initrd_path: paths.root.join("helpers").join(AVF_LINUX_INITRD_HELPER),
            guest_agent_path: {
                let path = paths
                    .root
                    .join("helpers")
                    .join(AVF_LINUX_GUEST_AGENT_HELPER);
                path.exists().then_some(path)
            },
            runtime_root: paths.root,
            rootfs_image: paths.bin,
            version: paths.version,
            managed: false,
        }
    }
}

pub(crate) fn runtime_target_label() -> String {
    if let Some(runtime) = bundled_avf_linux_guest_runtime() {
        return format!("{AVF_LINUX_GUEST_RUNTIME_ID}:bundled:{}", runtime.version);
    }
    managed_avf_linux_guest_source()
        .map(|source| format!("{AVF_LINUX_GUEST_RUNTIME_ID}:{}", source.version.trim()))
        .unwrap_or_else(|| AVF_LINUX_GUEST_RUNTIME_ID.to_string())
}

pub(crate) fn helper_path() -> Result<PathBuf> {
    if !cfg!(target_os = "macos") {
        bail!("AVF Linux VM runtime is only supported on macOS");
    }
    let value = std::env::var(AVF_LINUX_HELPER_PATH_ENV)
        .context("CTX_AVF_LINUX_HELPER_PATH is required for the AVF Linux VM backend")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("CTX_AVF_LINUX_HELPER_PATH is empty");
    }
    let path = PathBuf::from(trimmed);
    if !path.is_file() {
        bail!("AVF Linux helper does not exist at {}", path.display());
    }
    Ok(path)
}

fn invoke_helper_json<T>(args: &[&str]) -> Result<T>
where
    T: DeserializeOwned,
{
    let helper = helper_path()?;
    let output = std::process::Command::new(&helper)
        .args(args)
        .output()
        .with_context(|| format!("spawning AVF Linux helper at {}", helper.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            bail!("AVF Linux helper exited with status {}", output.status);
        }
        bail!("AVF Linux helper failed: {combined}");
    }
    serde_json::from_slice(&output.stdout).with_context(|| {
        let joined = args.join(" ");
        format!("parsing AVF Linux helper response for `{joined}`")
    })
}

pub(crate) fn probe_helper() -> Result<AvfLinuxHelperProbe> {
    invoke_helper_json(&["probe"])
}

pub(crate) fn prepare_runtime_layout(data_root: &Path) -> Result<AvfLinuxRuntimeLayout> {
    invoke_helper_json(&["prepare-runtime-layout", &data_root.to_string_lossy()])
}

pub(crate) fn shared_vm_state(data_root: &Path) -> Result<AvfLinuxSharedVmState> {
    invoke_helper_json(&["shared-vm-state", &data_root.to_string_lossy()])
}

pub(crate) fn prepare_guest_worktree(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
) -> Result<AvfLinuxGuestWorktree> {
    invoke_helper_json(&[
        "prepare-guest-worktree",
        &data_root.to_string_lossy(),
        &workspace_id.0.to_string(),
        &worktree_id.0.to_string(),
        &host_workspace_root.to_string_lossy(),
        base_commit_sha,
        branch_name,
    ])
}

pub(crate) fn build_guest_exec_command(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    cwd: &Path,
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    user: Option<&str>,
    pty: bool,
) -> Result<tokio::process::Command> {
    let helper = helper_path()?;
    let mut child = tokio::process::Command::new(&helper);
    child
        .arg("guest-exec")
        .arg("--data-root")
        .arg(data_root)
        .arg("--workspace-id")
        .arg(workspace_id.0.to_string())
        .arg("--worktree-id")
        .arg(worktree_id.0.to_string())
        .arg("--cwd")
        .arg(cwd)
        .arg("--command")
        .arg(command);
    if let Some(user) = user.map(str::trim).filter(|value| !value.is_empty()) {
        child.arg("--user").arg(user);
    }
    if pty {
        child.arg("--pty");
    }
    let mut env_pairs = env.iter().collect::<Vec<_>>();
    env_pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in env_pairs {
        child.arg("--env").arg(format!("{key}={value}"));
    }
    child.arg("--");
    child.args(args);
    Ok(child)
}

pub(crate) async fn run_guest_exec_capture(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    cwd: &Path,
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    user: Option<&str>,
    pty: bool,
) -> Result<std::process::Output> {
    let mut child = build_guest_exec_command(
        data_root,
        workspace_id,
        worktree_id,
        cwd,
        command,
        args,
        env,
        user,
        pty,
    )?;
    child.stdin(std::process::Stdio::null());
    child.output().await.with_context(|| {
        format!(
            "running AVF guest exec `{command}` for workspace {} worktree {}",
            workspace_id.0, worktree_id.0
        )
    })
}

#[allow(dead_code)]
pub(crate) fn start_shared_vm(
    data_root: &Path,
    runtime: &AvfLinuxGuestRuntime,
) -> Result<AvfLinuxSharedVmState> {
    invoke_helper_json(&[
        "start-shared-vm",
        &data_root.to_string_lossy(),
        &runtime.runtime_root.to_string_lossy(),
        &runtime.rootfs_image.to_string_lossy(),
        &runtime.kernel_path.to_string_lossy(),
        &runtime.initrd_path.to_string_lossy(),
        &runtime.version,
    ])
}

#[allow(dead_code)]
pub(crate) fn stop_shared_vm(data_root: &Path) -> Result<AvfLinuxSharedVmState> {
    invoke_helper_json(&["stop-shared-vm", &data_root.to_string_lossy()])
}

pub(crate) fn runtime_available() -> bool {
    probe_helper().map(|probe| probe.supported).unwrap_or(false)
}

pub(crate) fn runtime_state(data_root: &Path) -> Result<(bool, bool)> {
    let helper_ready = probe_helper().map(|probe| probe.supported).unwrap_or(false);
    let runtime_ready = if let Some(runtime) = bundled_avf_linux_guest_runtime() {
        avf_linux_runtime_is_ready(&runtime)
    } else if let Some(source) = managed_avf_linux_guest_source() {
        let runtime = AvfLinuxGuestRuntime::from_source(data_root, &source)?;
        avf_linux_runtime_is_ready(&runtime)
    } else {
        false
    };
    if !helper_ready || !runtime_ready {
        return Ok((false, runtime_ready));
    }
    let state = shared_vm_state(data_root)?;
    let machine_ready = matches!(state.state, AvfLinuxSharedVmLifecycleState::Running);
    Ok((machine_ready, runtime_ready))
}

pub(crate) async fn ensure_shared_vm_ready_with_observer(
    data_root: &Path,
    _settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<AvfLinuxSharedVmState> {
    observe_phase(
        observer,
        HarnessSetupPhase::MachineCheck,
        "checking AVF Linux helper availability",
    );
    let probe = probe_helper()?;
    if !probe.supported {
        bail!("AVF Linux helper reported that this host is unsupported");
    }
    observe_log(
        observer,
        HarnessSetupPhase::MachineCheck,
        HarnessSetupLogLevel::Info,
        &format!(
            "using AVF Linux helper {} on {}/{}",
            probe.helper_version, probe.host_os, probe.host_arch
        ),
    );
    for note in &probe.notes {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            note,
        );
    }

    let runtime = ensure_managed_avf_linux_guest_runtime(data_root, observer, None).await?;
    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!(
            "AVF Linux guest runtime {} is ready (rootfs={}, kernel={}, initrd={}, guest_agent={})",
            runtime.version,
            runtime.rootfs_image.display(),
            runtime.kernel_path.display(),
            runtime.initrd_path.display(),
            runtime
                .guest_agent_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
    );

    let layout = prepare_runtime_layout(data_root)?;
    let state = shared_vm_state(data_root)?;
    observe_log(
        observer,
        HarnessSetupPhase::MachineCheck,
        HarnessSetupLogLevel::Info,
        &format!(
            "AVF Linux shared VM layout is ready at {} (logs={}, state={:?})",
            layout.vm_root.display(),
            layout.logs_root.display(),
            state.state
        ),
    );
    if matches!(state.state, AvfLinuxSharedVmLifecycleState::Running) {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            "AVF Linux shared VM is already running",
        );
        return Ok(state);
    }

    observe_phase(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        "starting AVF Linux shared VM",
    );
    let started = start_shared_vm(data_root, &runtime)?;
    observe_log(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        HarnessSetupLogLevel::Info,
        &format!(
            "AVF Linux shared VM start completed with state {:?} (simulated={})",
            started.state, started.simulated
        ),
    );
    for note in &started.notes {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            note,
        );
    }
    if !matches!(started.state, AvfLinuxSharedVmLifecycleState::Running) {
        bail!(
            "AVF Linux shared VM did not reach a running state after start (state={:?})",
            started.state
        );
    }
    Ok(started)
}

pub(crate) async fn ensure_guest_worktree_from_host_copy(
    data_root: &Path,
    workspace_id: WorkspaceId,
    worktree_id: WorktreeId,
    host_workspace_root: &Path,
    base_commit_sha: &str,
    branch_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    observe_phase(
        observer,
        HarnessSetupPhase::ContainerStartOrCreate,
        "preparing AVF Linux guest worktree",
    );
    let prepared = prepare_guest_worktree(
        data_root,
        workspace_id,
        worktree_id,
        host_workspace_root,
        base_commit_sha,
        branch_name,
    )?;
    observe_log(
        observer,
        HarnessSetupPhase::ContainerStartOrCreate,
        HarnessSetupLogLevel::Info,
        &format!(
            "AVF Linux guest worktree {} is {} at {} (shadow={})",
            worktree_id.0,
            match prepared.status {
                AvfLinuxGuestWorktreeStatus::Prepared => "prepared",
                AvfLinuxGuestWorktreeStatus::AlreadyPresent => "already present",
            },
            prepared.guest_root.display(),
            prepared.host_shadow_root.display()
        ),
    );
    for note in &prepared.notes {
        observe_log(
            observer,
            HarnessSetupPhase::ContainerStartOrCreate,
            HarnessSetupLogLevel::Info,
            note,
        );
    }
    Ok(prepared.guest_root)
}

#[allow(dead_code)]
pub(crate) async fn prefetch_runtime_with_observer(
    data_root: &Path,
    _settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    observe_phase(
        observer,
        HarnessSetupPhase::MachineCheck,
        "checking AVF Linux helper availability",
    );
    let probe = probe_helper()?;
    if !probe.supported {
        bail!("AVF Linux helper reported that this host is unsupported");
    }
    observe_log(
        observer,
        HarnessSetupPhase::MachineCheck,
        HarnessSetupLogLevel::Info,
        &format!(
            "using AVF Linux helper {} on {}/{}",
            probe.helper_version, probe.host_os, probe.host_arch
        ),
    );
    for note in &probe.notes {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            note,
        );
    }
    let runtime = ensure_managed_avf_linux_guest_runtime(data_root, observer, None).await?;
    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!(
            "AVF Linux guest runtime {} is ready (rootfs={}, kernel={}, initrd={}, guest_agent={})",
            runtime.version,
            runtime.rootfs_image.display(),
            runtime.kernel_path.display(),
            runtime.initrd_path.display(),
            runtime
                .guest_agent_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "none".to_string())
        ),
    );
    let layout = prepare_runtime_layout(data_root)?;
    observe_log(
        observer,
        HarnessSetupPhase::MachineCheck,
        HarnessSetupLogLevel::Info,
        &format!(
            "AVF Linux shared VM layout is ready at {} (logs={}, state={:?})",
            layout.vm_root.display(),
            layout.logs_root.display(),
            shared_vm_state(data_root)?.state
        ),
    );
    Ok(())
}

fn managed_avf_linux_guest_source() -> Option<bundled_assets::ManagedRuntimeSource> {
    #[cfg(test)]
    if let Some(source) = test_runtime_source_override()
        .lock()
        .expect("AVF Linux runtime override mutex poisoned")
        .clone()
    {
        return Some(source);
    }
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    bundled_assets::managed_runtime_source(AVF_LINUX_GUEST_RUNTIME_ID, os, arch)
}

fn managed_artifact_extension(uri: &str) -> &'static str {
    let path = url::Url::parse(uri)
        .ok()
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_else(|| uri.to_string());
    let path_lc = path.to_ascii_lowercase();
    if path_lc.ends_with(".tar.gz") {
        "tar.gz"
    } else if path_lc.ends_with(".tgz") {
        "tgz"
    } else if path_lc.ends_with(".tar") {
        "tar"
    } else {
        "zip"
    }
}

fn managed_avf_linux_archive_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    let ext = managed_artifact_extension(&source.uri);
    data_root
        .join("managed")
        .join("downloads")
        .join(AVF_LINUX_GUEST_RUNTIME_ID)
        .join(os)
        .join(arch)
        .join(format!(
            "sha256-{}.{}",
            source.sha256.trim().to_ascii_lowercase(),
            ext
        ))
}

fn managed_avf_linux_runtime_root(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = (std::env::consts::OS, std::env::consts::ARCH);
    data_root
        .join("managed")
        .join("runtimes")
        .join(AVF_LINUX_GUEST_RUNTIME_ID)
        .join(os)
        .join(arch)
        .join(format!(
            "{AVF_LINUX_GUEST_RUNTIME_ID}-{}",
            source.version.trim()
        ))
}

fn managed_avf_linux_helper_path(runtime_root: &Path, helper_name: &str) -> Option<PathBuf> {
    let helper = helper_name.trim();
    if helper.is_empty() {
        return None;
    }
    Some(runtime_root.join("helpers").join(helper))
}

fn managed_avf_linux_runtime_ready_marker_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join(AVF_LINUX_RUNTIME_READY_MARKER)
}

fn avf_linux_runtime_is_ready(runtime: &AvfLinuxGuestRuntime) -> bool {
    runtime.rootfs_image.exists()
        && runtime.kernel_path.exists()
        && runtime.initrd_path.exists()
        && (!runtime.managed
            || managed_avf_linux_runtime_ready_marker_path(&runtime.runtime_root).exists())
}

fn managed_avf_linux_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn mark_managed_avf_linux_runtime_ready(runtime_root: &Path) -> Result<()> {
    let marker = managed_avf_linux_runtime_ready_marker_path(runtime_root);
    fs::write(&marker, b"ready")
        .await
        .with_context(|| format!("writing {}", marker.display()))
}

fn bundled_avf_linux_guest_runtime() -> Option<AvfLinuxGuestRuntime> {
    let runtime = bundled_assets::bundled_avf_linux_guest_runtime()?;
    let runtime = AvfLinuxGuestRuntime::from_bundled(runtime);
    if avf_linux_runtime_is_ready(&runtime) {
        Some(runtime)
    } else {
        None
    }
}

pub(crate) async fn ensure_managed_avf_linux_guest_runtime(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<AvfLinuxGuestRuntime> {
    ensure_managed_avf_linux_guest_runtime_with_override(
        data_root,
        None,
        observer,
        download_aggregate,
    )
    .await
}

pub(crate) async fn ensure_managed_avf_linux_guest_runtime_with_override(
    data_root: &Path,
    source_override: Option<&bundled_assets::ManagedRuntimeSource>,
    observer: Option<&dyn HarnessSetupObserver>,
    download_aggregate: Option<ManagedDownloadAggregate>,
) -> Result<AvfLinuxGuestRuntime> {
    if source_override.is_none() {
        if let Some(runtime) = bundled_avf_linux_guest_runtime() {
            return Ok(runtime);
        }
    }

    let source = match source_override.cloned() {
        Some(source) => source,
        None => managed_avf_linux_guest_source().ok_or_else(|| {
            anyhow::anyhow!(
                "managed AVF Linux guest runtime source is not available for {}/{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        })?,
    };
    let runtime = AvfLinuxGuestRuntime::from_source(data_root, &source)?;
    if avf_linux_runtime_is_ready(&runtime) {
        return Ok(runtime);
    }

    let _install_guard = managed_avf_linux_install_lock().lock().await;
    let runtime = AvfLinuxGuestRuntime::from_source(data_root, &source)?;
    if avf_linux_runtime_is_ready(&runtime) {
        return Ok(runtime);
    }

    observe_log(
        observer,
        HarnessSetupPhase::ArtifactDownload,
        HarnessSetupLogLevel::Info,
        &format!(
            "installing managed AVF Linux guest runtime {}",
            runtime.version
        ),
    );

    let final_archive = managed_avf_linux_archive_path(data_root, &source);
    let archive_lock = managed_artifact_lock_path(&final_archive);
    let _archive_guard = acquire_managed_artifact_file_lock(
        &archive_lock,
        "AVF Linux guest runtime archive",
        observer,
        HarnessSetupPhase::ArtifactDownload,
    )
    .await?;
    let partial_archive = managed_artifact_partial_path(&final_archive);
    if final_archive.exists() {
        let digest = updates::sha256_hex_file(&final_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", final_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&final_archive).await;
        } else {
            let _ = fs::remove_file(&partial_archive).await;
        }
    }
    if !final_archive.exists() {
        let Some(parent) = final_archive.parent() else {
            bail!(
                "managed AVF Linux archive path has no parent: {}",
                final_archive.display()
            );
        };
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
        download_managed_artifact(
            &source.uri,
            &partial_archive,
            Some(ManagedArtifactDownloadReporter::new(
                observer,
                download_aggregate.clone(),
                HarnessSetupPhase::ArtifactDownload,
                AVF_LINUX_ROOTFS_LABEL,
            )),
        )
        .await?;
        finalize_managed_artifact_download(
            &partial_archive,
            &final_archive,
            &source.sha256,
            "managed AVF Linux guest runtime archive",
        )
        .await?;
    }

    let Some(parent) = runtime.runtime_root.parent() else {
        bail!(
            "managed AVF Linux runtime root has no parent: {}",
            runtime.runtime_root.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let staging_dir = parent.join(format!(
        ".avf-linux-staging-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if staging_dir.exists() {
        let _ = fs::remove_dir_all(&staging_dir).await;
    }
    fs::create_dir_all(&staging_dir)
        .await
        .with_context(|| format!("creating {}", staging_dir.display()))?;
    let extract_dir = staging_dir.join("extract");
    fs::create_dir_all(&extract_dir)
        .await
        .with_context(|| format!("creating {}", extract_dir.display()))?;
    let archive_for_extract = final_archive.clone();
    let uri_for_extract = source.uri.clone();
    let extract_dir_for_extract = extract_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_archive_to_dir(
            &archive_for_extract,
            &uri_for_extract,
            &extract_dir_for_extract,
        )
    })
    .await
    .context("joining managed AVF Linux extract task")??;
    let extracted_root = tokio::task::spawn_blocking({
        let extract_dir = extract_dir.clone();
        move || resolve_single_extracted_root(&extract_dir)
    })
    .await
    .context("joining managed AVF Linux extraction root task")??;

    if runtime.runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime.runtime_root).await;
    }
    fs::rename(&extracted_root, &runtime.runtime_root)
        .await
        .with_context(|| {
            format!(
                "moving extracted AVF Linux runtime into place: {} -> {}",
                extracted_root.display(),
                runtime.runtime_root.display()
            )
        })?;
    let _ = fs::remove_dir_all(&staging_dir).await;

    let mut helper_downloads = Vec::new();
    for (helper_name, label) in [
        (AVF_LINUX_KERNEL_HELPER, "Linux kernel"),
        (AVF_LINUX_INITRD_HELPER, "Linux initrd"),
    ] {
        let helper_source = source.helpers.get(helper_name).cloned().ok_or_else(|| {
            anyhow::anyhow!("managed AVF Linux runtime is missing helper '{helper_name}'")
        })?;
        let helper_root = runtime.runtime_root.clone();
        let aggregate = download_aggregate.clone();
        helper_downloads.push(async move {
            let helper_path =
                managed_avf_linux_helper_path(&helper_root, helper_name).ok_or_else(|| {
                    anyhow::anyhow!("invalid AVF Linux helper path for '{helper_name}'")
                })?;
            let helper_lock = managed_artifact_lock_path(&helper_path);
            let _guard = acquire_managed_artifact_file_lock(
                &helper_lock,
                label,
                observer,
                HarnessSetupPhase::ArtifactDownload,
            )
            .await?;
            if let Some(parent) = helper_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            if helper_path.exists() {
                let digest = updates::sha256_hex_file(&helper_path)
                    .await
                    .with_context(|| format!("computing sha256 for {}", helper_path.display()))?;
                if digest.eq_ignore_ascii_case(helper_source.sha256.trim()) {
                    let _ = fs::remove_file(managed_artifact_partial_path(&helper_path)).await;
                    return Ok(()) as Result<()>;
                }
                let _ = fs::remove_file(&helper_path).await;
            }
            let tmp = managed_artifact_partial_path(&helper_path);
            download_managed_artifact(
                &helper_source.uri,
                &tmp,
                Some(ManagedArtifactDownloadReporter::new(
                    observer,
                    aggregate,
                    HarnessSetupPhase::ArtifactDownload,
                    label,
                )),
            )
            .await?;
            finalize_managed_artifact_download(&tmp, &helper_path, &helper_source.sha256, label)
                .await?;
            Ok(())
        });
    }
    for result in futures::future::join_all(helper_downloads).await {
        result?;
    }

    if !runtime.rootfs_image.exists() {
        bail!(
            "managed AVF Linux runtime installed but rootfs image is missing at {}",
            runtime.rootfs_image.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for path in [&runtime.kernel_path, &runtime.initrd_path] {
            if path.exists() {
                let mut perms = fs::metadata(path)
                    .await
                    .with_context(|| format!("metadata {}", path.display()))?
                    .permissions();
                perms.set_mode(0o644);
                fs::set_permissions(path, perms)
                    .await
                    .with_context(|| format!("chmod {}", path.display()))?;
            }
        }
    }
    mark_managed_avf_linux_runtime_ready(&runtime.runtime_root).await?;
    Ok(runtime)
}

#[cfg(test)]
fn test_runtime_source_override() -> &'static StdMutex<Option<bundled_assets::ManagedRuntimeSource>>
{
    static OVERRIDE: OnceLock<StdMutex<Option<bundled_assets::ManagedRuntimeSource>>> =
        OnceLock::new();
    OVERRIDE.get_or_init(|| StdMutex::new(None))
}

#[cfg(test)]
pub(crate) struct TestManagedAvfLinuxRuntimeSourceGuard {
    previous: Option<bundled_assets::ManagedRuntimeSource>,
}

#[cfg(test)]
impl Drop for TestManagedAvfLinuxRuntimeSourceGuard {
    fn drop(&mut self) {
        let mut guard = test_runtime_source_override()
            .lock()
            .expect("AVF Linux runtime override mutex poisoned");
        *guard = self.previous.take();
    }
}

#[cfg(test)]
pub(crate) fn override_managed_avf_linux_runtime_source_for_test(
    source: bundled_assets::ManagedRuntimeSource,
) -> TestManagedAvfLinuxRuntimeSourceGuard {
    let mut guard = test_runtime_source_override()
        .lock()
        .expect("AVF Linux runtime override mutex poisoned");
    let previous = guard.replace(source);
    TestManagedAvfLinuxRuntimeSourceGuard { previous }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.prev.take() {
                std::env::set_var(self.key, prev);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn helper_env_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::podman_env_test_lock()
    }

    fn write_probe_helper(dir: &Path) -> PathBuf {
        let path = dir.join(if cfg!(windows) {
            "ctx-avf-linux-helper-test.cmd"
        } else {
            "ctx-avf-linux-helper-test.sh"
        });
        let script = if cfg!(windows) {
            "@echo off\r\nif \"%1\"==\"probe\" (\r\n  echo {\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"macos\",\"host_arch\":\"aarch64\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}\r\n  exit /b 0\r\n)\r\n>&2 echo unexpected helper invocation: %*\r\nexit /b 1\r\n"
        } else {
            "#!/bin/sh\nif [ \"$1\" = \"probe\" ]; then\n  printf '%s\\n' '{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"macos\",\"host_arch\":\"aarch64\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}'\n  exit 0\nfi\necho \"unexpected helper invocation: $*\" >&2\nexit 1\n"
        };
        std::fs::write(&path, script).expect("write AVF Linux helper shim");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod AVF Linux helper shim");
        }
        path
    }

    #[cfg(unix)]
    fn write_lifecycle_helper(dir: &Path) -> PathBuf {
        let path = dir.join("ctx-avf-linux-helper-lifecycle-test.sh");
        let host_os = std::env::consts::OS;
        let host_arch = std::env::consts::ARCH;
        let script = format!(
            "#!/bin/sh\ncmd=\"$1\"\nshift\ncase \"$cmd\" in\n  probe)\n    printf '%s\\n' '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"{host_os}\",\"host_arch\":\"{host_arch}\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}}'\n    ;;\n  prepare-runtime-layout)\n    data_root=\"$1\"\n    vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n    logs_root=\"$vm_root/logs\"\n    state_path=\"$vm_root/shared-vm-state.json\"\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"layout_status\":\"prepared\",\"notes\":[\"layout ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\"\n    ;;\n  shared-vm-state)\n    data_root=\"$1\"\n    vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n    logs_root=\"$vm_root/logs\"\n    state_path=\"$vm_root/shared-vm-state.json\"\n    log_path=\"$logs_root/shared-vm.log\"\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"stopped\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"simulated\":true,\"notes\":[\"state ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\"\n    ;;\n  start-shared-vm)\n    data_root=\"$1\"\n    runtime_root=\"$2\"\n    rootfs_image=\"$3\"\n    kernel_path=\"$4\"\n    initrd_path=\"$5\"\n    runtime_version=\"$6\"\n    vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n    logs_root=\"$vm_root/logs\"\n    state_path=\"$vm_root/shared-vm-state.json\"\n    log_path=\"$logs_root/shared-vm.log\"\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"runtime_root\":\"%s\",\"rootfs_image\":\"%s\",\"kernel_path\":\"%s\",\"initrd_path\":\"%s\",\"runtime_version\":\"%s\",\"transition_status\":\"scaffolded\",\"simulated\":true,\"notes\":[\"scaffolded\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\" \"$runtime_root\" \"$rootfs_image\" \"$kernel_path\" \"$initrd_path\" \"$runtime_version\"\n    ;;\n  stop-shared-vm)\n    data_root=\"$1\"\n    vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n    logs_root=\"$vm_root/logs\"\n    state_path=\"$vm_root/shared-vm-state.json\"\n    log_path=\"$logs_root/shared-vm.log\"\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"stopped\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"transition_status\":\"stopped\",\"simulated\":true,\"notes\":[\"stopped\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\"\n    ;;\n  prepare-guest-worktree)\n    data_root=\"$1\"\n    workspace_id=\"$2\"\n    worktree_id=\"$3\"\n    vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n    host_shadow_root=\"$vm_root/worktrees/$workspace_id/$worktree_id/shadow-root\"\n    metadata_path=\"$vm_root/worktrees/$workspace_id/$worktree_id/worktree.json\"\n    guest_root=\"/ctx/ws/worktrees/$worktree_id\"\n    mkdir -p \"$host_shadow_root\"\n    if [ ! -f \"$metadata_path\" ]; then\n      mkdir -p \"$(dirname \"$metadata_path\")\"\n      printf '{{\"workspace_id\":\"%s\",\"worktree_id\":\"%s\"}}\\n' \"$workspace_id\" \"$worktree_id\" > \"$metadata_path\"\n      status=\"prepared\"\n      note=\"prepared guest worktree\"\n    else\n      status=\"already_present\"\n      note=\"existing guest worktree\"\n    fi\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"workspace_id\":\"%s\",\"worktree_id\":\"%s\",\"guest_root\":\"%s\",\"host_shadow_root\":\"%s\",\"metadata_path\":\"%s\",\"status\":\"%s\",\"simulated\":true,\"notes\":[\"%s\"]}}\\n' \"$workspace_id\" \"$worktree_id\" \"$guest_root\" \"$host_shadow_root\" \"$metadata_path\" \"$status\" \"$note\"\n    ;;\n  *)\n    echo \"unexpected helper invocation: $cmd $*\" >&2\n    exit 1\n    ;;\nesac\n"
        );
        std::fs::write(&path, script).expect("write AVF Linux lifecycle helper shim");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod AVF Linux lifecycle helper shim");
        path
    }

    #[cfg(unix)]
    fn write_guest_exec_helper(dir: &Path) -> (PathBuf, PathBuf) {
        let path = dir.join("ctx-avf-linux-helper-guest-exec-test.sh");
        let log_path = dir.join("ctx-avf-linux-helper-guest-exec.log");
        let script = format!(
            "#!/bin/sh\ncmd=\"$1\"\nshift\nif [ \"$cmd\" = \"guest-exec\" ]; then\n  printf '%s\\n' \"$*\" >> \"{}\"\n  printf 'guest-exec-ok\\n'\n  exit 0\nfi\nif [ \"$cmd\" = \"probe\" ]; then\n  printf '%s\\n' '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"macos\",\"host_arch\":\"aarch64\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}}'\n  exit 0\nfi\necho \"unexpected helper invocation: $cmd $*\" >&2\nexit 1\n",
            log_path.display()
        );
        std::fs::write(&path, script).expect("write AVF Linux guest-exec helper shim");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod AVF Linux guest-exec helper shim");
        (path, log_path)
    }

    fn runtime_archive_bytes() -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        {
            let mut tar = tar::Builder::new(&mut encoder);
            let payload = b"rootfs";
            let mut header = tar::Header::new_gnu();
            header.set_path("runtime/rootfs.img").expect("set tar path");
            header.set_size(payload.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append(&header, &payload[..])
                .expect("append rootfs image");
            tar.finish().expect("finish tar");
        }
        encoder.finish().expect("finish gzip encoder")
    }

    async fn spawn_static_http_server(
        body: Vec<u8>,
        suffix: &'static str,
    ) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local http listener");
        let addr = listener.local_addr().expect("listener local addr");
        let payload = std::sync::Arc::new(body);
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                let payload = std::sync::Arc::clone(&payload);
                tokio::spawn(async move {
                    let mut req_buf = [0u8; 1024];
                    let _ = socket.read(&mut req_buf).await;
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        payload.len()
                    );
                    let _ = socket.write_all(headers.as_bytes()).await;
                    let _ = socket.write_all(payload.as_slice()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        (format!("http://{addr}/{suffix}"), task)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex::encode(hasher.finalize())
    }

    #[test]
    fn helper_probe_uses_configured_helper_binary() {
        let _serial = helper_env_test_lock().blocking_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        let helper = write_probe_helper(temp.path());
        let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper.to_string_lossy());

        let probe = probe_helper().expect("helper probe should succeed");
        assert!(probe.supported);
        assert_eq!(probe.protocol_schema, "ctx.avf_linux_helper.v1");
    }

    #[tokio::test]
    async fn managed_avf_linux_runtime_downloads_archive_and_helpers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let archive_bytes = runtime_archive_bytes();
        let kernel_bytes = b"kernel".to_vec();
        let initrd_bytes = b"initrd".to_vec();
        let (archive_url, archive_server) =
            spawn_static_http_server(archive_bytes.clone(), "guest-runtime.tar.gz").await;
        let (kernel_url, kernel_server) =
            spawn_static_http_server(kernel_bytes.clone(), "vmlinuz").await;
        let (initrd_url, initrd_server) =
            spawn_static_http_server(initrd_bytes.clone(), "initrd.img").await;

        let source = bundled_assets::ManagedRuntimeSource {
            uri: archive_url,
            sha256: sha256_hex(&archive_bytes),
            version: "ubuntu-minimal-test".to_string(),
            bin: "rootfs.img".to_string(),
            helpers: [
                (
                    AVF_LINUX_KERNEL_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: kernel_url,
                        sha256: sha256_hex(&kernel_bytes),
                    },
                ),
                (
                    AVF_LINUX_INITRD_HELPER.to_string(),
                    bundled_assets::ManagedArtifactSource {
                        uri: initrd_url,
                        sha256: sha256_hex(&initrd_bytes),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };

        let runtime = ensure_managed_avf_linux_guest_runtime_with_override(
            temp.path(),
            Some(&source),
            None,
            None,
        )
        .await
        .expect("managed AVF Linux runtime should install");

        assert!(runtime.rootfs_image.exists());
        assert!(runtime.kernel_path.exists());
        assert!(runtime.initrd_path.exists());
        assert!(avf_linux_runtime_is_ready(&runtime));

        archive_server.abort();
        kernel_server.abort();
        initrd_server.abort();
    }

    #[tokio::test]
    async fn ensure_avf_linux_runtime_prefers_bundled_guest_runtime_over_managed_source() {
        let _serial = helper_env_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let bundle_root = temp.path().join("bundle");
        let runtime_root = bundle_root
            .join("runtimes")
            .join(AVF_LINUX_GUEST_RUNTIME_ID)
            .join(std::env::consts::OS)
            .join(std::env::consts::ARCH)
            .join("bundled-test");
        std::fs::create_dir_all(runtime_root.join("helpers")).expect("create bundled helpers");
        std::fs::write(runtime_root.join("rootfs.raw"), b"rootfs").expect("write bundled rootfs");
        std::fs::write(runtime_root.join("helpers").join("kernel"), b"kernel")
            .expect("write bundled kernel");
        std::fs::write(runtime_root.join("helpers").join("initrd"), b"initrd")
            .expect("write bundled initrd");

        let manifest = bundled_assets::BundledAssetsManifest {
            version: 1,
            generated_at: None,
            providers: vec![],
            runtimes: vec![bundled_assets::BundledRuntime {
                id: AVF_LINUX_GUEST_RUNTIME_ID.to_string(),
                version: "bundled-test".to_string(),
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                sha256: "sha-bundled".to_string(),
                root: format!(
                    "runtimes/{}/{}/{}/{}",
                    AVF_LINUX_GUEST_RUNTIME_ID,
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                    "bundled-test"
                ),
                bin: "rootfs.raw".to_string(),
                npm_cli: None,
            }],
            images: vec![],
        };
        let _bundle_guard = bundled_assets::override_bundled_assets_manifest_for_test(
            bundle_root.clone(),
            manifest,
        );

        let runtime =
            ensure_managed_avf_linux_guest_runtime_with_override(temp.path(), None, None, None)
                .await
                .expect("bundled AVF Linux runtime should be preferred");

        assert!(!runtime.managed);
        assert_eq!(runtime.version, "bundled-test");
        assert_eq!(runtime.runtime_root, runtime_root);
        assert_eq!(runtime.rootfs_image, runtime_root.join("rootfs.raw"));
        assert_eq!(
            runtime.kernel_path,
            runtime_root.join("helpers").join("kernel")
        );
        assert_eq!(
            runtime.initrd_path,
            runtime_root.join("helpers").join("initrd")
        );
        assert_eq!(
            runtime_target_label(),
            format!("{AVF_LINUX_GUEST_RUNTIME_ID}:bundled:bundled-test")
        );
    }

    #[cfg(unix)]
    #[test]
    fn helper_lifecycle_commands_round_trip_structured_state() {
        let _serial = helper_env_test_lock().blocking_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        let helper = write_lifecycle_helper(temp.path());
        let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper.to_string_lossy());
        let data_root = temp.path().join("ctx-data");

        let layout = prepare_runtime_layout(&data_root).expect("prepare runtime layout");
        assert_eq!(layout.layout_status, AvfLinuxRuntimeLayoutStatus::Prepared);
        assert!(layout.vm_root.ends_with(Path::new("shared")));

        let state = shared_vm_state(&data_root).expect("shared VM state");
        assert_eq!(state.state, AvfLinuxSharedVmLifecycleState::Stopped);
        assert!(state.simulated);

        let runtime_root = temp.path().join("runtime-root");
        let rootfs_image = runtime_root.join("rootfs.img");
        let kernel_path = runtime_root.join("helpers").join("kernel");
        let initrd_path = runtime_root.join("helpers").join("initrd");
        std::fs::create_dir_all(kernel_path.parent().expect("kernel parent"))
            .expect("create helpers dir");
        std::fs::write(&rootfs_image, b"rootfs").expect("write rootfs image");
        std::fs::write(&kernel_path, b"kernel").expect("write kernel");
        std::fs::write(&initrd_path, b"initrd").expect("write initrd");
        let runtime = AvfLinuxGuestRuntime {
            runtime_root,
            rootfs_image,
            kernel_path,
            initrd_path,
            guest_agent_path: None,
            version: "ubuntu-minimal-test".to_string(),
            managed: false,
        };

        let started = start_shared_vm(&data_root, &runtime).expect("start shared VM");
        assert_eq!(
            started.transition_status,
            Some(AvfLinuxSharedVmTransitionStatus::Scaffolded)
        );
        assert_eq!(started.state, AvfLinuxSharedVmLifecycleState::Running);
        assert_eq!(
            started.runtime_version.as_deref(),
            Some("ubuntu-minimal-test")
        );

        let stopped = stop_shared_vm(&data_root).expect("stop shared VM");
        assert_eq!(
            stopped.transition_status,
            Some(AvfLinuxSharedVmTransitionStatus::Stopped)
        );
        assert_eq!(stopped.state, AvfLinuxSharedVmLifecycleState::Stopped);
    }

    #[cfg(unix)]
    #[test]
    fn helper_prepare_guest_worktree_round_trips_structured_state() {
        let _serial = helper_env_test_lock().blocking_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        let helper = write_lifecycle_helper(temp.path());
        let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper.to_string_lossy());
        let data_root = temp.path().join("ctx-data");

        let runtime_root = temp.path().join("runtime-root");
        let rootfs_image = runtime_root.join("rootfs.img");
        let kernel_path = runtime_root.join("helpers").join("kernel");
        let initrd_path = runtime_root.join("helpers").join("initrd");
        std::fs::create_dir_all(kernel_path.parent().expect("kernel parent"))
            .expect("create helpers dir");
        std::fs::write(&rootfs_image, b"rootfs").expect("write rootfs image");
        std::fs::write(&kernel_path, b"kernel").expect("write kernel");
        std::fs::write(&initrd_path, b"initrd").expect("write initrd");
        let runtime = AvfLinuxGuestRuntime {
            runtime_root,
            rootfs_image,
            kernel_path,
            initrd_path,
            guest_agent_path: None,
            version: "ubuntu-minimal-test".to_string(),
            managed: false,
        };
        let _ = start_shared_vm(&data_root, &runtime).expect("start shared VM");

        let prepared = prepare_guest_worktree(
            &data_root,
            WorkspaceId::new(),
            WorktreeId::new(),
            temp.path(),
            "deadbeef",
            "ctx/test/worktree",
        )
        .expect("prepare guest worktree");
        assert_eq!(prepared.status, AvfLinuxGuestWorktreeStatus::Prepared);
        assert!(prepared.guest_root.starts_with("/ctx/ws/worktrees"));
        assert!(prepared.host_shadow_root.exists());

        let reused = prepare_guest_worktree(
            &data_root,
            WorkspaceId(uuid::Uuid::parse_str(&prepared.workspace_id).expect("workspace id")),
            WorktreeId(uuid::Uuid::parse_str(&prepared.worktree_id).expect("worktree id")),
            temp.path(),
            "deadbeef",
            "ctx/test/worktree",
        )
        .expect("reused guest worktree");
        assert_eq!(reused.status, AvfLinuxGuestWorktreeStatus::AlreadyPresent);
        assert_eq!(reused.guest_root, prepared.guest_root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_guest_exec_capture_invokes_helper_with_expected_args() {
        let _serial = helper_env_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let (helper, log_path) = write_guest_exec_helper(temp.path());
        let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper.to_string_lossy());
        let workspace_id = WorkspaceId::new();
        let worktree_id = WorktreeId::new();
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());

        let output = run_guest_exec_capture(
            temp.path(),
            workspace_id,
            worktree_id,
            Path::new("/ctx/ws/worktrees/demo/src"),
            "/usr/bin/env",
            &["--version".to_string()],
            &env,
            Some("ctxagent"),
            true,
        )
        .await
        .expect("guest exec capture");

        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "guest-exec-ok\n");

        let log = std::fs::read_to_string(&log_path).expect("read helper log");
        assert!(log.contains("--workspace-id"));
        assert!(log.contains(&workspace_id.0.to_string()));
        assert!(log.contains("--worktree-id"));
        assert!(log.contains(&worktree_id.0.to_string()));
        assert!(log.contains("--cwd /ctx/ws/worktrees/demo/src"));
        assert!(log.contains("--command /usr/bin/env"));
        assert!(log.contains("--user ctxagent"));
        assert!(log.contains("--pty"));
        assert!(log.contains("--env TERM=xterm-256color"));
        assert!(log.contains("-- --version"));
    }
}
