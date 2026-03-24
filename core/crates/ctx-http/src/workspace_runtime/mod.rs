use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ctx_core::ids::SessionId;
use ctx_core::models::ExecutionEnvironment;
use ctx_store::StoreManager;
use futures::StreamExt;
use sha2::Digest;
use sysinfo::System;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::{fs, io::AsyncWriteExt};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::worktrees_root;
use serde::{Deserialize, Serialize};

use crate::bundled_assets;
use crate::network_allowlist;
use crate::resource_utilization::{ResourceSampler, SystemSnapshot};
use crate::settings::{
    normalize_container_machine_idle_shutdown_seconds, ContainerExecutionSettings,
    ContainerMachineMemoryProfile, ContainerMountMode, ContainerNetworkMode, ContainerRuntimeKind,
    ExecutionMode, ExecutionSettings,
};
use crate::terminals::TerminalManager;
use crate::updates;
use url::Url;

mod avf_linux_vm;
mod container;
mod image;
mod machine;
mod network_policy_transition;
mod podman;
mod podman_machine_lifecycle;
mod podman_recovery;
#[cfg(test)]
mod reclaim_unit_tests;

static AVF_DAEMON_GATEWAY_PROXIES: OnceLock<StdMutex<HashMap<u16, tokio::task::JoinHandle<()>>>> =
    OnceLock::new();

pub(crate) use self::avf_linux_vm::build_guest_exec_command as build_avf_linux_guest_exec_command;
pub(crate) use self::avf_linux_vm::ensure_guest_worktree_from_host_copy as ensure_avf_linux_guest_worktree_from_host_copy;
pub(crate) use self::avf_linux_vm::helper_path as avf_linux_helper_path;
#[cfg(test)]
pub(crate) use self::avf_linux_vm::override_managed_avf_linux_runtime_source_for_test;
pub(crate) use self::avf_linux_vm::run_guest_exec_capture as run_avf_linux_guest_exec_capture;
#[cfg(test)]
pub(crate) use self::avf_linux_vm::AVF_LINUX_HELPER_PATH_ENV;
use self::avf_linux_vm::{
    ensure_workspace_vm_ready_with_observer as ensure_avf_linux_workspace_vm_ready_with_observer,
    prefetch_runtime_with_observer as prefetch_avf_linux_runtime_with_observer,
    runtime_available as avf_linux_runtime_available, runtime_state as avf_linux_runtime_state,
    runtime_target_label as avf_linux_runtime_target_label,
    stop_workspace_vm as stop_avf_linux_workspace_vm,
    workspace_vm_data_root as avf_linux_workspace_vm_data_root,
    workspace_vm_state as avf_linux_workspace_vm_state,
};
pub(crate) use self::container::AVF_GUEST_HOST_GATEWAY;
#[cfg(test)]
use self::container::{bind_mount, should_mount_bundle_dir_in_container};
use self::container::{
    build_mounts, container_data_root, container_user, daemon_port_from_url,
    podman_machine_required, proxy_runtime_path, proxy_runtime_root,
    rewrite_daemon_url_for_avf_guest, rewrite_daemon_url_for_container, should_use_keep_id_userns,
    verify_disk_isolated_container_mounts,
};
use self::image::ensure_container_image_available;
pub(crate) use self::image::resolve_container_image;
pub use self::image::{
    bundled_default_container_image_tar, container_image_present, container_image_status,
    default_container_image, is_default_container_image, prefetch_container_image,
    prefetch_container_image_with_observer, prefetch_container_startup_artifacts_with_observer,
    ContainerImageStatus,
};
#[cfg(test)]
use self::image::{
    ensure_managed_default_container_image_tar_with_source, managed_default_image_install_lock,
};
use self::machine::{
    ctx_podman_machine_name, download_managed_artifact, ensure_managed_podman_machine_cache,
    ensure_managed_podman_runtime, managed_podman_runtime_bin_path, managed_podman_runtime_source,
    persist_podman_machine_cache_to_shared_best_effort, podman_home_root, podman_runtime_root,
    podman_temp_root, seed_shared_podman_machine_cache_best_effort,
    ManagedArtifactDownloadReporter, ManagedDownloadAggregate,
};
#[cfg(test)]
use self::machine::{
    persist_podman_machine_cache_to_shared, podman_machine_cache_root,
    seed_shared_podman_machine_cache,
};
use self::network_policy_transition::{
    apply_avf_linux_network_policy, apply_container_network_policy,
};
#[cfg(test)]
use self::podman::podman_binary_path;
use self::podman::{
    command_output_message, container_exists, container_running, ensure_workspace_volume,
};
pub(crate) use self::podman::{
    command_output_with_timeout, container_runtime_available, podman_command, podman_engine_ready,
    podman_invocation,
};
#[allow(unused_imports)]
pub(crate) use self::podman_machine_lifecycle::{
    HarnessRuntimeStats, PrewarmArtifactActivityGuard, RuntimeOperationGuard,
};
use self::podman_recovery::{
    ensure_podman_machine_running_with_observer, podman_machine_present,
    podman_machine_singleflight_lock, run_podman_machine_init,
};

pub(crate) fn local_runtime_available(data_root: &Path, runtime: &ContainerRuntimeKind) -> bool {
    match runtime {
        ContainerRuntimeKind::Podman => container_runtime_available(data_root),
        ContainerRuntimeKind::AvfLinuxVm => avf_linux_runtime_available(),
    }
}

pub(crate) fn runtime_prewarm_target(settings: &ContainerExecutionSettings) -> String {
    match settings.runtime {
        ContainerRuntimeKind::Podman => resolve_container_image(settings),
        ContainerRuntimeKind::AvfLinuxVm => avf_linux_runtime_target_label(),
    }
}

