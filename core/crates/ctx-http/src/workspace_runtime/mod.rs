use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use sha2::Digest;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::{fs, io::AsyncWriteExt};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::worktrees_root;
use serde::{Deserialize, Serialize};

use crate::bundled_assets;
use crate::network_allowlist;
use crate::settings::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ExecutionMode,
    ExecutionSettings,
};
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
pub(crate) use self::image::prefetch_container_startup_artifacts_with_overrides;
pub(crate) use self::image::resolve_container_image;
pub use self::image::{
    bundled_default_container_image_tar, container_image_present, container_image_status,
    default_container_image, is_default_container_image, prefetch_container_image,
    prefetch_container_image_with_observer, ContainerImageStatus,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct HarnessRuntimeStats {
    pub container_count: usize,
    pub container_allowlist_entries: usize,
    pub container_external_mounts: usize,
    pub container_egress_guards: usize,
}

impl HarnessRuntimeManager {
    pub fn new(data_root: PathBuf) -> Self {
        Self {
            data_root,
            containers: Mutex::new(HashMap::new()),
        }
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

    pub fn spawn_background_podman_machine_download(self: &Arc<Self>) {
        if !podman_machine_required() {
            return;
        }
        // Opt-in only: initializing a Podman machine is a visible side effect (disk + network).
        // The launcher wizard should provision eagerly when the user selects container execution.
        let enabled = std::env::var("CTX_PODMAN_MACHINE_PREFETCH")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
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
        if podman_machine_present(&self.data_root).await? {
            persist_podman_machine_cache_to_shared_best_effort(&self.data_root, None).await;
            return Ok(());
        }
        let init_outcome = run_podman_machine_init(
            &self.data_root,
            &machine_name,
            machine_image.as_deref(),
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
        self.ensure_container_machine_ready(observer)
            .await
            .context("podman unavailable and execution mode is container")?;
        self.ensure_workspace_container_after_machine_ready_with_observer(
            workspace,
            settings,
            daemon_url,
            observer,
            ContainerReadinessState::MachineReady,
        )
        .await
    }

    async fn ensure_workspace_container_after_machine_ready_with_observer(
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
        self.ensure_workspace_container_after_machine_ready_with_observer(
            workspace,
            settings,
            daemon_url,
            observer,
            ContainerReadinessState::RuntimeReady,
        )
        .await
    }

    async fn ensure_container_machine_ready(
        &self,
        observer: Option<&dyn HarnessSetupObserver>,
    ) -> Result<()> {
        observe_phase(
            observer,
            HarnessSetupPhase::MachineCheck,
            "checking container runtime",
        );
        ensure_managed_podman_runtime(&self.data_root, observer, None).await?;
        ensure_podman_machine_running_with_observer(&self.data_root, observer).await?;
        Ok(())
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
            "loading harness image into podman",
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
        self.ensure_container_machine_ready(observer).await?;
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
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use chrono::Utc;
    use ctx_core::ids::{WorkspaceId, WorktreeId};
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, Duration};

    use super::network_policy_transition::transparent_proxy_policy;
    use super::podman_recovery::{
        collect_ctx_managed_podman_helper_pids,
        collect_ctx_managed_podman_helper_pids_from_ps_output, initialize_podman_machine,
        is_ctx_managed_podman_helper_process_command, kill_ctx_managed_podman_helper_processes,
        literal_pkill_pattern, looks_like_missing_machine_error,
        looks_like_recoverable_machine_start_error,
        looks_like_running_but_unreachable_machine_start_error, podman_machine_temp_state_paths,
    };

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
            if let Some(value) = self.prev.take() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
        crate::test_support::podman_env_test_lock()
    }

    fn sample_workspace(tmp: &TempDir) -> Workspace {
        Workspace {
            id: WorkspaceId::new(),
            name: "ws".to_string(),
            root_path: tmp.path().to_string_lossy().to_string(),
            created_at: Utc::now(),
            vcs_kind: None,
        }
    }

    fn sample_worktree(tmp: &TempDir, workspace_id: WorkspaceId) -> Worktree {
        Worktree {
            id: WorktreeId::new(),
            workspace_id,
            root_path: tmp.path().to_string_lossy().to_string(),
            base_commit_sha: "deadbeef".to_string(),
            git_branch: Some("main".to_string()),
            vcs_kind: None,
            base_revision: None,
            vcs_ref: None,
            created_at: Utc::now(),
            bootstrap_status: None,
            bootstrap_started_at: None,
            bootstrap_finished_at: None,
            bootstrap_exit_code: None,
            bootstrap_timeout_sec: None,
            bootstrap_error: None,
            bootstrap_log_path: None,
            bootstrap_log_truncated: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        }
    }

    async fn runtime_manager(tmp: &TempDir) -> HarnessRuntimeManager {
        HarnessRuntimeManager::new(tmp.path().to_path_buf())
    }

    fn sample_cached_container() -> HarnessContainer {
        let mut external_mounts = HashSet::new();
        external_mounts.insert("/tmp/external".to_string());
        HarnessContainer {
            name: "ctx-harness-sample".to_string(),
            mount_mode: ContainerMountMode::HostMounted,
            network_mode: ContainerNetworkMode::Allowlist,
            allowlist: vec!["github.com".to_string()],
            external_mounts,
            egress_guard: true,
        }
    }

    fn sample_container_settings() -> ContainerExecutionSettings {
        ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::Podman,
            mount_mode: ContainerMountMode::HostMounted,
            network_mode: ContainerNetworkMode::Allowlist,
            allowlist: vec!["github.com".to_string()],
            image: None,
        }
    }

    async fn spawn_static_http_server(body: Vec<u8>) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local http listener");
        let addr = listener.local_addr().expect("listener local addr");
        let shared = Arc::new(body);
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                let payload = Arc::clone(&shared);
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
        (format!("http://{addr}/image.tar"), task)
    }

    async fn install_test_managed_machine_cache_source(
        body: Vec<u8>,
    ) -> (
        crate::bundled_assets::TestManagedPodmanMachineCacheSourceGuard,
        JoinHandle<()>,
    ) {
        let digest = {
            let mut hasher = Sha256::new();
            hasher.update(&body);
            hex::encode(hasher.finalize())
        };
        let (url, server) = spawn_static_http_server(body).await;
        let guard = crate::bundled_assets::override_managed_podman_machine_cache_source_for_test(
            bundled_assets::ManagedArtifactSource {
                uri: url,
                sha256: digest,
            },
        );
        (guard, server)
    }

    #[tokio::test]
    async fn container_mode_errors_when_podman_unavailable() {
        let _serial = env_var_test_lock().lock().await;
        let _guard = EnvGuard::set("CTX_TEST_PODMAN_AVAILABLE", "0");
        let tmp = tempfile::tempdir().unwrap();
        let manager = runtime_manager(&tmp).await;
        let workspace = sample_workspace(&tmp);
        let worktree = sample_worktree(&tmp, workspace.id);
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            container: ContainerExecutionSettings::default(),
        };

        let err = manager
            .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:9999")
            .await
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("podman unavailable"));
        assert!(message.contains("execution mode is container"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepare_reuses_running_workspace_container_without_front_loading_image_readiness() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        let manager = runtime_manager(&temp).await;
        let workspace = sample_workspace(&temp);
        let worktree = sample_worktree(&temp, workspace.id);
        let container_name = workspace_container_name(workspace.id);
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            container: ContainerExecutionSettings {
                network_mode: ContainerNetworkMode::All,
                ..Default::default()
            },
        };

        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"exists\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"rm\" ] && [ \"$2\" = \"-f\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

        let plan = manager
            .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
            .await
            .expect("running workspace container should be reused without image checks");

        match plan.runtime {
            HarnessRuntimeKind::Container { name } => assert_eq!(name, container_name),
            HarnessRuntimeKind::Host => panic!("expected container runtime"),
        }

        let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
        assert!(
            log.contains(&format!("container exists {container_name}")),
            "expected running container existence check in log:\n{log}"
        );
        assert!(
            log.contains(&format!(
                "container inspect --format {{{{.State.Running}}}} {container_name}"
            )),
            "expected running-container inspect in log:\n{log}"
        );
        assert!(
            !log.contains("image exists"),
            "running-container reuse should not front-load image checks:\n{log}"
        );
        assert!(
            !log.contains("run -d --name"),
            "running-container reuse should not recreate the container:\n{log}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepare_starts_cached_workspace_container_when_podman_reports_it_stopped() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        let manager = runtime_manager(&temp).await;
        let workspace = sample_workspace(&temp);
        let worktree = sample_worktree(&temp, workspace.id);
        let container_name = workspace_container_name(workspace.id);
        let settings = ExecutionSettings {
            mode: ExecutionMode::Container,
            container: ContainerExecutionSettings {
                network_mode: ContainerNetworkMode::All,
                allowlist: Vec::new(),
                ..Default::default()
            },
        };
        let mount_plan = build_mounts(
            temp.path(),
            &workspace,
            Some(&worktree),
            &settings.container,
        );

        manager.containers.lock().await.insert(
            workspace.id,
            HarnessContainer {
                name: container_name.clone(),
                mount_mode: settings.container.mount_mode.clone(),
                network_mode: settings.container.network_mode.clone(),
                allowlist: settings.container.allowlist.clone(),
                external_mounts: mount_plan.external_mounts,
                egress_guard: false,
            },
        );

        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'false\\n'\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

        let plan = manager
            .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
            .await
            .expect("stopped cached workspace container should be restarted");

        match plan.runtime {
            HarnessRuntimeKind::Container { name } => assert_eq!(name, container_name),
            HarnessRuntimeKind::Host => panic!("expected container runtime"),
        }

        let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
        assert!(
            log.contains(&format!("container exists {container_name}")),
            "expected container existence check in log:\n{log}"
        );
        assert!(
            log.contains(&format!(
                "container inspect --format {{{{.State.Running}}}} {container_name}"
            )),
            "expected stopped-container inspect in log:\n{log}"
        );
        assert!(
            log.contains(&format!("start {container_name}")),
            "expected stopped cached container to be started:\n{log}"
        );
        assert!(
            !log.contains("image exists"),
            "starting a stopped cached container should not front-load image checks:\n{log}"
        );
        assert!(
            !log.contains("run -d --name"),
            "starting a stopped cached container should not recreate the container:\n{log}"
        );
    }

    #[test]
    fn podman_binary_path_uses_env_override() {
        let _serial = env_var_test_lock().blocking_lock();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &path);
        let resolved = podman_binary_path(Path::new("/tmp")).expect("env override should resolve");
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn podman_invocation_sets_paths_under_short_runtime_root() {
        let _serial = env_var_test_lock().blocking_lock();
        let tmp = tempfile::tempdir().expect("tempdir");
        let podman_bin = tempfile::NamedTempFile::new().expect("podman bin");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_bin.path().to_string_lossy());
        let inv = podman_invocation(tmp.path()).expect("podman invocation");
        let runtime_dir = inv
            .env
            .get("XDG_RUNTIME_DIR")
            .cloned()
            .expect("runtime dir env");
        let home = inv.env.get("HOME").cloned().expect("home env");
        let tmpdir = inv.env.get("TMPDIR").cloned().expect("tmpdir env");
        assert_eq!(
            PathBuf::from(runtime_dir),
            podman_runtime_root(tmp.path()).join("run")
        );
        assert_eq!(PathBuf::from(home), podman_home_root(tmp.path()));
        assert_eq!(PathBuf::from(tmpdir), podman_temp_root(tmp.path()));
    }

    #[test]
    fn keep_id_userns_is_only_enabled_on_linux() {
        assert_eq!(should_use_keep_id_userns(), cfg!(target_os = "linux"));
    }

    #[tokio::test]
    async fn seed_shared_podman_machine_cache_populates_local_cache_root() {
        let _serial = env_var_test_lock().lock().await;
        let shared = tempfile::tempdir().expect("shared tempdir");
        let data_root = tempfile::tempdir().expect("data root tempdir");
        let _guard = EnvGuard::set(
            "CTX_PODMAN_MACHINE_CACHE_DIR",
            &shared.path().to_string_lossy(),
        );
        let relpath = PathBuf::from("applehv")
            .join("cache")
            .join("78e5fea350d7.raw.zst");
        let shared_file = shared.path().join(&relpath);
        std::fs::create_dir_all(shared_file.parent().expect("shared cache parent"))
            .expect("create shared cache dir");
        std::fs::write(&shared_file, b"seeded-machine-cache").expect("write shared cache file");

        seed_shared_podman_machine_cache(data_root.path(), None)
            .await
            .expect("seed shared cache");

        let local_file = podman_machine_cache_root(data_root.path()).join(relpath);
        let local_body = std::fs::read(&local_file).expect("read local cache file");
        assert_eq!(local_body, b"seeded-machine-cache");
    }

    #[tokio::test]
    async fn persist_podman_machine_cache_to_shared_does_not_depend_on_local_path() {
        let _serial = env_var_test_lock().lock().await;
        let shared = tempfile::tempdir().expect("shared tempdir");
        let data_root = tempfile::tempdir().expect("data root tempdir");
        let _guard = EnvGuard::set(
            "CTX_PODMAN_MACHINE_CACHE_DIR",
            &shared.path().to_string_lossy(),
        );
        let relpath = PathBuf::from("applehv")
            .join("cache")
            .join("persisted-machine-cache.raw.zst");
        let local_file = podman_machine_cache_root(data_root.path()).join(&relpath);
        std::fs::create_dir_all(local_file.parent().expect("local cache parent"))
            .expect("create local cache dir");
        std::fs::write(&local_file, b"persisted-machine-cache").expect("write local cache file");

        persist_podman_machine_cache_to_shared(data_root.path(), None)
            .await
            .expect("persist shared cache");

        std::fs::remove_file(&local_file).expect("remove local cache file");
        let shared_file = shared.path().join(relpath);
        let shared_body = std::fs::read(&shared_file).expect("read shared cache file");
        assert_eq!(shared_body, b"persisted-machine-cache");
    }

    #[tokio::test]
    async fn ensure_podman_machine_download_skips_when_machine_lock_is_busy() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        let (_machine_cache_guard, machine_cache_server) =
            install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

        let manager = HarnessRuntimeManager::new(temp.path().to_path_buf());
        let machine_name = ctx_podman_machine_name(temp.path());
        let machine_lock = podman_machine_singleflight_lock(&machine_name);
        let _machine_guard = machine_lock.lock().await;

        manager
            .ensure_podman_machine_download()
            .await
            .expect("prefetch should skip while launch holds the machine lock");

        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        assert!(log.trim().is_empty());
        machine_cache_server.abort();
    }

    #[test]
    fn missing_machine_error_detection_matches_expected_shapes() {
        assert!(looks_like_missing_machine_error(
            "error: no machine with this name exists"
        ));
        assert!(looks_like_missing_machine_error(
            "Error: machine ctx not found"
        ));
        assert!(!looks_like_missing_machine_error(
            "error: machine already running"
        ));
    }

    #[test]
    fn recoverable_machine_start_error_detection_matches_expected_shapes() {
        assert!(looks_like_recoverable_machine_start_error(
            "error: machine is already starting"
        ));
        assert!(looks_like_recoverable_machine_start_error(
            "Error: unable to start \"ctx\": already running\nStarting machine \"ctx\""
        ));
        assert!(looks_like_recoverable_machine_start_error(
            "error: resource busy while acquiring lock"
        ));
        assert!(looks_like_recoverable_machine_start_error(
            "error: operation timed out while waiting for vm startup"
        ));
        assert!(looks_like_recoverable_machine_start_error(
            "time=\"2026-03-05T00:23:28-06:00\" level=warning msg=\"detected port conflict on machine ssh port [49401], reassigning\"\nError: vfkit exited unexpectedly with exit code 1"
        ));
        assert!(looks_like_recoverable_machine_start_error(
            "Error: unable to connect to \"gvproxy\" socket at \"/tmp/podman.sock\""
        ));
        assert!(!looks_like_recoverable_machine_start_error(
            "error: unknown vm provider configuration"
        ));
    }

    #[test]
    fn running_but_unreachable_machine_start_error_detection_matches_expected_shapes() {
        assert!(looks_like_running_but_unreachable_machine_start_error(
            "Error: unable to start \"ctx\": already running"
        ));
        assert!(looks_like_running_but_unreachable_machine_start_error(
            "Error: unable to connect to \"gvproxy\" socket at \"/tmp/podman.sock\""
        ));
        assert!(!looks_like_running_but_unreachable_machine_start_error(
            "error: resource busy while acquiring lock"
        ));
        assert!(!looks_like_running_but_unreachable_machine_start_error(
            "error: operation timed out while waiting for vm startup"
        ));
    }

    #[test]
    fn collect_ctx_managed_podman_helper_pids_matches_only_ctx_scoped_helpers() {
        let temp = tempfile::tempdir().expect("tempdir");
        let machine_name = ctx_podman_machine_name(temp.path());
        let helper_dir = temp
            .path()
            .join("managed")
            .join("runtimes")
            .join("podman")
            .join("macos")
            .join("aarch64")
            .join("podman-5.8.0")
            .join("usr")
            .join("libexec")
            .join("podman");
        let matches = collect_ctx_managed_podman_helper_pids(
            vec![
                (
                    42,
                    vec![
                        helper_dir.join("gvproxy").to_string_lossy().into_owned(),
                        machine_name.clone(),
                        podman_temp_root(temp.path())
                            .join("podman")
                            .join(format!("{machine_name}-api.sock"))
                            .to_string_lossy()
                            .into_owned(),
                    ],
                ),
                (
                    77,
                    vec![
                        "/opt/homebrew/bin/vfkit".to_string(),
                        temp.path()
                            .join("podman")
                            .join("xdg")
                            .join("data")
                            .join("containers")
                            .join("podman")
                            .join("machine")
                            .join("applehv")
                            .join(format!("{machine_name}-arm64.raw"))
                            .to_string_lossy()
                            .into_owned(),
                        machine_name.clone(),
                    ],
                ),
                (
                    88,
                    vec![
                        "/opt/homebrew/libexec/podman/gvproxy".to_string(),
                        "/tmp/podman/podman-machine-default-api.sock".to_string(),
                        "podman-machine-default".to_string(),
                    ],
                ),
            ],
            temp.path(),
            &machine_name,
        );
        assert_eq!(matches, vec![42, 77]);
    }

    #[test]
    fn ctx_managed_podman_helper_process_detection_matches_expected_shapes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let machine_name = ctx_podman_machine_name(temp.path());
        let matching_gvproxy = vec![
            temp.path()
                .join("managed")
                .join("runtimes")
                .join("podman")
                .join("macos")
                .join("aarch64")
                .join("podman-5.8.0")
                .join("usr")
                .join("libexec")
                .join("podman")
                .join("gvproxy")
                .to_string_lossy()
                .into_owned(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-api.sock"))
                .to_string_lossy()
                .into_owned(),
            machine_name.clone(),
        ];
        assert!(is_ctx_managed_podman_helper_process_command(
            &matching_gvproxy,
            temp.path(),
            &machine_name
        ));

        let matching_vfkit = vec![
            String::from("/opt/homebrew/bin/vfkit"),
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("applehv")
                .join(format!("{machine_name}-arm64.raw"))
                .to_string_lossy()
                .into_owned(),
            machine_name.clone(),
        ];
        assert!(is_ctx_managed_podman_helper_process_command(
            &matching_vfkit,
            temp.path(),
            &machine_name
        ));

        let wrong_machine = vec![
            String::from("/opt/homebrew/bin/vfkit"),
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("applehv")
                .join("ctx-someone-else-arm64.raw")
                .to_string_lossy()
                .into_owned(),
        ];
        assert!(!is_ctx_managed_podman_helper_process_command(
            &wrong_machine,
            temp.path(),
            &machine_name
        ));

        let host_helper = vec![
            String::from("/opt/homebrew/libexec/podman/gvproxy"),
            String::from("/tmp/podman/podman-machine-default-api.sock"),
            String::from("podman-machine-default"),
        ];
        assert!(!is_ctx_managed_podman_helper_process_command(
            &host_helper,
            temp.path(),
            &machine_name
        ));
    }

    #[test]
    fn collect_ctx_managed_podman_helper_pids_from_ps_output_matches_real_macos_shapes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let machine_name = ctx_podman_machine_name(temp.path());
        let helper_dir = temp
            .path()
            .join("managed")
            .join("runtimes")
            .join("podman")
            .join("macos")
            .join("aarch64")
            .join("podman-5.8.0")
            .join("usr")
            .join("libexec")
            .join("podman");
        let gvproxy_line = format!(
            " 6622 {} -mtu 1500 -listen-vfkit unixgram://{} -forward-sock {} -forward-identity {} -pid-file {}/gvproxy.pid",
            helper_dir.join("gvproxy").display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("machine")
                .display(),
            podman_temp_root(temp.path()).join("podman").display(),
        );
        let vfkit_line = format!(
            "12484 /Users/example-user/Library/Application Support/vfkit --device virtio-blk,path={} --device virtio-vsock,port=1025,socketURL={} --device virtio-net,unixSocketPath={}",
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("applehv")
                .join(format!("{machine_name}-arm64.raw"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}.sock"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
        );
        let host_line = String::from(
            "88 /opt/homebrew/libexec/podman/gvproxy -forward-sock /tmp/podman/podman-machine-default-api.sock podman-machine-default",
        );
        let ps_output = format!("{gvproxy_line}\n{vfkit_line}\n{host_line}\n");

        let matches = collect_ctx_managed_podman_helper_pids_from_ps_output(
            &ps_output,
            temp.path(),
            &machine_name,
        );
        assert_eq!(matches, vec![6622, 12484]);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn ensure_podman_machine_running_recreates_immediately_for_already_running_unreachable_machine(
    ) {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let state_path = temp.path().join("podman-ready");
        let start_count_path = temp.path().join("podman-start-count");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nSTART_COUNT=\"{start_count}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  count=0\n  if [ -f \"$START_COUNT\" ]; then\n    count=$(cat \"$START_COUNT\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$START_COUNT\"\n  if [ \"$count\" -eq 1 ]; then\n    echo 'Error: unable to start \"ctx\": already running' >&2\n    exit 125\n  fi\n  touch \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nexit 0\n",
                log = log_path.display(),
                state = state_path.display(),
                start_count = start_count_path.display(),
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        let (_machine_cache_guard, machine_cache_server) =
            install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

        ensure_podman_machine_running_with_observer(temp.path(), None)
            .await
            .expect("already-running unreachable machine should recover");

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert!(log.contains("info"));
        assert!(log.contains("machine start "));
        assert!(log.contains("machine inspect "));
        assert!(log.contains("machine rm -f "));
        assert!(log.contains("machine init "));
        assert!(!log.contains("machine stop "));
        machine_cache_server.abort();
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn ensure_podman_machine_running_fails_fast_on_unknown_start_error() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  echo 'error: unknown vm provider configuration' >&2\n  exit 125\nfi\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

        let err = ensure_podman_machine_running_with_observer(temp.path(), None)
            .await
            .expect_err("unknown start error should fail");
        let message = format!("{err:#}");
        assert!(message.contains("unknown vm provider configuration"));

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert!(log.contains("machine start "));
        assert!(!log.contains("machine stop "));
        assert!(!log.contains("machine rm -f "));
    }

    #[tokio::test]
    async fn initialize_podman_machine_uses_init_then_start_without_now() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        let (_machine_cache_guard, machine_cache_server) =
            install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

        let mut last_err = String::new();
        initialize_podman_machine(temp.path(), "ctx-test-machine", None, &mut last_err)
            .await
            .expect("initialize machine");

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert!(log.contains("machine init ctx-test-machine"));
        assert!(log.contains("machine start ctx-test-machine"));
        assert!(!log.contains("--now"));
        assert!(last_err.is_empty());
        machine_cache_server.abort();
    }

    #[tokio::test]
    async fn initialize_podman_machine_terminates_stuck_init_when_machine_is_present() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exec sleep 30\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
        let (_machine_cache_guard, machine_cache_server) =
            install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

        let mut last_err = String::new();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            initialize_podman_machine(temp.path(), "ctx-test-machine", None, &mut last_err),
        )
        .await;
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let init_result = result.unwrap_or_else(|_| {
            panic!("initialize_podman_machine timed out; invocation log:\n{log}")
        });
        init_result.expect("initialize machine");

        assert!(log.contains("machine init ctx-test-machine"));
        assert!(log.contains("machine inspect "));
        assert!(log.contains("machine start ctx-test-machine"));
        assert!(!log.contains("--now"));
        machine_cache_server.abort();
    }

    #[test]
    fn podman_machine_temp_state_paths_match_expected_names() {
        let data_root = tempfile::tempdir().expect("tempdir");
        let paths = podman_machine_temp_state_paths(data_root.path(), "ctx");
        let rendered: Vec<String> = paths
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        let expected_tmp_prefix = podman_temp_root(data_root.path())
            .join("podman")
            .to_string_lossy()
            .to_string();
        assert!(rendered.iter().any(|p| p.starts_with(&expected_tmp_prefix)));
        assert!(rendered.iter().any(|p| p.ends_with("podman/gvproxy.pid")));
        assert!(rendered.iter().any(|p| p.ends_with("podman/ctx-api.sock")));
        assert!(rendered
            .iter()
            .any(|p| p.ends_with("podman/ctx-gvproxy.sock")));
        assert!(rendered.iter().any(|p| p.ends_with("podman/ctx.sock")));
        assert!(rendered
            .iter()
            .any(|p| p.ends_with("home/.podman/ctx-api.sock")));
        assert!(rendered
            .iter()
            .any(|p| p.ends_with("home/.podman/ctx-gvproxy.sock")));
    }

    #[tokio::test]
    async fn podman_machine_singleflight_lock_reuses_lock_for_same_machine() {
        let first = podman_machine_singleflight_lock("ctx-machine-a");
        let second = podman_machine_singleflight_lock("ctx-machine-a");
        assert!(Arc::ptr_eq(&first, &second));

        let guard = first.lock().await;
        assert!(second.try_lock().is_err());
        drop(guard);
        assert!(second.try_lock().is_ok());
    }

    #[tokio::test]
    async fn podman_machine_singleflight_lock_isolated_by_machine_name() {
        let first = podman_machine_singleflight_lock("ctx-machine-b");
        let second = podman_machine_singleflight_lock("ctx-machine-c");
        assert!(!Arc::ptr_eq(&first, &second));

        let _guard = first.lock().await;
        assert!(second.try_lock().is_ok());
    }

    #[test]
    fn transparent_proxy_policy_maps_llm_only_to_explicit_allowlist_entries() {
        let settings = ContainerExecutionSettings::default();
        let (mode, allowlist) = transparent_proxy_policy(&settings);
        assert_eq!(mode, ContainerNetworkMode::Allowlist);
        assert!(allowlist.iter().any(|entry| entry == "openrouter.ai"));
        assert!(allowlist.iter().any(|entry| entry == "api.openai.com"));
    }

    #[test]
    fn transparent_proxy_policy_preserves_custom_allowlist_mode() {
        let settings = ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::Allowlist,
            allowlist: vec!["example.com".to_string(), "api.example.com".to_string()],
            ..Default::default()
        };
        let (mode, allowlist) = transparent_proxy_policy(&settings);
        assert_eq!(mode, ContainerNetworkMode::Allowlist);
        assert_eq!(allowlist, settings.allowlist);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unrestricted_network_transition_surfaces_teardown_failures() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let fakebin = temp.path().join("fakebin");
        std::fs::create_dir_all(&fakebin).expect("create fakebin");
        let helper_log_path = temp.path().join("cleanup-helpers.log");
        let pid_file_path = temp.path().join("ctx-egress-proxy.pid");
        std::fs::write(&pid_file_path, b"\n").expect("write fake proxy pid file");
        let _pid_file_guard = EnvGuard::set(
            "CTX_EGRESS_PROXY_PID_FILE",
            &pid_file_path.to_string_lossy(),
        );

        let rm_path = fakebin.join("rm");
        std::fs::write(
            &rm_path,
            format!(
                "#!/bin/sh\nprintf 'rm %s\\n' \"$*\" >> \"{log}\"\necho 'failed to remove proxy pid file' >&2\nexit 23\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake rm");
        std::fs::set_permissions(&rm_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake rm");

        let iptables_path = fakebin.join("iptables");
        std::fs::write(
            &iptables_path,
            format!(
                "#!/bin/sh\nprintf 'iptables %s\\n' \"$*\" >> \"{log}\"\nif [ \"$1\" = \"-P\" ] && [ \"$2\" = \"OUTPUT\" ] && [ \"$3\" = \"ACCEPT\" ]; then\n  echo 'failed to reset output policy' >&2\n  exit 42\nfi\nexit 0\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake iptables");
        std::fs::set_permissions(&iptables_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake iptables");

        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nFAKEBIN=\"{fakebin}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  PATH=\"$FAKEBIN:$PATH\" /bin/sh -c \"$7\"\n  exit $?\nfi\nexit 0\n",
                log = log_path.display(),
                fakebin = fakebin.display(),
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

        let settings = ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            ..Default::default()
        };
        let err = apply_container_network_policy(
            temp.path(),
            WorkspaceId::new(),
            "ctx-harness-test",
            &settings,
            "127.0.0.1",
            4399,
        )
        .await
        .expect_err("teardown failure should be explicit");

        let message = format!("{err:#}");
        assert!(message.contains("failed to tear down restricted container network policy"));
        assert!(message.contains("stop transparent proxy"));
        assert!(message.contains("failed to remove proxy pid file"));
        assert!(message.contains("clear egress guard"));
        assert!(message.contains("failed to reset output policy"));

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert_eq!(
            log.lines().filter(|line| line.starts_with("exec ")).count(),
            2,
            "expected both teardown steps to run before surfacing the failure"
        );

        let helper_log =
            std::fs::read_to_string(&helper_log_path).expect("read cleanup helper invocation log");
        assert!(helper_log.contains(&format!("rm -f {}", pid_file_path.display())));
        assert!(helper_log.contains("iptables -t nat -F OUTPUT"));
        assert!(helper_log.contains("iptables -F OUTPUT"));
        assert!(helper_log.contains("iptables -P OUTPUT ACCEPT"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unrestricted_network_transition_ignores_stale_proxy_pid_file() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().lock().await;
        let temp = tempfile::tempdir().expect("tempdir");
        let log_path = temp.path().join("podman-invocations.log");
        let fakebin = temp.path().join("fakebin");
        std::fs::create_dir_all(&fakebin).expect("create fakebin");
        let helper_log_path = temp.path().join("cleanup-helpers.log");
        let pid_file_path = temp.path().join("ctx-egress-proxy.pid");
        std::fs::write(&pid_file_path, b"999999\n").expect("write stale proxy pid file");
        let _pid_file_guard = EnvGuard::set(
            "CTX_EGRESS_PROXY_PID_FILE",
            &pid_file_path.to_string_lossy(),
        );

        let rm_path = fakebin.join("rm");
        std::fs::write(
            &rm_path,
            format!(
                "#!/bin/sh\nprintf 'rm %s\\n' \"$*\" >> \"{log}\"\nexec /bin/rm \"$@\"\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake rm");
        std::fs::set_permissions(&rm_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake rm");

        let iptables_path = fakebin.join("iptables");
        std::fs::write(
            &iptables_path,
            format!(
                "#!/bin/sh\nprintf 'iptables %s\\n' \"$*\" >> \"{log}\"\nexit 0\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake iptables");
        std::fs::set_permissions(&iptables_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake iptables");

        let podman_path = temp.path().join("podman.sh");
        std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nFAKEBIN=\"{fakebin}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  PATH=\"$FAKEBIN:$PATH\" /bin/sh -c \"$7\"\n  exit $?\nfi\nexit 0\n",
                log = log_path.display(),
                fakebin = fakebin.display(),
            ),
        )
        .expect("write podman shim");
        std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod podman shim");
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

        let settings = ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            ..Default::default()
        };
        let applied = apply_container_network_policy(
            temp.path(),
            WorkspaceId::new(),
            "ctx-harness-test",
            &settings,
            "127.0.0.1",
            4399,
        )
        .await
        .expect("stale proxy pid should be ignored during unrestricted teardown");

        assert!(!applied.egress_guard);
        assert!(
            !pid_file_path.exists(),
            "stale proxy pid file should be removed during teardown"
        );

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert_eq!(
            log.lines().filter(|line| line.starts_with("exec ")).count(),
            2,
            "expected both unrestricted teardown steps to run"
        );

        let helper_log =
            std::fs::read_to_string(&helper_log_path).expect("read cleanup helper invocation log");
        assert!(helper_log.contains(&format!("rm -f {}", pid_file_path.display())));
        assert!(helper_log.contains("iptables -t nat -F OUTPUT"));
        assert!(helper_log.contains("iptables -F OUTPUT"));
        assert!(helper_log.contains("iptables -P OUTPUT ACCEPT"));
    }

    #[cfg(unix)]
    #[test]
    fn kill_ctx_managed_podman_helper_processes_reports_only_successful_kills() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().blocking_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        let machine_name = ctx_podman_machine_name(temp.path());
        let fakebin = temp.path().join("fakebin");
        std::fs::create_dir_all(&fakebin).expect("create fakebin");
        let helper_dir = temp
            .path()
            .join("managed")
            .join("runtimes")
            .join("podman")
            .join("macos")
            .join("aarch64")
            .join("podman-5.8.0")
            .join("usr")
            .join("libexec")
            .join("podman");
        let pkill_log_path = temp.path().join("pkill-invocations.log");
        let ps_count_path = temp.path().join("ps-count");

        let gvproxy = format!(
            "{} -forward-sock {} {}",
            helper_dir.join("gvproxy").display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            machine_name,
        );
        let vfkit = format!(
            "/opt/homebrew/bin/vfkit --device virtio-blk,path={} --device virtio-net,unixSocketPath={}",
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("applehv")
                .join(format!("{machine_name}-arm64.raw"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
        );
        let escaped_gvproxy = literal_pkill_pattern(&gvproxy);
        let escaped_vfkit = literal_pkill_pattern(&vfkit);

        let ps_path = fakebin.join("ps");
        std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n12484 {vfkit}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  printf '12484 {vfkit}\\n'\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
                vfkit = vfkit,
            ),
        )
        .expect("write fake ps");
        std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake ps");

        let kill_path = fakebin.join("pkill");
        std::fs::write(
            &kill_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\nif [ \"$4\" = '{vfkit}' ]; then\n  exit 1\nfi\nexit 0\n",
                log = pkill_log_path.display(),
                vfkit = escaped_vfkit,
            ),
        )
        .expect("write fake pkill");
        std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake pkill");

        let prior_path = std::env::var("PATH").unwrap_or_default();
        let path_value = format!("{}:{prior_path}", fakebin.display());
        let _guard = EnvGuard::set("PATH", &path_value);

        let outcome = kill_ctx_managed_podman_helper_processes(temp.path(), &machine_name);
        assert_eq!(outcome.killed, vec![6622]);
        assert_eq!(outcome.failed, vec![12484]);
        assert!(outcome.skipped.is_empty());

        let kill_log = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
        assert!(kill_log.contains(&format!("-9 -f -x {escaped_gvproxy}")));
        assert!(kill_log.contains(&format!("-9 -f -x {escaped_vfkit}")));
    }

    #[cfg(unix)]
    #[test]
    fn kill_ctx_managed_podman_helper_processes_escapes_regex_metacharacters_for_pkill() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().blocking_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        let machine_name = ctx_podman_machine_name(temp.path());
        let fakebin = temp.path().join("fakebin");
        std::fs::create_dir_all(&fakebin).expect("create fakebin");
        let helper_dir = temp
            .path()
            .join("managed")
            .join("runtimes")
            .join("podman")
            .join("macos")
            .join("aarch64")
            .join("podman-5.8.0")
            .join("usr")
            .join("libexec")
            .join("podman");
        let pkill_log_path = temp.path().join("pkill-invocations.log");
        let ps_count_path = temp.path().join("ps-count");

        let gvproxy = format!(
            "{} -forward-sock {} {}",
            helper_dir.join("gvproxy").display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            machine_name,
        );

        let ps_path = fakebin.join("ps");
        std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
            ),
        )
        .expect("write fake ps");
        std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake ps");

        let kill_path = fakebin.join("pkill");
        std::fs::write(
            &kill_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$4\" >> \"{log}\"\nexit 0\n",
                log = pkill_log_path.display(),
            ),
        )
        .expect("write fake pkill");
        std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake pkill");

        let prior_path = std::env::var("PATH").unwrap_or_default();
        let path_value = format!("{}:{prior_path}", fakebin.display());
        let _guard = EnvGuard::set("PATH", &path_value);

        let outcome = kill_ctx_managed_podman_helper_processes(temp.path(), &machine_name);
        assert_eq!(outcome.killed, vec![6622]);
        assert!(outcome.failed.is_empty());
        assert!(outcome.skipped.is_empty());

        let pkill_pattern = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
        assert_eq!(pkill_pattern.trim(), literal_pkill_pattern(&gvproxy));
    }

    #[cfg(unix)]
    #[test]
    fn kill_ctx_managed_podman_helper_processes_skips_reused_pid_after_command_scoped_kill() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock().blocking_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        let machine_name = ctx_podman_machine_name(temp.path());
        let fakebin = temp.path().join("fakebin");
        std::fs::create_dir_all(&fakebin).expect("create fakebin");
        let helper_dir = temp
            .path()
            .join("managed")
            .join("runtimes")
            .join("podman")
            .join("macos")
            .join("aarch64")
            .join("podman-5.8.0")
            .join("usr")
            .join("libexec")
            .join("podman");
        let pkill_log_path = temp.path().join("pkill-invocations.log");
        let ps_count_path = temp.path().join("ps-count");

        let gvproxy = format!(
            "{} -forward-sock {} {}",
            helper_dir.join("gvproxy").display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            machine_name,
        );

        let ps_path = fakebin.join("ps");
        std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  printf ' 6622 /usr/bin/python3 /tmp/not-ctx-helper.py\\n'\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
            ),
        )
        .expect("write fake ps");
        std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake ps");

        let kill_path = fakebin.join("pkill");
        std::fs::write(
            &kill_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\nexit 1\n",
                log = pkill_log_path.display(),
            ),
        )
        .expect("write fake pkill");
        std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake pkill");

        let prior_path = std::env::var("PATH").unwrap_or_default();
        let path_value = format!("{}:{prior_path}", fakebin.display());
        let _guard = EnvGuard::set("PATH", &path_value);

        let outcome = kill_ctx_managed_podman_helper_processes(temp.path(), &machine_name);
        assert!(outcome.killed.is_empty());
        assert!(outcome.failed.is_empty());
        assert_eq!(outcome.skipped, vec![6622]);
        let pkill_log = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
        assert!(pkill_log.contains(&format!("-9 -f -x {}", literal_pkill_pattern(&gvproxy))));
    }

    #[test]
    fn cached_container_action_reuses_when_mounts_and_network_match() {
        let cached = sample_cached_container();
        let settings = sample_container_settings();
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Reuse);
    }

    #[test]
    fn cached_container_action_recreates_when_mount_mode_changes() {
        let cached = sample_cached_container();
        let mut settings = sample_container_settings();
        settings.mount_mode = ContainerMountMode::DiskIsolated;
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Recreate);
    }

    #[test]
    fn cached_container_action_recreates_when_external_mounts_change() {
        let cached = sample_cached_container();
        let settings = sample_container_settings();
        let mut changed_mounts = cached.external_mounts.clone();
        changed_mounts.insert("/tmp/another".to_string());
        let action = cached_container_action(&cached, &settings, &changed_mounts);
        assert_eq!(action, CachedContainerAction::Recreate);
    }

    #[test]
    fn cached_container_action_reconfigures_when_network_mode_changes() {
        let cached = sample_cached_container();
        let mut settings = sample_container_settings();
        settings.network_mode = ContainerNetworkMode::All;
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Reconfigure);
    }

    #[test]
    fn cached_container_action_reconfigures_when_allowlist_changes() {
        let cached = sample_cached_container();
        let mut settings = sample_container_settings();
        settings.allowlist = vec!["example.com".to_string()];
        let action = cached_container_action(&cached, &settings, &cached.external_mounts);
        assert_eq!(action, CachedContainerAction::Reconfigure);
    }

    #[test]
    fn bundle_dir_mount_policy_matches_platform_expectations() {
        // Linux runtime is host-native; bundle mounts are always reachable.
        if cfg!(target_os = "linux") {
            assert!(should_mount_bundle_dir_in_container(Path::new(
                "/Applications/ctx.app/Contents/Resources/bundles"
            )));
            return;
        }

        // Podman-machine platforms cannot reliably mount non-home host paths (for example
        // /Applications in macOS release installs).
        if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
            assert!(!should_mount_bundle_dir_in_container(Path::new(
                "/Applications/ctx.app/Contents/Resources/bundles"
            )));
            let home_var = if cfg!(target_os = "windows") {
                "USERPROFILE"
            } else {
                "HOME"
            };
            if let Some(home) = std::env::var_os(home_var).map(PathBuf::from) {
                assert!(should_mount_bundle_dir_in_container(
                    &home.join("ctx-bundles")
                ));
            }
        }
    }

    #[test]
    fn build_mounts_only_includes_bundle_dir_when_shareable() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let bundle_dir = tmp.path().join("bundles");
        std::fs::create_dir_all(&bundle_dir).expect("create bundle dir");
        let _guard = EnvGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());

        let workspace = sample_workspace(&tmp);
        let mounts = build_mounts(
            tmp.path(),
            &workspace,
            None,
            &ContainerExecutionSettings::default(),
        )
        .mounts;
        let expected = bind_mount(&bundle_dir, &bundle_dir, true);
        let has_bundle_mount = mounts.iter().any(|mount| mount == &expected);
        assert_eq!(
            has_bundle_mount,
            should_mount_bundle_dir_in_container(&bundle_dir)
        );
    }

    #[tokio::test]
    async fn managed_default_image_install_lock_serializes_callers() {
        let lock = managed_default_image_install_lock();
        let guard = lock.lock().await;
        let acquired = Arc::new(AtomicBool::new(false));
        let acquired_clone = Arc::clone(&acquired);

        let waiter = tokio::spawn(async move {
            let _wait_guard = lock.lock().await;
            acquired_clone.store(true, Ordering::SeqCst);
        });

        sleep(Duration::from_millis(30)).await;
        assert!(
            !acquired.load(Ordering::SeqCst),
            "second caller should still be blocked while first holds the lock"
        );
        drop(guard);

        waiter.await.expect("waiter task");
        assert!(
            acquired.load(Ordering::SeqCst),
            "second caller should acquire lock after first releases it"
        );
    }

    #[tokio::test]
    async fn managed_default_image_ensure_is_concurrency_safe() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let body = b"ctx-test-managed-image".to_vec();
        let digest = {
            let mut hasher = Sha256::new();
            hasher.update(&body);
            hex::encode(hasher.finalize())
        };
        let (url, server) = spawn_static_http_server(body.clone()).await;
        let source = bundled_assets::ManagedArtifactSource {
            uri: url,
            sha256: digest,
        };
        let data_root = tmp.path().to_path_buf();

        let root_a = data_root.clone();
        let root_b = data_root.clone();
        let source_a = source.clone();
        let source_b = source.clone();
        let (res_a, res_b) = tokio::join!(
            tokio::spawn(async move {
                ensure_managed_default_container_image_tar_with_source(
                    &root_a, &source_a, None, None,
                )
                .await
            }),
            tokio::spawn(async move {
                ensure_managed_default_container_image_tar_with_source(
                    &root_b, &source_b, None, None,
                )
                .await
            })
        );
        server.abort();

        let path_a = res_a.expect("join a").expect("ensure a");
        let path_b = res_b.expect("join b").expect("ensure b");
        assert_eq!(path_a, path_b);
        let cached = tokio::fs::read(&path_a).await.expect("read cached tar");
        assert_eq!(cached, body);
    }
}
