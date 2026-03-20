use std::collections::{HashMap, HashSet};
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
    ContainerMachineMemoryProfile, ContainerMountMode, ContainerNetworkMode, ExecutionMode,
    ExecutionSettings,
};
use crate::terminals::TerminalManager;
use crate::updates;
use url::Url;

mod container;
mod image;
mod machine;
mod network_policy_transition;
mod podman;
mod podman_recovery;

#[cfg(test)]
use self::container::{bind_mount, should_mount_bundle_dir_in_container};
use self::container::{
    build_mounts, container_data_root, container_user, daemon_port_from_url,
    podman_machine_required, proxy_runtime_path, proxy_runtime_root,
    rewrite_daemon_url_for_container, should_use_keep_id_userns,
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
use self::network_policy_transition::apply_container_network_policy;
#[cfg(test)]
use self::podman::podman_binary_path;
use self::podman::{
    command_output_message, container_exists, container_running, ensure_workspace_volume,
};
pub(crate) use self::podman::{
    command_output_with_timeout, container_runtime_available, podman_command, podman_engine_ready,
    podman_invocation,
};
use self::podman_recovery::{
    ensure_podman_machine_running_with_observer, podman_machine_present,
    podman_machine_singleflight_lock, run_podman_machine_init,
};

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
}

#[derive(Debug, Clone)]
pub struct HarnessExecutionPlan {
    pub runtime: HarnessRuntimeKind,
    pub env_overrides: HashMap<String, String>,
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

#[derive(Debug, Clone, Serialize)]
pub struct HarnessRuntimeStats {
    pub container_count: usize,
    pub container_allowlist_entries: usize,
    pub container_external_mounts: usize,
    pub container_egress_guards: usize,
}

pub(crate) struct RuntimeOperationGuard<'a> {
    manager: &'a HarnessRuntimeManager,
}

impl Drop for RuntimeOperationGuard<'_> {
    fn drop(&mut self) {
        self.manager.note_runtime_activity();
        self.manager
            .active_runtime_operations
            .fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) struct PrewarmArtifactActivityGuard<'a> {
    manager: &'a HarnessRuntimeManager,
}