pub(crate) async fn selected_runtime_state(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
) -> Result<(bool, bool)> {
    match settings.runtime {
        ContainerRuntimeKind::Podman => {
            let machine_ready =
                normalize_podman_engine_ready_for_runtime(podman_engine_ready(data_root).await)?;
            let image_present = if machine_ready {
                container_image_present(data_root, &resolve_container_image(settings)).await?
            } else {
                false
            };
            Ok((machine_ready, image_present))
        }
        ContainerRuntimeKind::AvfLinuxVm => avf_linux_runtime_state(data_root),
    }
}

pub(crate) async fn prewarm_selected_runtime_with_observer(
    data_root: &Path,
    settings: &ContainerExecutionSettings,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    match settings.runtime {
        ContainerRuntimeKind::Podman => {
            let image = resolve_container_image(settings);
            let machine_ready =
                normalize_podman_engine_ready_for_runtime(podman_engine_ready(data_root).await)?;
            if machine_ready {
                prefetch_container_image_with_observer(data_root, &image, observer).await
            } else {
                prefetch_container_startup_artifacts_with_observer(data_root, &image, observer)
                    .await
            }
        }
        ContainerRuntimeKind::AvfLinuxVm => {
            prefetch_avf_linux_runtime_with_observer(data_root, settings, observer).await
        }
    }
}

fn normalize_podman_engine_ready_for_runtime(result: Result<bool>) -> Result<bool> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => {
            if err
                .to_string()
                .to_ascii_lowercase()
                .contains("podman binary unavailable")
            {
                return Ok(false);
            }
            Err(err)
        }
    }
}

// Default container image for ctx-managed execution.
//
// This must include:
// - iptables (for restricted egress enforcement)
// - /usr/local/bin/ctx-egress-proxy (Linux binary executed inside the container)
const DEFAULT_CONTAINER_IMAGE: &str = "ghcr.io/ctxrs/ctx-harness:ubuntu-24.04";
const PODMAN_PATH_ENV: &str = "CTX_PODMAN_PATH";
const PODMAN_MACHINE_CACHE_DIR_ENV: &str = "CTX_PODMAN_MACHINE_CACHE_DIR";
const EGRESS_PROXY_BINARY: &str = "ctx-egress-proxy";
const EGRESS_PROXY_RUNTIME_ID: &str = "ctx-egress-proxy";
const EGRESS_PROXY_CONFIG_NAME: &str = "egress-proxy.json";
const TRANSPARENT_PROXY_PORT: u16 = 15001;
const EGRESS_PROXY_CONTAINER_PATH: &str = "/usr/local/bin/ctx-egress-proxy";
// Dedicated Podman machine name prefix for ctx-managed container execution on macOS/Windows.
//
// Final machine name is deterministic per daemon data_root to avoid cross-daemon collisions in
// Podman's host-global machine temp/socket state.
const CTX_PODMAN_MACHINE_PREFIX: &str = "ctx";
// In-container root for disk-isolated workspaces (Podman volume mounted here).
pub(crate) const CTX_CONTAINER_WORKSPACE_ROOT: &str = "/ctx/ws";
pub(crate) const CTX_HARNESS_RUNTIME_KIND_ENV: &str = "CTX_HARNESS_RUNTIME_KIND";
pub(crate) const CTX_HARNESS_LINUX_SANDBOX_ENV: &str = "CTX_HARNESS_LINUX_SANDBOX";
pub(crate) const CTX_AVF_HOST_DATA_ROOT_ENV: &str = "CTX_AVF_HOST_DATA_ROOT";
pub(crate) const CTX_AVF_WORKSPACE_ID_ENV: &str = "CTX_AVF_WORKSPACE_ID";
pub(crate) const CTX_AVF_WORKTREE_ID_ENV: &str = "CTX_AVF_WORKTREE_ID";
pub(crate) const CTX_AVF_HOST_WORKTREE_ROOT_ENV: &str = "CTX_AVF_HOST_WORKTREE_ROOT";
const PODMAN_INFO_TIMEOUT: Duration = Duration::from_secs(5);
const PODMAN_MACHINE_START_TIMEOUT: Duration = Duration::from_secs(180);
// Bound machine init so wedged podman subprocesses cannot stall launch indefinitely.
const PODMAN_MACHINE_INIT_TIMEOUT: Duration = Duration::from_secs(8 * 60);
// First boot can be slow on fresh installs (image download + provisioning), but readiness loops
// must remain bounded tightly enough to surface actionable errors quickly.
const PODMAN_MACHINE_READY_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const PODMAN_OP_TIMEOUT: Duration = Duration::from_secs(60);
const PODMAN_LOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DEFAULT_PRESET_HOST_MEMORY_MB: u32 = 32 * 1024;
const PODMAN_MACHINE_MEMORY_PRESET_FLOOR_MB: u32 = 4096;
const PODMAN_MACHINE_MEMORY_ECONOMY_CAP_MB: u32 = 8192;
const PODMAN_MACHINE_MEMORY_BALANCED_CAP_MB: u32 = 16 * 1024;
const PODMAN_MACHINE_MEMORY_PERFORMANCE_CAP_MB: u32 = 32 * 1024;
const MI_B: u64 = 1024 * 1024;

fn avf_daemon_gateway_proxies() -> &'static StdMutex<HashMap<u16, tokio::task::JoinHandle<()>>> {
    AVF_DAEMON_GATEWAY_PROXIES.get_or_init(|| StdMutex::new(HashMap::new()))
}

async fn ensure_avf_guest_gateway_proxy(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    {
        let mut proxies = avf_daemon_gateway_proxies()
            .lock()
            .expect("AVF daemon gateway proxy mutex poisoned");
        proxies.retain(|_, handle| !handle.is_finished());
        if proxies.contains_key(&port) {
            return Ok(());
        }
    }

    match tokio::net::TcpListener::bind(gateway_addr).await {
        Ok(listener) => {
            let gateway_addr = gateway_addr.to_string();
            let backend_addr = backend_addr.to_string();
            let gateway_addr_for_task = gateway_addr.clone();
            let backend_addr_for_task = backend_addr.clone();
            let handle = tokio::spawn(async move {
                loop {
                    let (mut inbound, peer_addr) = match listener.accept().await {
                        Ok(parts) => parts,
                        Err(err) => {
                            tracing::warn!(
                                gateway_addr = gateway_addr_for_task,
                                backend_addr = backend_addr_for_task,
                                "AVF daemon gateway proxy accept failed: {err}"
                            );
                            break;
                        }
                    };
                    let backend_addr = backend_addr_for_task.clone();
                    let gateway_addr = gateway_addr_for_task.clone();
                    tokio::spawn(async move {
                        match tokio::net::TcpStream::connect(&backend_addr).await {
                            Ok(mut outbound) => {
                                if let Err(err) =
                                    tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await
                                {
                                    tracing::debug!(
                                        gateway_addr,
                                        backend_addr,
                                        %peer_addr,
                                        "AVF daemon gateway proxy relay closed with error: {err}"
                                    );
                                }
                            }
                            Err(err) => {
                                tracing::warn!(
                                    gateway_addr,
                                    backend_addr,
                                    %peer_addr,
                                    "AVF daemon gateway proxy could not connect to backend: {err}"
                                );
                            }
                        }
                    });
                }
            });
            let mut proxies = avf_daemon_gateway_proxies()
                .lock()
                .expect("AVF daemon gateway proxy mutex poisoned");
            if let Some(existing) = proxies.get(&port) {
                if !existing.is_finished() {
                    handle.abort();
                    return Ok(());
                }
            }
            proxies.insert(port, handle);
            tracing::info!(
                gateway_addr,
                backend_addr,
                "started AVF daemon gateway proxy"
            );
            Ok(())
        }
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            tracing::debug!(
                gateway_addr,
                backend_addr,
                "AVF daemon gateway proxy port already in use; assuming a guest-reachable listener already exists"
            );
            Ok(())
        }
        Err(err) => Err(err).with_context(|| {
            format!("binding AVF guest gateway proxy at {gateway_addr} for {backend_addr}")
        }),
    }
}

async fn resolve_daemon_url_for_avf_guest(daemon_url: &str) -> Result<String> {
    let Ok(url) = Url::parse(daemon_url) else {
        return Ok(daemon_url.to_string());
    };
    let Some(host) = url.host_str() else {
        return Ok(daemon_url.to_string());
    };
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Ok(daemon_url.to_string());
    }
    let Some(port) = url.port_or_known_default() else {
        return Ok(daemon_url.to_string());
    };
    let gateway_addr = format!("{AVF_GUEST_HOST_GATEWAY}:{port}");
    match tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(&gateway_addr),
    )
    .await
    {
        Ok(Ok(_)) => Ok(rewrite_daemon_url_for_avf_guest(daemon_url)),
        Ok(Err(_)) | Err(_) => {
            let backend_host = match host {
                "localhost" | "::1" => "127.0.0.1",
                other => other,
            };
            let backend_addr = format!("{backend_host}:{port}");
            ensure_avf_guest_gateway_proxy(&gateway_addr, &backend_addr, port).await?;
            Ok(rewrite_daemon_url_for_avf_guest(daemon_url))
        }
    }
}

#[cfg(test)]
pub(crate) async fn ensure_avf_guest_gateway_proxy_for_test(
    gateway_addr: &str,
    backend_addr: &str,
    port: u16,
) -> Result<()> {
    ensure_avf_guest_gateway_proxy(gateway_addr, backend_addr, port).await
}

fn podman_machine_reclaim_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(30)
    }
}

fn podman_machine_pressure_idle_grace() -> Duration {
    if cfg!(test) {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(60)
    }
}

fn detected_host_memory_mb() -> Option<u32> {
    #[cfg(test)]
    if let Ok(raw) = std::env::var("CTX_TEST_HOST_MEMORY_MB") {
        if let Ok(value) = raw.parse::<u32>() {
            if value > 0 {
                return Some(value);
            }
        }
    }

    let mut system = System::new();
    system.refresh_memory();
    let total_bytes = system.total_memory();
    let total_mb = total_bytes / MI_B;
    if total_mb == 0 {
        return None;
    }
    u32::try_from(total_mb).ok()
}

fn preset_memory_mb(total_memory_mb: u32, numerator: u32, denominator: u32, cap_mb: u32) -> u32 {
    total_memory_mb
        .saturating_mul(numerator)
        .checked_div(denominator)
        .unwrap_or(PODMAN_MACHINE_MEMORY_PRESET_FLOOR_MB)
        .clamp(PODMAN_MACHINE_MEMORY_PRESET_FLOOR_MB, cap_mb)
}