impl Drop for PrewarmArtifactActivityGuard<'_> {
    fn drop(&mut self) {
        self.manager.note_runtime_activity();
        self.manager
            .active_prewarm_artifact_operations
            .fetch_sub(1, Ordering::SeqCst);
    }
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

    fn note_runtime_activity(&self) {
        let mut last_activity = match self.last_activity.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        *last_activity = Instant::now();
    }

    fn runtime_idle_for(&self) -> Duration {
        let last_activity = match self.last_activity.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        last_activity.elapsed()
    }

    pub(crate) fn begin_runtime_operation(&self) -> RuntimeOperationGuard<'_> {
        self.note_runtime_activity();
        self.active_runtime_operations
            .fetch_add(1, Ordering::SeqCst);
        RuntimeOperationGuard { manager: self }
    }

    pub(crate) fn begin_prewarm_artifact_activity(&self) -> PrewarmArtifactActivityGuard<'_> {
        self.note_runtime_activity();
        self.active_prewarm_artifact_operations
            .fetch_add(1, Ordering::SeqCst);
        PrewarmArtifactActivityGuard { manager: self }
    }

    pub fn spawn_background_podman_machine_reclaim(
        self: &Arc<Self>,
        stores: StoreManager,
        running_sessions: Arc<Mutex<HashSet<SessionId>>>,
        terminals: Arc<TerminalManager>,
    ) {
        if cfg!(test) || !podman_machine_required() {
            return;
        }
        if self
            .reclaim_loop_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let mut sampler = ResourceSampler::new();
            loop {
                tokio::time::sleep(podman_machine_reclaim_poll_interval()).await;
                if let Err(err) = manager
                    .run_podman_machine_reclaim_once(
                        &stores,
                        &running_sessions,
                        terminals.as_ref(),
                        &mut sampler,
                    )
                    .await
                {
                    tracing::debug!("podman machine reclaim skipped: {err:#}");
                }
            }
        });
    }

    pub async fn stats(&self) -> HarnessRuntimeStats {
        let containers = self.containers.lock().await;
        let mut container_allowlist_entries = 0;
        let mut container_external_mounts = 0;
        let mut container_egress_guards = 0;
        for container in containers.values() {
            container_allowlist_entries += container.allowlist.len();
            container_external_mounts += container.external_mounts.len();
            if container.egress_guard {
                container_egress_guards += 1;
            }
        }
        HarnessRuntimeStats {
            container_count: containers.len(),
            container_allowlist_entries,
            container_external_mounts,
            container_egress_guards,
        }
    }

    async fn run_podman_machine_reclaim_once(
        &self,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
        sampler: &mut ResourceSampler,
    ) -> Result<()> {
        if !podman_machine_required() {
            return Ok(());
        }
        let execution = crate::settings::load_settings(stores.global())
            .await?
            .execution
            .unwrap_or_default();
        let (system, _disks, _cache_age_ms) = sampler.system_snapshot();
        let _ = self
            .maybe_reclaim_podman_machine(
                &execution.container,
                &system,
                None,
                stores,
                running_sessions,
                terminals,
            )
            .await?;
        Ok(())
    }

    pub fn spawn_background_podman_machine_download(self: &Arc<Self>) {
        if !podman_machine_required() {
            return;
        }
        // Opt-in only: initializing a Podman machine is a visible side effect (disk + network).
        // The launcher wizard should provision eagerly when the user selects container execution.
        let enabled = std::env::var("CTX_PODMAN_MACHINE_PREFETCH")
            .ok()
            .as_deref()
            .and_then(ctx_core::boolish::parse_boolish)
            .unwrap_or(false);
        if !enabled {
            return;
        }
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(err) = manager.ensure_podman_machine_download().await {
                tracing::debug!("podman machine download skipped: {err:#}");
            }
        });
    }

    async fn ensure_podman_machine_download(&self) -> Result<()> {
        if !podman_machine_required() {
            return Ok(());
        }
        ensure_managed_podman_runtime(&self.data_root, None, None).await?;
        let machine_image = if cfg!(target_os = "macos") {
            Some(ensure_managed_podman_machine_cache(&self.data_root, None, None).await?)
        } else {
            None
        };
        let machine_name = ctx_podman_machine_name(&self.data_root);
        let machine_lock = podman_machine_singleflight_lock(&machine_name);
        let _machine_guard = match machine_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => return Ok(()),
        };
        seed_shared_podman_machine_cache_best_effort(&self.data_root, None).await;
        if podman_machine_present(&self.data_root, &machine_name).await? {
            persist_podman_machine_cache_to_shared_best_effort(&self.data_root, None).await;
            return Ok(());
        }
        let init_outcome = run_podman_machine_init(
            &self.data_root,
            &machine_name,
            machine_image.as_deref(),
            Some(container_machine_memory_mb(
                &ContainerExecutionSettings::default(),
            )),
            None,
        )
        .await?;
        let output = init_outcome.output;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if init_outcome.continued_after_machine_present
            || output.status.success()
            || combined.to_ascii_lowercase().contains("already exists")
        {
            persist_podman_machine_cache_to_shared_best_effort(&self.data_root, None).await;
            return Ok(());
        }
        anyhow::bail!("podman machine init failed: {}", combined);
    }

    async fn inspect_podman_machine_memory_mb(&self, machine_name: &str) -> Result<Option<u32>> {
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("machine").arg("inspect").arg(machine_name);
        let output = command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await?;
        if !output.status.success() {
            return Ok(None);
        }
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("parsing podman machine inspect output")?;
        let machine = value
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&value);
        Ok(machine
            .get("Resources")
            .and_then(|resources| resources.get("Memory"))
            .or_else(|| {
                machine
                    .get("resources")
                    .and_then(|resources| resources.get("memory"))
            })
            .and_then(|memory| memory.as_u64())
            .and_then(|memory| u32::try_from(memory).ok()))
    }

    async fn inspect_podman_machine_state(&self, machine_name: &str) -> Result<Option<String>> {
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("machine").arg("inspect").arg(machine_name);
        let output = match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(output) => output,
            Err(err) => {
                tracing::debug!(
                    "unable to inspect local sandbox runtime state during workload probe fallback: {err:#}"
                );
                return Ok(None);
            }
        };
        if !output.status.success() {
            return Ok(None);
        }
        let value: serde_json::Value = match serde_json::from_slice(&output.stdout) {
            Ok(value) => value,
            Err(err) => {
                tracing::debug!(
                    "unable to parse local sandbox runtime inspect output during workload probe fallback: {err:#}"
                );
                return Ok(None);
            }
        };
        let machine = value
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&value);
        Ok(machine
            .get("State")
            .or_else(|| machine.get("state"))
            .and_then(|state| state.as_str())
            .map(|state| state.trim().to_ascii_lowercase()))
    }

    async fn init_podman_machine_locked(
        &self,
        machine_name: &str,
        desired_memory_mb: u32,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let machine_image = if cfg!(target_os = "macos") {
            Some(ensure_managed_podman_machine_cache(&self.data_root, observer, None).await?)
        } else {
            None
        };
        let init_outcome = run_podman_machine_init(
            &self.data_root,
            machine_name,
            machine_image.as_deref(),
            Some(desired_memory_mb),
            observer,
        )
        .await?;
        let output = init_outcome.output;
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if init_outcome.continued_after_machine_present
            || output.status.success()
            || combined.to_ascii_lowercase().contains("already exists")
        {
            persist_podman_machine_cache_to_shared_best_effort(&self.data_root, observer).await;
            return Ok(());
        }
        anyhow::bail!("podman machine init failed: {combined}");
    }

    async fn stop_podman_machine_locked(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("machine").arg("stop").arg(machine_name);
        let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
        if output.status.success() {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "stopped local sandbox runtime",
            );
            return Ok(true);
        }
        let combined = command_output_message(&output);
        let combined_lc = combined.to_ascii_lowercase();
        if combined_lc.contains("already stopped")
            || combined_lc.contains("not running")
            || combined_lc.contains("no machine")
            || combined_lc.contains("does not exist")
        {
            return Ok(false);
        }
        anyhow::bail!("podman machine stop failed: {combined}");
    }

    async fn remove_podman_machine_locked(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        let _ = self
            .stop_podman_machine_locked(machine_name, observer)
            .await;
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("machine").arg("rm").arg("-f").arg(machine_name);
        let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
        if output.status.success() {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "removed local sandbox runtime for reconfiguration",
            );
            return Ok(());
        }
        let combined = command_output_message(&output);
        let combined_lc = combined.to_ascii_lowercase();
        if combined_lc.contains("does not exist") || combined_lc.contains("no machine") {
            return Ok(());
        }
        anyhow::bail!("podman machine rm -f failed: {combined}");
    }

    async fn ensure_podman_machine_materialized(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if !podman_machine_required() {
            return Ok(());
        }
        let desired_memory_mb = container_machine_memory_mb(settings);
        let machine_name = ctx_podman_machine_name(&self.data_root);
        let machine_lock = podman_machine_singleflight_lock(&machine_name);
        let _machine_guard = machine_lock.lock().await;
        seed_shared_podman_machine_cache_best_effort(&self.data_root, observer).await;

        let present = podman_machine_present(&self.data_root, &machine_name).await?;
        if present {
            let actual_memory_mb = self.inspect_podman_machine_memory_mb(&machine_name).await?;
            if actual_memory_mb == Some(desired_memory_mb) {
                return Ok(());
            }
            if self
                .should_defer_disk_isolated_machine_reconfiguration(
                    settings,
                    &machine_name,
                    observer,
                )
                .await?
            {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "deferring local sandbox runtime memory reconfiguration because disk-isolated workspace volumes would be destroyed by machine recreation",
                );
                return Ok(());
            }
            if self
                .has_running_workspace_containers_for_stopped_machine_reconfiguration(observer)
                .await?
            {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "deferring local sandbox runtime memory reconfiguration until active workspace containers stop",
                );
                return Ok(());
            }
            let detail = actual_memory_mb
                .map(|value| format!("{value} MiB"))
                .unwrap_or_else(|| "unknown".to_string());
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                &format!(
                    "reconfiguring local sandbox runtime memory from {detail} to {} MiB",
                    desired_memory_mb
                ),
            );
            self.remove_podman_machine_locked(&machine_name, observer)
                .await?;
        }

        self.init_podman_machine_locked(&machine_name, desired_memory_mb, observer)
            .await
    }

    async fn reconcile_running_podman_machine_memory(
        &self,
        settings: &ContainerExecutionSettings,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        if !podman_machine_required() {
            return Ok(());
        }
        let desired_memory_mb = container_machine_memory_mb(settings);
        let machine_name = ctx_podman_machine_name(&self.data_root);
        let machine_lock = podman_machine_singleflight_lock(&machine_name);
        let _machine_guard = machine_lock.lock().await;

        if !podman_machine_present(&self.data_root, &machine_name).await? {
            observe_log(
                observer,
                HarnessSetupPhase::MachineCheck,
                HarnessSetupLogLevel::Warn,
                "local sandbox runtime is reachable but machine state could not be inspected; leaving memory profile unchanged",
            );
            return Ok(());
        }

        let actual_memory_mb = self.inspect_podman_machine_memory_mb(&machine_name).await?;
        if actual_memory_mb == Some(desired_memory_mb) {
            return Ok(());
        }
        if self
            .should_defer_disk_isolated_machine_reconfiguration(settings, &machine_name, observer)
            .await?
        {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "deferring local sandbox runtime memory reconfiguration because disk-isolated workspace volumes would be destroyed by machine recreation",
            );
            return Ok(());
        }
        if self
            .has_running_workspace_containers_for_stopped_machine_reconfiguration(observer)
            .await?
        {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "deferring local sandbox runtime memory reconfiguration until active workspace containers stop",
            );
            return Ok(());
        }
        let detail = actual_memory_mb
            .map(|value| format!("{value} MiB"))
            .unwrap_or_else(|| "unknown".to_string());
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "reconfiguring local sandbox runtime memory from {detail} to {} MiB",
                desired_memory_mb
            ),
        );
        self.remove_podman_machine_locked(&machine_name, observer)
            .await?;
        self.init_podman_machine_locked(&machine_name, desired_memory_mb, observer)
            .await
    }

    async fn has_running_workspace_containers(&self) -> Result<bool> {
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("ps").arg("--format").arg("{{.Names}}");
        let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
        if !output.status.success() {
            anyhow::bail!("podman ps failed: {}", command_output_message(&output));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout
            .lines()
            .map(str::trim)
            .any(|name| name.starts_with("ctx-harness-")))
    }

    async fn should_defer_disk_isolated_machine_reconfiguration(
        &self,
        settings: &ContainerExecutionSettings,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if !matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            return Ok(false);
        }
        self.disk_isolated_workspace_volumes_exist(machine_name, observer)
            .await
    }

    async fn disk_isolated_workspace_volumes_exist(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if !self
            .ensure_engine_ready_for_disk_state_inspection(machine_name, observer)
            .await?
        {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                "unable to verify disk-isolated workspace volumes before memory reconfiguration; leaving local sandbox runtime unchanged",
            );
            return Ok(true);
        }

        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("volume").arg("ls").arg("--format").arg("{{.Name}}");
        let output = match command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await {
            Ok(output) => output,
            Err(err) => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "failed to inspect disk-isolated workspace volumes before memory reconfiguration: {err}"
                    ),
                );
                return Ok(true);
            }
        };
        if !output.status.success() {
            let detail = command_output_message(&output);
            let suffix = if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            };
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!(
                    "unable to inspect disk-isolated workspace volumes before memory reconfiguration{suffix}"
                ),
            );
            return Ok(true);
        }

        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .any(|name| name.starts_with("ctx-ws-")))
    }

    async fn ensure_engine_ready_for_disk_state_inspection(
        &self,
        machine_name: &str,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        if podman_engine_ready(&self.data_root).await? {
            return Ok(true);
        }

        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "starting local sandbox runtime to inspect disk-isolated workspace volumes before memory reconfiguration",
        );
        let mut start = podman_command(&self.data_root)?;
        start.arg("machine").arg("start").arg(machine_name);
        let output = command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await?;
        if !output.status.success() {
            return Ok(false);
        }

        let deadline = Instant::now() + podman_machine_ready_timeout();
        loop {
            if podman_engine_ready(&self.data_root).await? {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(podman_machine_ready_poll_interval()).await;
        }
    }

    async fn has_running_workspace_containers_for_stopped_machine_reconfiguration(
        &self,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<bool> {
        match self.has_running_workspace_containers().await {
            Ok(has_running) => Ok(has_running),
            Err(err) => {
                if podman_engine_ready(&self.data_root).await.unwrap_or(false) {
                    return Err(err);
                }
                let machine_name = ctx_podman_machine_name(&self.data_root);
                match self.inspect_podman_machine_state(&machine_name).await? {
                    Some(state) if state.contains("running") || state.contains("starting") => {
                        observe_log(
                            observer,
                            HarnessSetupPhase::MachineStartOrInit,
                            HarnessSetupLogLevel::Warn,
                            "local sandbox runtime appears to be running but unreachable; deferring memory reconfiguration until workload probes recover",
                        );
                        tracing::debug!(
                            "treating workspace container probe failure as busy because the local sandbox runtime still reports a running state: {err:#}"
                        );
                        return Ok(true);
                    }
                    Some(_) => {}
                    None => {
                        observe_log(
                            observer,
                            HarnessSetupPhase::MachineStartOrInit,
                            HarnessSetupLogLevel::Warn,
                            "local sandbox runtime state is unknown while workload probes are unreachable; deferring memory reconfiguration until the runtime can be inspected safely",
                        );
                        tracing::debug!(
                            "treating workspace container probe failure as busy because the local sandbox runtime state is unknown: {err:#}"
                        );
                        return Ok(true);
                    }
                }
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    "local sandbox runtime is not reachable; continuing memory reconfiguration without workload probe",
                );
                tracing::debug!(
                    "treating workspace container probe failure as idle because podman engine is not reachable: {err:#}"
                );
                Ok(false)
            }
        }
    }

    async fn maybe_reclaim_podman_machine(
        &self,
        settings: &ContainerExecutionSettings,
        system: &SystemSnapshot,
        observer: Option<&dyn HarnessSetupObserver>,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
    ) -> Result<bool> {
        if !podman_machine_required() {
            return Ok(false);
        }
        if self.active_runtime_operations.load(Ordering::SeqCst) > 0
            || self
                .active_prewarm_artifact_operations
                .load(Ordering::SeqCst)
                > 0
        {
            return Ok(false);
        }
        if self
            .should_defer_reclaim_for_active_container_runtime(stores, running_sessions, terminals)
            .await
        {
            self.note_runtime_activity();
            return Ok(false);
        }
        let idle_for = self.runtime_idle_for();
        let idle_timeout = Duration::from_secs(normalize_container_machine_idle_shutdown_seconds(
            settings.machine.idle_shutdown_seconds,
        ));
        let swap_threshold_bytes =
            u64::from(settings.machine.host_pressure_swap_threshold_mb) * 1024 * 1024;
        let host_pressure =
            swap_threshold_bytes > 0 && system.swap_used_bytes >= swap_threshold_bytes;
        let should_stop = idle_for >= idle_timeout
            || (host_pressure && idle_for >= podman_machine_pressure_idle_grace());
        if !should_stop {
            return Ok(false);
        }

        let machine_name = ctx_podman_machine_name(&self.data_root);
        let machine_lock = podman_machine_singleflight_lock(&machine_name);
        let _machine_guard = machine_lock.lock().await;
        if self.active_runtime_operations.load(Ordering::SeqCst) > 0
            || self
                .active_prewarm_artifact_operations
                .load(Ordering::SeqCst)
                > 0
        {
            return Ok(false);
        }
        if self
            .should_defer_reclaim_for_active_container_runtime(stores, running_sessions, terminals)
            .await
        {
            self.note_runtime_activity();
            return Ok(false);
        }
        if !podman_machine_present(&self.data_root, &machine_name).await? {
            return Ok(false);
        }
        let stopped = self
            .stop_podman_machine_locked(&machine_name, observer)
            .await?;
        if stopped {
            self.note_runtime_activity();
        }
        Ok(stopped)
    }

    async fn should_defer_reclaim_for_active_container_runtime(
        &self,
        stores: &StoreManager,
        running_sessions: &Arc<Mutex<HashSet<SessionId>>>,
        terminals: &TerminalManager,
    ) -> bool {
        if terminals.has_running_container_backed().await {
            return true;
        }

        let session_ids = {
            let running = running_sessions.lock().await;
            running.iter().copied().collect::<Vec<_>>()
        };

        for session_id in session_ids {
            let store = match stores.store_for_session(session_id).await {
                Ok(store) => store,
                Err(err) => {
                    tracing::warn!(
                        session_id = ?session_id,
                        "deferring local sandbox reclaim because running session store lookup failed: {err:#}"
                    );
                    return true;
                }
            };
            let session = match store.get_session(session_id).await {
                Ok(Some(session)) => session,
                Ok(None) => continue,
                Err(err) => {
                    tracing::warn!(
                        session_id = ?session_id,
                        "deferring local sandbox reclaim because running session lookup failed: {err:#}"
                    );
                    return true;
                }
            };
            if matches!(
                session.execution_environment,
                ExecutionEnvironment::ContainerHostMounted
                    | ExecutionEnvironment::ContainerDiskIsolated
            ) {
                return true;
            }
        }

        false
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
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::Host,
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
            .context("podman unavailable and execution mode is container")?;
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
        if !container_exists(&self.data_root, &name).await? {
            return Ok(None);
        }
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
        Ok(Some(HarnessContainerStatus {
            name,
            running,
            known,
            mount_mode,
            network_mode,
            allowlist,
            egress_guard,
        }))
    }

    pub async fn stop_container(&self, workspace_id: WorkspaceId) -> Result<bool> {
        let _activity = self.begin_runtime_operation();
        let name = format!("ctx-harness-{}", workspace_id.0);
        if !container_exists(&self.data_root, &name).await? {
            return Ok(false);
        }
        let mut containers = self.containers.lock().await;
        containers.remove(&workspace_id);
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("rm").arg("-f").arg(&name);
        let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
        if output.status.success() {
            Ok(true)
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
mod reclaim_unit_tests {
    use super::*;
    use std::time::{Duration, Instant};

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

    fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::podman_env_test_lock()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn idle_runtime_reclaim_stops_machine_even_with_running_ctx_harness_container() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let manager = HarnessRuntimeManager::new(temp.path().to_path_buf());
        let machine_name = ctx_podman_machine_name(temp.path());
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"ps\" ]; then\n  printf 'ctx-harness-123\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        {
            let mut last_activity = manager
                .last_activity
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *last_activity = Instant::now() - Duration::from_secs(600);
        }
        let settings = ContainerExecutionSettings {
            machine: crate::settings::ContainerMachineSettings {
                idle_shutdown_seconds: 60,
                ..crate::settings::ContainerMachineSettings::default()
            },
            ..ContainerExecutionSettings::default()
        };
        let stores = StoreManager::open(temp.path()).await.expect("open stores");
        let running_sessions = Arc::new(Mutex::new(HashSet::new()));
        let terminals = TerminalManager::default();
        let snapshot = SystemSnapshot {
            cpu_pct: 0.0,
            memory_total_bytes: 32 * 1024 * 1024 * 1024,
            memory_used_bytes: 8 * 1024 * 1024 * 1024,
            swap_total_bytes: 4 * 1024 * 1024 * 1024,
            swap_used_bytes: 0,
        };

        let stopped = manager
            .maybe_reclaim_podman_machine(
                &settings,
                &snapshot,
                None,
                &stores,
                &running_sessions,
                &terminals,
            )
            .await
            .expect("idle reclaim should not be blocked by parked workspace containers");

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert!(stopped);
        assert!(log.contains(&format!("machine stop {machine_name}")));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn active_prewarm_artifact_activity_suppresses_reclaim() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let manager = HarnessRuntimeManager::new(temp.path().to_path_buf());
        let machine_name = ctx_podman_machine_name(temp.path());
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        {
            let mut last_activity = manager
                .last_activity
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *last_activity = Instant::now() - Duration::from_secs(600);
        }
        let settings = ContainerExecutionSettings {
            machine: crate::settings::ContainerMachineSettings {
                idle_shutdown_seconds: 60,
                ..crate::settings::ContainerMachineSettings::default()
            },
            ..ContainerExecutionSettings::default()
        };
        let stores = StoreManager::open(temp.path()).await.expect("open stores");
        let running_sessions = Arc::new(Mutex::new(HashSet::new()));
        let terminals = TerminalManager::default();
        let snapshot = SystemSnapshot {
            cpu_pct: 0.0,
            memory_total_bytes: 32 * 1024 * 1024 * 1024,
            memory_used_bytes: 8 * 1024 * 1024 * 1024,
            swap_total_bytes: 4 * 1024 * 1024 * 1024,
            swap_used_bytes: 0,
        };
        let _prewarm_activity = manager.begin_prewarm_artifact_activity();

        let stopped = manager
            .maybe_reclaim_podman_machine(
                &settings,
                &snapshot,
                None,
                &stores,
                &running_sessions,
                &terminals,
            )
            .await
            .expect("active prewarm should suppress reclaim");

        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(!stopped);
        assert!(!log.contains(&format!("machine stop {machine_name}")));
    }
}

#[cfg(test)]
// EXCEPTION: these tests intentionally serialize env-var mutations with a sync lock
// that spans async calls so process-global state cannot interleave across test cases.
#[allow(clippy::await_holding_lock)]
mod tests;