fn container_machine_memory_mb_for_host_memory(
    settings: &ContainerExecutionSettings,
    host_memory_mb: u32,
) -> u32 {
    match settings.machine.memory_profile {
        ContainerMachineMemoryProfile::Economy => {
            preset_memory_mb(host_memory_mb, 1, 8, PODMAN_MACHINE_MEMORY_ECONOMY_CAP_MB)
        }
        ContainerMachineMemoryProfile::Balanced => {
            preset_memory_mb(host_memory_mb, 1, 4, PODMAN_MACHINE_MEMORY_BALANCED_CAP_MB)
        }
        ContainerMachineMemoryProfile::Performance => preset_memory_mb(
            host_memory_mb,
            1,
            2,
            PODMAN_MACHINE_MEMORY_PERFORMANCE_CAP_MB,
        ),
        ContainerMachineMemoryProfile::Custom => settings
            .machine
            .custom_memory_mb
            .unwrap_or(preset_memory_mb(
                host_memory_mb,
                1,
                4,
                PODMAN_MACHINE_MEMORY_BALANCED_CAP_MB,
            ))
            .max(1024),
    }
}

fn container_machine_memory_mb(settings: &ContainerExecutionSettings) -> u32 {
    let host_memory_mb = detected_host_memory_mb().unwrap_or(DEFAULT_PRESET_HOST_MEMORY_MB);
    container_machine_memory_mb_for_host_memory(settings, host_memory_mb)
}

fn podman_machine_init_created_machine_grace() -> Duration {
    if cfg!(test) {
        Duration::from_millis(300)
    } else {
        Duration::from_secs(10)
    }
}

fn podman_machine_init_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(1)
    }
}

fn podman_machine_ready_timeout() -> Duration {
    if cfg!(test) {
        Duration::from_millis(300)
    } else {
        PODMAN_MACHINE_READY_TIMEOUT
    }
}

fn podman_machine_ready_poll_interval() -> Duration {
    if cfg!(test) {
        Duration::from_millis(25)
    } else {
        Duration::from_secs(1)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSetupPhase {
    ArtifactDownload,
    MachineCheck,
    MachineStartOrInit,
    ImageCheck,
    ImageLoad,
    ContainerCheck,
    ContainerStartOrCreate,
    RuntimeNetworkSetup,
    Ready,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HarnessSetupLogLevel {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessSetupDownloadStatus {
    pub artifact: String,
    pub downloaded_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_per_sec: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessSetupProgressUpdate {
    pub phase: HarnessSetupPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_download: Option<HarnessSetupDownloadStatus>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ManagedContainerBootstrapOverrides {
    pub(crate) podman_runtime_source: Option<bundled_assets::ManagedRuntimeSource>,
    pub(crate) default_image_source: Option<bundled_assets::ManagedArtifactSource>,
}

pub trait HarnessSetupObserver: Send + Sync {
    fn on_phase(&self, phase: HarnessSetupPhase, message: &str);
    fn on_log(&self, phase: HarnessSetupPhase, level: HarnessSetupLogLevel, message: &str);
    fn on_progress(&self, _progress: HarnessSetupProgressUpdate) {}
}

fn observe_phase(
    observer: Option<&dyn HarnessSetupObserver>,
    phase: HarnessSetupPhase,
    message: &str,
) {
    if let Some(observer) = observer {
        observer.on_phase(phase, message);
    }
}

fn observe_log(
    observer: Option<&dyn HarnessSetupObserver>,
    phase: HarnessSetupPhase,
    level: HarnessSetupLogLevel,
    message: &str,
) {
    if let Some(observer) = observer {
        observer.on_log(phase, level, message);
    }
}

fn observe_progress(
    observer: Option<&dyn HarnessSetupObserver>,
    progress: HarnessSetupProgressUpdate,
) {
    if let Some(observer) = observer {
        observer.on_progress(progress);
    }
}

pub(crate) fn workspace_container_name(workspace_id: WorkspaceId) -> String {
    format!("ctx-harness-{}", workspace_id.0)
}

#[derive(Debug, Clone)]
pub enum HarnessRuntimeKind {
    Host,
    Container { name: String },
    AvfLinuxVm,
}

impl HarnessRuntimeKind {
    pub fn is_linux_sandbox(&self) -> bool {
        !matches!(self, Self::Host)
    }
}

#[derive(Debug, Clone)]
pub struct HarnessExecutionPlan {
    pub runtime: HarnessRuntimeKind,
    pub env_overrides: HashMap<String, String>,
}

impl HarnessExecutionPlan {
    pub fn is_linux_sandbox(&self) -> bool {
        self.runtime.is_linux_sandbox()
            || self
                .env_overrides
                .get(CTX_HARNESS_LINUX_SANDBOX_ENV)
                .is_some_and(|value| value == "1")
    }

    pub fn runtime_data_root(&self) -> Option<&Path> {
        self.env_overrides.get("CTX_DATA_ROOT").map(Path::new)
    }
}

#[derive(Debug, Clone)]
struct HarnessContainer {
    name: String,
    mount_mode: ContainerMountMode,
    network_mode: ContainerNetworkMode,
    allowlist: Vec<String>,
    external_mounts: HashSet<String>,
    egress_guard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CachedContainerAction {
    Reuse,
    Reconfigure,
    Recreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerReadinessState {
    MachineReady,
    RuntimeReady,
}

struct EnsureContainerRequest<'a> {
    workspace: &'a Workspace,
    worktree: Option<&'a Worktree>,
    settings: &'a ContainerExecutionSettings,
    daemon_host: &'a str,
    daemon_port: u16,
    observer: Option<&'a dyn HarnessSetupObserver>,
    readiness: ContainerReadinessState,
}

fn cached_container_action(
    cached: &HarnessContainer,
    settings: &ContainerExecutionSettings,
    external_mounts: &HashSet<String>,
) -> CachedContainerAction {
    if cached.mount_mode != settings.mount_mode || cached.external_mounts != *external_mounts {
        return CachedContainerAction::Recreate;
    }
    if cached.network_mode != settings.network_mode || cached.allowlist != settings.allowlist {
        return CachedContainerAction::Reconfigure;
    }
    CachedContainerAction::Reuse
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessContainerStatus {
    pub name: String,
    pub running: bool,
    pub known: bool,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Vec<String>,
    pub egress_guard: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct TransparentProxyConfig {
    listen: String,
    mode: ContainerNetworkMode,
    allowlist: Vec<String>,
    max_peek_bytes: usize,
}

pub struct HarnessRuntimeManager {
    data_root: PathBuf,
    containers: Mutex<HashMap<WorkspaceId, HarnessContainer>>,
    last_activity: StdMutex<Instant>,
    active_runtime_operations: AtomicUsize,
    active_prewarm_artifact_operations: AtomicUsize,
    reclaim_loop_started: AtomicBool,
}

impl HarnessRuntimeManager {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            containers: Mutex::new(HashMap::new()),
            last_activity: StdMutex::new(Instant::now()),
            active_runtime_operations: AtomicUsize::new(0),
            active_prewarm_artifact_operations: AtomicUsize::new(0),
            reclaim_loop_started: AtomicBool::new(false),
        }
    }

    pub async fn prepare(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<HarnessExecutionPlan> {
        let mut env_overrides = HashMap::new();
        env_overrides.insert(
            "CTX_DATA_ROOT_HOST".to_string(),
            self.data_root.to_string_lossy().to_string(),
        );
        env_overrides.insert(CTX_HARNESS_RUNTIME_KIND_ENV.to_string(), "host".to_string());
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::Host,
                env_overrides,
            });
        }
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            let _activity = self.begin_runtime_operation();
            if !matches!(
                settings.container.mount_mode,
                ContainerMountMode::DiskIsolated
            ) {
                anyhow::bail!(
                    "AVF Linux VM host-mounted workspaces are not implemented yet; use disk-isolated mode"
                );
            }
            let workspace_vm = ensure_avf_linux_workspace_vm_ready_with_observer(
                &self.data_root,
                workspace.id,
                &settings.container,
                None,
            )
            .await?;
            let branch_name = worktree
                .git_branch
                .as_deref()
                .or(worktree.vcs_ref.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| format!("ctx/{}/{}", workspace.id.0, worktree.id.0));
            let guest_worktree_root = ensure_avf_linux_guest_worktree_from_host_copy(
                &self.data_root,
                workspace.id,
                worktree.id,
                Path::new(&workspace.root_path),
                &worktree.base_commit_sha,
                &branch_name,
                None,
            )
            .await?;
            let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
            let egress_guard = apply_avf_linux_network_policy(
                &self.data_root,
                workspace.id,
                worktree.id,
                &guest_worktree_root,
                &settings.container,
                "192.168.64.1",
                daemon_port,
            )
            .await?
            .egress_guard;
            let avf_data_root = container_data_root(&self.data_root, workspace.id);
            tokio::fs::create_dir_all(&avf_data_root).await.ok();
            env_overrides.insert(
                "CTX_DATA_ROOT".to_string(),
                avf_data_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                CTX_HARNESS_RUNTIME_KIND_ENV.to_string(),
                "avf_linux_vm".to_string(),
            );
            env_overrides.insert(CTX_HARNESS_LINUX_SANDBOX_ENV.to_string(), "1".to_string());
            env_overrides.insert(
                "CTX_HARNESS_GUEST_WORKSPACE_ROOT".to_string(),
                CTX_CONTAINER_WORKSPACE_ROOT.to_string(),
            );
            env_overrides.insert(
                "CTX_AVF_WORKSPACE_VM_ROOT".to_string(),
                workspace_vm.vm_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                avf_linux_vm::AVF_LINUX_HELPER_PATH_ENV.to_string(),
                avf_linux_helper_path()?.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                CTX_AVF_HOST_DATA_ROOT_ENV.to_string(),
                self.data_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                "CTX_AVF_WORKSPACE_VM_DATA_ROOT".to_string(),
                avf_linux_workspace_vm_data_root(&self.data_root, workspace.id)
                    .to_string_lossy()
                    .to_string(),
            );
            env_overrides.insert(
                CTX_AVF_WORKSPACE_ID_ENV.to_string(),
                workspace.id.0.to_string(),
            );
            env_overrides.insert(
                CTX_AVF_WORKTREE_ID_ENV.to_string(),
                worktree.id.0.to_string(),
            );
            env_overrides.insert(
                CTX_AVF_HOST_WORKTREE_ROOT_ENV.to_string(),
                worktree.root_path.clone(),
            );
            env_overrides.insert(
                "CTX_AVF_GUEST_WORKTREE_ROOT".to_string(),
                guest_worktree_root.to_string_lossy().to_string(),
            );
            env_overrides.insert(
                "CTX_DAEMON_URL".to_string(),
                resolve_daemon_url_for_avf_guest(daemon_url).await?,
            );
            if let Some(log_path) = workspace_vm.log_path.as_ref() {
                env_overrides.insert(
                    "CTX_AVF_WORKSPACE_VM_LOG".to_string(),
                    log_path.to_string_lossy().to_string(),
                );
            }
            {
                let mut containers = self.containers.lock().await;
                containers.insert(
                    workspace.id,
                    HarnessContainer {
                        name: format!("ctx-avf-linux-vm-{}", workspace.id.0),
                        mount_mode: settings.container.mount_mode.clone(),
                        network_mode: settings.container.network_mode.clone(),
                        allowlist: settings.container.allowlist.clone(),
                        external_mounts: HashSet::new(),
                        egress_guard,
                    },
                );
            }
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::AvfLinuxVm,
                env_overrides,
            });
        }
        let _activity = self.begin_runtime_operation();
        let podman_bin = ensure_managed_podman_runtime(&self.data_root, None, None)
            .await
            .context("podman unavailable and execution mode is container")?;
        env_overrides.insert(
            PODMAN_PATH_ENV.to_string(),
            podman_bin.to_string_lossy().to_string(),
        );

        let proxy_host = "host.containers.internal";
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let container = self
            .ensure_container(
                workspace,
                Some(worktree),
                &settings.container,
                proxy_host,
                daemon_port,
                None,
            )
            .await
            .map_err(|err| anyhow::anyhow!("container runtime failed: {err:#}"))?;

        let container_data_root = container_data_root(&self.data_root, workspace.id);
        tokio::fs::create_dir_all(&container_data_root).await.ok();
        env_overrides.insert(
            "CTX_DATA_ROOT".to_string(),
            container_data_root.to_string_lossy().to_string(),
        );
        env_overrides.insert(
            CTX_HARNESS_RUNTIME_KIND_ENV.to_string(),
            "podman_container".to_string(),
        );
        env_overrides.insert(CTX_HARNESS_LINUX_SANDBOX_ENV.to_string(), "1".to_string());

        let daemon_url = rewrite_daemon_url_for_container(daemon_url, proxy_host);
        env_overrides.insert("CTX_DAEMON_URL".to_string(), daemon_url);

        env_overrides.insert(
            "CTX_HARNESS_CONTAINER_ID".to_string(),
            container.name.clone(),
        );
        if let Some(user) = container_user() {
            env_overrides.insert("CTX_HARNESS_CONTAINER_USER".to_string(), user);
        }

        Ok(HarnessExecutionPlan {
            runtime: HarnessRuntimeKind::Container {
                name: container.name,
            },
            env_overrides,
        })
    }

    pub async fn ensure_workspace_container(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<()> {
        self.ensure_workspace_container_with_observer(workspace, settings, daemon_url, None)
            .await
    }

    pub async fn ensure_workspace_container_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        let _activity = self.begin_runtime_operation();
        self.ensure_container_machine_ready(&settings.container, observer)
            .await
            .context("local sandbox runtime is unavailable")?;
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            return Ok(());
        }
        self.ensure_workspace_container_after_machine_ready_with_observer(
            workspace, settings, daemon_url, observer,
        )
        .await
    }

    pub(crate) async fn ensure_workspace_container_after_machine_ready_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.ensure_workspace_container_after_readiness_with_observer(
            workspace,
            settings,
            daemon_url,
            observer,
            ContainerReadinessState::MachineReady,
        )
        .await
    }

    async fn ensure_workspace_container_after_readiness_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
        readiness: ContainerReadinessState,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        if matches!(settings.container.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            return Ok(());
        }
        let proxy_host = "host.containers.internal";
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let _ = self
            .ensure_container_after_machine_ready(EnsureContainerRequest {
                workspace,
                worktree: None,
                settings: &settings.container,
                daemon_host: proxy_host,
                daemon_port,
                observer,
                readiness,
            })
            .await?;
        Ok(())
    }

    pub async fn ensure_workspace_container_after_runtime_ready_with_observer(
        &self,
        workspace: &Workspace,
        settings: &ExecutionSettings,
        daemon_url: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        let _activity = self.begin_runtime_operation();
        self.ensure_workspace_container_after_readiness_with_observer(
            workspace,
            settings,
            daemon_url,
            observer,
            ContainerReadinessState::RuntimeReady,
        )
        .await
    }

    pub(crate) async fn ensure_container_machine_ready(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if matches!(settings.runtime, ContainerRuntimeKind::AvfLinuxVm) {
            prefetch_avf_linux_runtime_with_observer(&self.data_root, settings, observer).await?;
            return Ok(());
        }
        observe_phase(
            observer,
            HarnessSetupPhase::MachineCheck,
            "checking container runtime",
        );
        ensure_managed_podman_runtime(&self.data_root, observer, None).await?;
        if podman_engine_ready(&self.data_root).await.unwrap_or(false) {
            self.reconcile_running_podman_machine_memory(settings, observer)
                .await?;
            if !podman_engine_ready(&self.data_root).await.unwrap_or(false) {
                self.ensure_podman_machine_materialized(settings, observer)
                    .await?;
                ensure_podman_machine_running_with_observer(&self.data_root, observer).await?;
                return Ok(());
            }
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Info,
                "local sandbox runtime is already reachable",
            );
            return Ok(());
        }
        self.ensure_podman_machine_materialized(settings, observer)
            .await?;
        ensure_podman_machine_running_with_observer(&self.data_root, observer).await?;
        Ok(())
    }

    pub(crate) async fn workspace_container_exists(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<bool> {
        let name = workspace_container_name(workspace_id);
        match container_exists(&self.data_root, &name).await {
            Ok(exists) => Ok(exists),
            Err(err) => {
                if podman_engine_ready(&self.data_root).await.unwrap_or(false) {
                    Err(err)
                } else {
                    Ok(false)
                }
            }
        }
    }

    async fn ensure_container_image_ready(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let image = resolve_container_image(settings);
        observe_phase(
            observer,
            HarnessSetupPhase::ImageCheck,
            "checking harness image availability",
        );
        if container_image_present(&self.data_root, &image).await? {
            observe_log(
                observer,
                HarnessSetupPhase::ImageCheck,
                HarnessSetupLogLevel::Info,
                "harness image already present",
            );
            return Ok(());
        }
        observe_phase(
            observer,
            HarnessSetupPhase::ImageLoad,
            "loading harness image into local sandbox runtime",
        );
        ensure_container_image_available(&self.data_root, &image, observer).await
    }

    pub async fn container_status(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<HarnessContainerStatus>> {
        let name = format!("ctx-harness-{}", workspace_id.0);
        let podman_exists = match container_exists(&self.data_root, &name).await {
            Ok(exists) => exists,
            Err(err) => {
                if err
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("podman binary unavailable")
                {
                    false
                } else {
                    return Err(err);
                }
            }
        };
        if podman_exists {
            let running = container_running(&self.data_root, &name)
                .await?
                .unwrap_or(false);
            let container = {
                let containers = self.containers.lock().await;
                containers.get(&workspace_id).cloned()
            };
            let (known, mount_mode, network_mode, allowlist, egress_guard) =
                if let Some(container) = container {
                    (
                        true,
                        Some(container.mount_mode),
                        Some(container.network_mode),
                        container.allowlist,
                        Some(container.egress_guard),
                    )
                } else {
                    (false, None, None, Vec::new(), None)
                };
            return Ok(Some(HarnessContainerStatus {
                name,
                running,
                known,
                mount_mode,
                network_mode,
                allowlist,
                egress_guard,
            }));
        }

        let state = match avf_linux_workspace_vm_state(&self.data_root, workspace_id) {
            Ok(state) => state,
            Err(_) => return Ok(None),
        };
        if matches!(
            state.state,
            avf_linux_vm::AvfLinuxSharedVmLifecycleState::Missing
        ) {
            return Ok(None);
        }
        let container = {
            let containers = self.containers.lock().await;
            containers.get(&workspace_id).cloned()
        };
        Ok(Some(HarnessContainerStatus {
            name: format!("ctx-avf-linux-vm-{}", workspace_id.0),
            running: matches!(
                state.state,
                avf_linux_vm::AvfLinuxSharedVmLifecycleState::Running
            ),
            known: true,
            mount_mode: container
                .as_ref()
                .map(|value| value.mount_mode.clone())
                .or(Some(ContainerMountMode::DiskIsolated)),
            network_mode: container.as_ref().map(|value| value.network_mode.clone()),
            allowlist: container
                .as_ref()
                .map(|value| value.allowlist.clone())
                .unwrap_or_default(),
            egress_guard: container.as_ref().map(|value| value.egress_guard),
        }))
    }

    pub async fn stop_container(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        let name = format!("ctx-harness-{}", workspace_id.0);
        let podman_exists = match container_exists(&self.data_root, &name).await {
            Ok(exists) => exists,
            Err(err) => {
                if err
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("podman binary unavailable")
                {
                    false
                } else {
                    return Err(err);
                }
            }
        };
        if podman_exists {
            let mut containers = self.containers.lock().await;
            containers.remove(&workspace_id);
            let mut cmd = podman_command(&self.data_root)?;
            cmd.arg("rm").arg("-f").arg(&name);
            let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
            if output.status.success() {
                return Ok(true);
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                if combined.is_empty() {
                    anyhow::bail!("podman rm failed for {name} (status: {})", output.status);
                }
                anyhow::bail!("podman rm failed for {name}: {combined}");
            }
        }

        let state = match avf_linux_workspace_vm_state(&self.data_root, workspace_id) {
            Ok(state) => state,
            Err(_) => return Ok(false),
        };
        if matches!(
            state.state,
            avf_linux_vm::AvfLinuxSharedVmLifecycleState::Missing
        ) {
            return Ok(false);
        }
        let stopped = stop_avf_linux_workspace_vm(&self.data_root, workspace_id)?;
        let mut containers = self.containers.lock().await;
        containers.remove(&workspace_id);
        Ok(!matches!(
            stopped.state,
            avf_linux_vm::AvfLinuxSharedVmLifecycleState::Missing
        ))
    }

    pub async fn remove_workspace_volume(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        // Best-effort cleanup: callers (e.g. workspace deletion) may ignore failures.
        let name = format!("ctx-ws-{}", workspace_id.0);
        let mut inspect = podman_command(&self.data_root)?;
        inspect.arg("volume").arg("inspect").arg(&name);
        let out = command_output_with_timeout(inspect, PODMAN_OP_TIMEOUT).await?;
        if !out.status.success() {
            return Ok(false);
        }

        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("volume").arg("rm").arg("-f").arg(&name);
        let out = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
        if out.status.success() {
            Ok(true)
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let combined = format!("{stderr}\n{stdout}").trim().to_string();
            if combined.is_empty() {
                anyhow::bail!(
                    "podman volume rm failed for {name} (status: {})",
                    out.status
                );
            }
            anyhow::bail!("podman volume rm failed for {name}: {combined}");
        }
    }

    async fn ensure_container(
        &self,
        workspace: &Workspace,
        worktree: Option<&Worktree>,
        settings: &ContainerExecutionSettings,
        daemon_host: &str,
        daemon_port: u16,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<HarnessContainer> {
        self.ensure_container_machine_ready(settings, observer)
            .await?;
        self.ensure_container_after_machine_ready(EnsureContainerRequest {
            workspace,
            worktree,
            settings,
            daemon_host,
            daemon_port,
            observer,
            readiness: ContainerReadinessState::MachineReady,
        })
        .await
    }

    async fn ensure_container_after_machine_ready(
        &self,
        request: EnsureContainerRequest<'_>,
    ) -> Result<HarnessContainer> {
        let EnsureContainerRequest {
            workspace,
            worktree,
            settings,
            daemon_host,
            daemon_port,
            observer,
            readiness,
        } = request;
        let name = format!("ctx-harness-{}", workspace.id.0);
        let image = resolve_container_image(settings);
        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            observe_log(
                observer,
                HarnessSetupPhase::ContainerCheck,
                HarnessSetupLogLevel::Info,
                "ensuring workspace volume for disk-isolated mode",
            );
            let _ = ensure_workspace_volume(&self.data_root, workspace.id).await?;
        }
        let mount_plan = build_mounts(&self.data_root, workspace, worktree, settings);
        let mut containers = self.containers.lock().await;
        let mut recreate = false;
        observe_phase(
            observer,
            HarnessSetupPhase::ContainerCheck,
            "checking existing workspace container",
        );
        if let Some(container) = containers.get(&workspace.id).cloned() {
            match cached_container_action(&container, settings, &mount_plan.external_mounts) {
                CachedContainerAction::Reuse => {
                    let exists = container_exists(&self.data_root, &name).await?;
                    let running = if exists {
                        container_running(&self.data_root, &name)
                            .await?
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if exists && running {
                        observe_log(
                            observer,
                            HarnessSetupPhase::ContainerCheck,
                            HarnessSetupLogLevel::Info,
                            "container already ready in runtime cache",
                        );
                        return Ok(container);
                    }
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        if exists {
                            "runtime cache entry stale; workspace container is stopped and will be restarted"
                        } else {
                            "runtime cache entry stale; workspace container is missing and will be recreated"
                        },
                    );
                    containers.remove(&workspace.id);
                }
                CachedContainerAction::Reconfigure => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        "container network policy changed; reconfiguring",
                    );
                }
                CachedContainerAction::Recreate => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        "container configuration changed; recreating",
                    );
                    recreate = true;
                }
            }
        }

        if recreate {
            observe_phase(
                observer,
                HarnessSetupPhase::ContainerStartOrCreate,
                "recreating workspace container",
            );
            if let Ok(mut cmd) = podman_command(&self.data_root) {
                cmd.arg("rm").arg("-f").arg(&name);
                let _ = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await;
            }
        }

        let exists = if recreate {
            false
        } else {
            container_exists(&self.data_root, &name).await?
        };

        if exists {
            let running = container_running(&self.data_root, &name)
                .await?
                .unwrap_or(false);
            if !running {
                observe_phase(
                    observer,
                    HarnessSetupPhase::ContainerStartOrCreate,
                    "starting existing workspace container",
                );
                let mut cmd = podman_command(&self.data_root)?;
                cmd.arg("start").arg(&name);
                let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let combined = format!("{stderr}\n{stdout}").trim().to_string();
                    if combined.is_empty() {
                        anyhow::bail!("podman start failed for {name} (status: {})", output.status);
                    }
                    anyhow::bail!("podman start failed for {name}: {combined}");
                }
            } else {
                observe_log(
                    observer,
                    HarnessSetupPhase::ContainerCheck,
                    HarnessSetupLogLevel::Info,
                    "workspace container already running",
                );
            }
        } else {
            if readiness == ContainerReadinessState::MachineReady {
                self.ensure_container_image_ready(settings, observer)
                    .await?;
            }
            observe_phase(
                observer,
                HarnessSetupPhase::ContainerStartOrCreate,
                "creating workspace container",
            );
            let mut cmd = podman_command(&self.data_root)?;
            cmd.arg("run").arg("-d").arg("--name").arg(&name);
            if should_use_keep_id_userns() {
                cmd.arg("--userns=keep-id");
            }
            if let Some(user) = container_user() {
                cmd.arg("--user").arg(user);
            }
            cmd.arg("--network")
                .arg("slirp4netns:allow_host_loopback=true");
            cmd.arg("--cap-add").arg("NET_ADMIN");
            cmd.arg("--add-host")
                .arg("host.containers.internal:host-gateway");
            for mount in &mount_plan.mounts {
                cmd.arg("--mount").arg(mount);
            }
            cmd.arg(&image);
            cmd.arg("/bin/sh")
                .arg("-c")
                .arg("while true; do sleep 100000; done");
            let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                if combined.is_empty() {
                    anyhow::bail!("podman run failed for {name} (status: {})", output.status);
                }
                anyhow::bail!("podman run failed for {name}: {combined}");
            }
        }

        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            verify_disk_isolated_container_mounts(&self.data_root, workspace, &name).await?;
        }

        observe_phase(
            observer,
            HarnessSetupPhase::RuntimeNetworkSetup,
            "configuring container network policy",
        );
        let egress_guard = apply_container_network_policy(
            &self.data_root,
            workspace.id,
            &name,
            settings,
            daemon_host,
            daemon_port,
        )
        .await?
        .egress_guard;
        observe_log(
            observer,
            HarnessSetupPhase::RuntimeNetworkSetup,
            HarnessSetupLogLevel::Info,
            "container network policy configured",
        );
        let container = HarnessContainer {
            name: name.clone(),
            mount_mode: settings.mount_mode.clone(),
            network_mode: settings.network_mode.clone(),
            allowlist: settings.allowlist.clone(),
            external_mounts: mount_plan.external_mounts,
            egress_guard,
        };
        containers.insert(workspace.id, container.clone());
        Ok(container)
    }
}
#[cfg(test)]
// EXCEPTION: these tests intentionally serialize env-var mutations with a sync lock
// that spans async calls so process-global state cannot interleave across test cases.
#[allow(clippy::await_holding_lock)]
mod tests;
