use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use sha2::Digest;
use sysinfo::{Pid, Signal, System};
use tokio::io::AsyncReadExt;
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

pub trait HarnessSetupObserver: Send + Sync {
    fn on_phase(&self, phase: HarnessSetupPhase, message: &str);
    fn on_log(&self, phase: HarnessSetupPhase, level: HarnessSetupLogLevel, message: &str);
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

pub(crate) fn workspace_container_name(workspace_id: WorkspaceId) -> String {
    format!("ctx-harness-{}", workspace_id.0)
}

fn ctx_podman_machine_name(data_root: &Path) -> String {
    let hash = podman_data_root_hash(data_root);
    format!("{CTX_PODMAN_MACHINE_PREFIX}-{hash}")
}

fn podman_data_root_hash(data_root: &Path) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(data_root.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn podman_runtime_root(data_root: &Path) -> PathBuf {
    let hash = podman_data_root_hash(data_root);
    #[cfg(unix)]
    {
        // Keep podman machine socket paths short enough for unix domain socket limits.
        PathBuf::from("/tmp").join("ctxp").join(hash)
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("ctxp").join(hash)
    }
}

fn podman_home_root(data_root: &Path) -> PathBuf {
    podman_runtime_root(data_root).join("home")
}

fn podman_temp_root(data_root: &Path) -> PathBuf {
    podman_runtime_root(data_root).join("tmp")
}

fn podman_machine_cache_root(data_root: &Path) -> PathBuf {
    data_root
        .join("podman")
        .join("xdg")
        .join("data")
        .join("containers")
        .join("podman")
        .join("machine")
}

fn shared_podman_machine_cache_root() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(PODMAN_MACHINE_CACHE_DIR_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    directories::BaseDirs::new().map(|base| {
        base.cache_dir()
            .join("ctx")
            .join("podman-machine")
            .join(std::env::consts::OS)
            .join(std::env::consts::ARCH)
    })
}

fn collect_podman_machine_cache_file_relpaths(root: &Path) -> Result<Vec<PathBuf>> {
    fn collect_recursive(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in
            std::fs::read_dir(dir).with_context(|| format!("read_dir {}", dir.display()))?
        {
            let entry = entry.with_context(|| format!("read_dir entry {}", dir.display()))?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("stat {}", path.display()))?;
            if file_type.is_dir() {
                collect_recursive(root, &path, out)?;
                continue;
            }
            if file_type.is_file() || file_type.is_symlink() {
                let relpath = path.strip_prefix(root).with_context(|| {
                    format!("computing relative cache path for {}", path.display())
                })?;
                out.push(relpath.to_path_buf());
            }
        }
        Ok(())
    }

    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut relpaths = Vec::new();
    for provider_dir in
        std::fs::read_dir(root).with_context(|| format!("read_dir {}", root.display()))?
    {
        let provider_dir =
            provider_dir.with_context(|| format!("read_dir entry {}", root.display()))?;
        let provider_path = provider_dir.path();
        if !provider_path.is_dir() {
            continue;
        }
        let cache_dir = provider_path.join("cache");
        if !cache_dir.is_dir() {
            continue;
        }
        collect_recursive(root, &cache_dir, &mut relpaths)?;
    }
    relpaths.sort();
    Ok(relpaths)
}

#[cfg(unix)]
fn symlink_cache_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(windows)]
fn symlink_cache_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dest)
}

#[cfg(not(any(unix, windows)))]
fn symlink_cache_file(_src: &Path, _dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "file symlinks are not supported on this platform",
    ))
}

fn podman_machine_cache_tmp_path(dest: &Path) -> PathBuf {
    let file_name = dest
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("cache-file");
    dest.with_file_name(format!(
        ".{file_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ))
}

async fn materialize_podman_machine_cache_file(
    src: &Path,
    dest: &Path,
    allow_symlink: bool,
) -> Result<()> {
    if src == dest || dest.exists() {
        return Ok(());
    }
    let Some(parent) = dest.parent() else {
        anyhow::bail!(
            "podman machine cache target has no parent: {}",
            dest.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;

    match std::fs::hard_link(src, dest) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(_) => {}
    }
    if allow_symlink {
        match symlink_cache_file(src, dest) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
            Err(_) => {}
        }
    }

    let tmp = podman_machine_cache_tmp_path(dest);
    let _ = fs::remove_file(&tmp).await;
    fs::copy(src, &tmp)
        .await
        .with_context(|| format!("copying {} -> {}", src.display(), tmp.display()))?;
    match fs::rename(&tmp, dest).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = fs::remove_file(&tmp).await;
            Ok(())
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp).await;
            Err(err).with_context(|| format!("moving {} -> {}", tmp.display(), dest.display()))
        }
    }
}

async fn seed_shared_podman_machine_cache(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let Some(shared_root) = shared_podman_machine_cache_root() else {
        return Ok(());
    };
    let relpaths = collect_podman_machine_cache_file_relpaths(&shared_root)?;
    if relpaths.is_empty() {
        return Ok(());
    }
    let local_root = podman_machine_cache_root(data_root);
    let mut seeded = 0usize;
    for relpath in relpaths {
        let src = shared_root.join(&relpath);
        let dest = local_root.join(&relpath);
        if dest.exists() {
            continue;
        }
        materialize_podman_machine_cache_file(&src, &dest, true).await?;
        seeded += 1;
    }
    if seeded > 0 {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "seeded {seeded} podman machine cache file(s) from {}",
                shared_root.display()
            ),
        );
    }
    Ok(())
}

async fn persist_podman_machine_cache_to_shared(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let Some(shared_root) = shared_podman_machine_cache_root() else {
        return Ok(());
    };
    let local_root = podman_machine_cache_root(data_root);
    let relpaths = collect_podman_machine_cache_file_relpaths(&local_root)?;
    if relpaths.is_empty() {
        return Ok(());
    }
    let mut persisted = 0usize;
    for relpath in relpaths {
        let src = local_root.join(&relpath);
        let dest = shared_root.join(&relpath);
        if src == dest || dest.exists() {
            continue;
        }
        materialize_podman_machine_cache_file(&src, &dest, false).await?;
        persisted += 1;
    }
    if persisted > 0 {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!(
                "persisted {persisted} podman machine cache file(s) into {}",
                shared_root.display()
            ),
        );
    }
    Ok(())
}

async fn seed_shared_podman_machine_cache_best_effort(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    if let Err(err) = seed_shared_podman_machine_cache(data_root, observer).await {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!("failed to seed shared podman machine cache: {err:#}"),
        );
        tracing::warn!("failed to seed shared podman machine cache: {err:#}");
    }
}

async fn persist_podman_machine_cache_to_shared_best_effort(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    if let Err(err) = persist_podman_machine_cache_to_shared(data_root, observer).await {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!("failed to persist shared podman machine cache: {err:#}"),
        );
        tracing::warn!("failed to persist shared podman machine cache: {err:#}");
    }
}

static PODMAN_MACHINE_SINGLEFLIGHT_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<Mutex<()>>>>> =
    OnceLock::new();

fn podman_machine_singleflight_lock(machine_name: &str) -> Arc<Mutex<()>> {
    let registry = PODMAN_MACHINE_SINGLEFLIGHT_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut guard = match registry.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard
        .entry(machine_name.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
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
        ensure_managed_podman_runtime(&self.data_root, None).await?;
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
        let init_outcome = run_podman_machine_init(&self.data_root, &machine_name, None).await?;
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
        let podman_bin = ensure_managed_podman_runtime(&self.data_root, None)
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
        ensure_managed_podman_runtime(&self.data_root, observer)
            .await
            .context("podman unavailable and execution mode is container")?;
        let proxy_host = "host.containers.internal";
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let _ = self
            .ensure_container(
                workspace,
                None,
                &settings.container,
                proxy_host,
                daemon_port,
                observer,
            )
            .await?;
        Ok(())
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
        let name = format!("ctx-harness-{}", workspace.id.0);
        let image = resolve_container_image(settings);
        observe_phase(
            observer,
            HarnessSetupPhase::MachineCheck,
            "checking container runtime",
        );
        // On macOS/Windows the engine is a VM; ensure it is running before we run any podman ops.
        ensure_podman_machine_running_with_observer(&self.data_root, observer).await?;
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
        if let Some(container) = containers.get(&workspace.id) {
            match cached_container_action(container, settings, &mount_plan.external_mounts) {
                CachedContainerAction::Reuse => {
                    observe_log(
                        observer,
                        HarnessSetupPhase::ContainerCheck,
                        HarnessSetupLogLevel::Info,
                        "container already ready in runtime cache",
                    );
                    return Ok(container.clone());
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
            observe_phase(
                observer,
                HarnessSetupPhase::ImageCheck,
                "checking harness image availability",
            );
            let image_present = container_image_present(&self.data_root, &image).await?;
            if !image_present {
                observe_phase(
                    observer,
                    HarnessSetupPhase::ImageLoad,
                    "loading harness image into podman",
                );
                ensure_container_image_available(&self.data_root, &image, observer).await?;
            } else {
                observe_log(
                    observer,
                    HarnessSetupPhase::ImageCheck,
                    HarnessSetupLogLevel::Info,
                    "harness image already present",
                );
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
        let egress_guard = if matches!(settings.network_mode, ContainerNetworkMode::All) {
            if let Err(err) = stop_transparent_proxy(&self.data_root, &name).await {
                observe_log(
                    observer,
                    HarnessSetupPhase::RuntimeNetworkSetup,
                    HarnessSetupLogLevel::Warn,
                    &format!("failed to stop transparent proxy: {err:#}"),
                );
                tracing::warn!("failed to stop transparent proxy: {err:#}");
            }
            if let Err(err) = clear_egress_guard(&self.data_root, &name).await {
                observe_log(
                    observer,
                    HarnessSetupPhase::RuntimeNetworkSetup,
                    HarnessSetupLogLevel::Warn,
                    &format!("failed to clear egress guard: {err:#}"),
                );
                tracing::warn!("failed to clear egress guard: {err:#}");
            }
            false
        } else {
            // Prefer the in-image proxy binary (required for out-of-the-box behavior on macOS/Windows).
            // If the image doesn't have it, we also allow an explicit override via CTX_EGRESS_PROXY_PATH,
            // but restricted modes must not silently fall back to full network access.
            let proxy_bin = match ensure_egress_proxy_available(&self.data_root, &name).await {
                Ok(()) => EGRESS_PROXY_CONTAINER_PATH.to_string(),
                Err(img_err) => {
                    // Optional escape hatch: allow a Linux proxy binary to be provided via the host
                    // (it is bind-mounted into the container under ~/.ctx/runtimes/...).
                    if std::env::var("CTX_EGRESS_PROXY_PATH").ok().is_some() {
                        let host_bin = ensure_egress_proxy_binary(&self.data_root).await?;
                        host_bin.to_string_lossy().to_string()
                    } else {
                        return Err(img_err).context(
                            "restricted container networking requires ctx-egress-proxy in the container image",
                        );
                    }
                }
            };
            let (proxy_mode, proxy_allowlist) = transparent_proxy_policy(settings);
            let proxy_config = TransparentProxyConfig {
                listen: format!("127.0.0.1:{TRANSPARENT_PROXY_PORT}"),
                mode: proxy_mode,
                allowlist: proxy_allowlist,
                max_peek_bytes: 16 * 1024,
            };
            let config_path = write_transparent_proxy_config(
                &container_data_root(&self.data_root, workspace.id),
                proxy_config,
            )
            .await?;
            start_transparent_proxy(
                &self.data_root,
                &name,
                &PathBuf::from(proxy_bin),
                &config_path,
            )
            .await?;
            configure_transparent_egress_guard(
                &self.data_root,
                &name,
                TRANSPARENT_PROXY_PORT,
                daemon_host,
                daemon_port,
            )
            .await?
        };
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

pub(crate) fn resolve_container_image(settings: &ContainerExecutionSettings) -> String {
    if let Ok(value) = std::env::var("CTX_HARNESS_CONTAINER_IMAGE") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    settings
        .image
        .clone()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_CONTAINER_IMAGE.to_string())
}

pub fn default_container_image() -> &'static str {
    DEFAULT_CONTAINER_IMAGE
}

pub fn is_default_container_image(image: &str) -> bool {
    image.trim() == DEFAULT_CONTAINER_IMAGE
}

pub fn bundled_default_container_image_tar() -> Option<PathBuf> {
    bundled_assets::bundled_ctx_harness_image_tar(DEFAULT_CONTAINER_IMAGE)
}

pub async fn prefetch_container_image_with_observer(
    data_root: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    observe_phase(
        observer,
        HarnessSetupPhase::MachineCheck,
        "checking container runtime",
    );
    ensure_managed_podman_runtime(data_root, observer).await?;
    // On macOS/Windows `podman` is a remote client; image ops require a running machine.
    ensure_podman_machine_running_with_observer(data_root, observer).await?;
    observe_phase(
        observer,
        HarnessSetupPhase::ImageCheck,
        "checking harness image availability",
    );
    if container_image_present(data_root, image).await? {
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
    ensure_container_image_available(data_root, image, observer).await
}

pub async fn prefetch_container_image(data_root: &Path, image: &str) -> Result<()> {
    prefetch_container_image_with_observer(data_root, image, None).await
}

pub async fn container_image_present(data_root: &Path, image: &str) -> Result<bool> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let mut cmd = podman_command(data_root)?;
    cmd.arg("image").arg("exists").arg("--").arg(image);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(true);
    }
    match output.status.code() {
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "podman image exists failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

async fn ensure_container_image_available(
    data_root: &Path,
    image: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }

    if container_image_present(data_root, image).await? {
        return Ok(());
    }

    // Preferred path for the default image: deterministic load from bundled tar.
    // If no bundled tar is present (minimal startup bundle), download from managed runtime-lock
    // source and load into Podman.
    if image == DEFAULT_CONTAINER_IMAGE {
        let image_tar = if let Some(tar) = bundled_assets::bundled_ctx_harness_image_tar(image) {
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "loading default harness image from bundled tar {}",
                    tar.display()
                ),
            );
            tar
        } else {
            let managed_tar =
                ensure_managed_default_container_image_tar(data_root, observer).await?;
            observe_log(
                observer,
                HarnessSetupPhase::ImageLoad,
                HarnessSetupLogLevel::Info,
                &format!(
                    "loading default harness image from managed cache {}",
                    managed_tar.display()
                ),
            );
            managed_tar
        };
        load_container_image_tar(data_root, &image_tar, image).await?;
        return Ok(());
    }

    anyhow::bail!(
        "container image '{}' is not present; registry pulls are disabled, so the image must already exist in podman",
        image
    );
}

fn managed_default_container_image_tar_path(data_root: &Path, sha256: &str) -> PathBuf {
    data_root
        .join("managed")
        .join("images")
        .join("ctx-harness")
        .join("linux")
        .join(std::env::consts::ARCH)
        .join(format!("sha256-{}.tar", sha256.trim().to_ascii_lowercase()))
}

fn managed_default_image_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn ensure_managed_default_container_image_tar(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    let source = bundled_assets::managed_ctx_harness_image_source(DEFAULT_CONTAINER_IMAGE)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "default harness image is missing from bundle and runtime lock managed sources"
            )
        })?;
    ensure_managed_default_container_image_tar_with_source(data_root, &source, observer).await
}

async fn ensure_managed_default_container_image_tar_with_source(
    data_root: &Path,
    source: &bundled_assets::ManagedArtifactSource,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    // Provider installs and startup prewarm can concurrently ensure the default harness image.
    // Serialize this path so callers don't race on the shared temp download file.
    let _install_guard = managed_default_image_install_lock().lock().await;

    let final_tar = managed_default_container_image_tar_path(data_root, &source.sha256);
    if final_tar.exists() {
        let digest = updates::sha256_hex_file(&final_tar)
            .await
            .with_context(|| format!("computing sha256 for {}", final_tar.display()))?;
        if digest.eq_ignore_ascii_case(source.sha256.trim()) {
            return Ok(final_tar);
        }
        observe_log(
            observer,
            HarnessSetupPhase::ImageLoad,
            HarnessSetupLogLevel::Warn,
            &format!(
                "managed image cache checksum mismatch for {}; re-downloading",
                final_tar.display()
            ),
        );
        let _ = fs::remove_file(&final_tar).await;
    }

    let Some(parent) = final_tar.parent() else {
        anyhow::bail!(
            "managed image cache path has no parent: {}",
            final_tar.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let tmp_tar = final_tar.with_extension("download");

    observe_log(
        observer,
        HarnessSetupPhase::ImageLoad,
        HarnessSetupLogLevel::Info,
        &format!("downloading default harness image from {}", source.uri),
    );
    download_managed_artifact(&source.uri, &tmp_tar).await?;

    let digest = updates::sha256_hex_file(&tmp_tar)
        .await
        .with_context(|| format!("computing sha256 for {}", tmp_tar.display()))?;
    if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
        let _ = fs::remove_file(&tmp_tar).await;
        anyhow::bail!(
            "managed harness image checksum mismatch: expected {}, got {}",
            source.sha256.trim(),
            digest
        );
    }
    fs::rename(&tmp_tar, &final_tar).await.with_context(|| {
        format!(
            "moving managed image tar into place: {} -> {}",
            tmp_tar.display(),
            final_tar.display()
        )
    })?;
    Ok(final_tar)
}

fn podman_platform_tokens() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH)
}

fn managed_podman_runtime_source() -> Option<bundled_assets::ManagedRuntimeSource> {
    let (os, arch) = podman_platform_tokens();
    bundled_assets::managed_runtime_source("podman", os, arch)
}

fn managed_podman_runtime_root(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = podman_platform_tokens();
    data_root
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join(os)
        .join(arch)
        .join(format!("podman-{}", source.version.trim()))
}

fn managed_podman_runtime_bin_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    managed_podman_runtime_root(data_root, source).join(source.bin.trim())
}

fn managed_podman_helper_path(runtime_root: &Path, helper_name: &str) -> Option<PathBuf> {
    let helper = helper_name.trim();
    if helper.is_empty() {
        return None;
    }
    Some(
        runtime_root
            .join("usr")
            .join("libexec")
            .join("podman")
            .join(helper),
    )
}

fn managed_artifact_extension(uri: &str) -> &'static str {
    let path = Url::parse(uri)
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

fn managed_podman_archive_path(
    data_root: &Path,
    source: &bundled_assets::ManagedRuntimeSource,
) -> PathBuf {
    let (os, arch) = podman_platform_tokens();
    let ext = managed_artifact_extension(&source.uri);
    data_root
        .join("managed")
        .join("downloads")
        .join("podman")
        .join(os)
        .join(arch)
        .join(format!(
            "sha256-{}.{}",
            source.sha256.trim().to_ascii_lowercase(),
            ext
        ))
}

fn extract_zip_to_dir(zip_path: &Path, out_dir: &Path) -> Result<()> {
    let file =
        std::fs::File::open(zip_path).with_context(|| format!("open {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("parsing zip archive")?;
    for idx in 0..archive.len() {
        let mut entry = archive.by_index(idx).context("zip entry")?;
        if entry.is_dir() {
            continue;
        }
        let Some(enclosed) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let dest = out_dir.join(enclosed);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let mut out =
            std::fs::File::create(&dest).with_context(|| format!("create {}", dest.display()))?;
        std::io::copy(&mut entry, &mut out).context("extract zip entry")?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

fn extract_archive_to_dir(archive_path: &Path, source_uri: &str, out_dir: &Path) -> Result<()> {
    let kind = managed_artifact_extension(source_uri);
    match kind {
        "zip" => extract_zip_to_dir(archive_path, out_dir),
        "tar.gz" | "tgz" => {
            let archive_file = std::fs::File::open(archive_path)
                .with_context(|| format!("open {}", archive_path.display()))?;
            let decoder = flate2::read::GzDecoder::new(archive_file);
            let mut archive = tar::Archive::new(decoder);
            archive.unpack(out_dir).context("extract tar.gz archive")
        }
        "tar" => {
            let archive_file = std::fs::File::open(archive_path)
                .with_context(|| format!("open {}", archive_path.display()))?;
            let mut archive = tar::Archive::new(archive_file);
            archive.unpack(out_dir).context("extract tar archive")
        }
        _ => anyhow::bail!("unsupported podman archive type for {source_uri}"),
    }
}

fn resolve_single_extracted_root(extract_dir: &Path) -> Result<PathBuf> {
    let mut dirs = Vec::new();
    let mut has_files = false;
    for entry in std::fs::read_dir(extract_dir)
        .with_context(|| format!("read_dir {}", extract_dir.display()))?
    {
        let entry = entry.with_context(|| format!("read_dir entry {}", extract_dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        } else {
            has_files = true;
        }
    }
    if has_files || dirs.len() != 1 {
        return Ok(extract_dir.to_path_buf());
    }
    Ok(dirs.remove(0))
}

fn managed_podman_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

async fn ensure_managed_podman_runtime(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var(PODMAN_PATH_ENV) {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Ok(path);
        }
    }
    if let Some(bundled) = bundled_assets::bundled_podman_runtime() {
        return Ok(bundled.bin);
    }
    let source = managed_podman_runtime_source().ok_or_else(|| {
        anyhow::anyhow!(
            "managed podman runtime source is not available for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let runtime_root = managed_podman_runtime_root(data_root, &source);
    let runtime_bin = managed_podman_runtime_bin_path(data_root, &source);
    if runtime_bin.exists() {
        return Ok(runtime_bin);
    }
    let _install_guard = managed_podman_install_lock().lock().await;
    if runtime_bin.exists() {
        return Ok(runtime_bin);
    }

    observe_log(
        observer,
        HarnessSetupPhase::MachineCheck,
        HarnessSetupLogLevel::Info,
        &format!("installing managed podman runtime {}", source.version),
    );

    let final_archive = managed_podman_archive_path(data_root, &source);
    if final_archive.exists() {
        let digest = updates::sha256_hex_file(&final_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", final_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&final_archive).await;
        }
    }
    if !final_archive.exists() {
        let Some(parent) = final_archive.parent() else {
            anyhow::bail!(
                "managed podman archive path has no parent: {}",
                final_archive.display()
            );
        };
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
        let tmp_archive = final_archive.with_extension("download");
        download_managed_artifact(&source.uri, &tmp_archive).await?;
        let digest = updates::sha256_hex_file(&tmp_archive)
            .await
            .with_context(|| format!("computing sha256 for {}", tmp_archive.display()))?;
        if !digest.eq_ignore_ascii_case(source.sha256.trim()) {
            let _ = fs::remove_file(&tmp_archive).await;
            anyhow::bail!(
                "managed podman runtime checksum mismatch: expected {}, got {}",
                source.sha256.trim(),
                digest
            );
        }
        fs::rename(&tmp_archive, &final_archive)
            .await
            .with_context(|| {
                format!(
                    "moving managed podman archive into place: {} -> {}",
                    tmp_archive.display(),
                    final_archive.display()
                )
            })?;
    }

    let Some(parent) = runtime_root.parent() else {
        anyhow::bail!(
            "managed runtime root has no parent: {}",
            runtime_root.display()
        );
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let staging_dir = parent.join(format!(".podman-staging-{}", uuid::Uuid::new_v4().simple()));
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
    .context("joining managed podman extract task")??;
    let extracted_root = tokio::task::spawn_blocking({
        let extract_dir = extract_dir.clone();
        move || resolve_single_extracted_root(&extract_dir)
    })
    .await
    .context("joining managed podman extraction root task")??;

    if runtime_root.exists() {
        let _ = fs::remove_dir_all(&runtime_root).await;
    }
    fs::rename(&extracted_root, &runtime_root)
        .await
        .with_context(|| {
            format!(
                "moving extracted podman runtime into place: {} -> {}",
                extracted_root.display(),
                runtime_root.display()
            )
        })?;
    let _ = fs::remove_dir_all(&staging_dir).await;

    for (name, helper) in &source.helpers {
        let Some(path) = managed_podman_helper_path(&runtime_root, name) else {
            continue;
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        if path.exists() {
            let digest = updates::sha256_hex_file(&path)
                .await
                .with_context(|| format!("computing sha256 for {}", path.display()))?;
            if digest.eq_ignore_ascii_case(helper.sha256.trim()) {
                continue;
            }
            let _ = fs::remove_file(&path).await;
        }
        let tmp = path.with_extension("download");
        download_managed_artifact(&helper.uri, &tmp).await?;
        let digest = updates::sha256_hex_file(&tmp)
            .await
            .with_context(|| format!("computing sha256 for {}", tmp.display()))?;
        if !digest.eq_ignore_ascii_case(helper.sha256.trim()) {
            let _ = fs::remove_file(&tmp).await;
            anyhow::bail!(
                "managed podman helper checksum mismatch ({}): expected {}, got {}",
                name,
                helper.sha256.trim(),
                digest
            );
        }
        fs::rename(&tmp, &path).await.with_context(|| {
            format!(
                "moving managed podman helper into place: {} -> {}",
                tmp.display(),
                path.display()
            )
        })?;
    }

    if !runtime_bin.exists() {
        anyhow::bail!(
            "managed podman runtime installed but binary is missing at {}",
            runtime_bin.display()
        );
    }
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&runtime_bin)
            .await
            .with_context(|| format!("metadata {}", runtime_bin.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&runtime_bin, perms)
            .await
            .with_context(|| format!("chmod {}", runtime_bin.display()))?;
        for name in source.helpers.keys() {
            if let Some(helper_path) = managed_podman_helper_path(&runtime_root, name) {
                if helper_path.exists() {
                    let mut helper_perms = fs::metadata(&helper_path)
                        .await
                        .with_context(|| format!("metadata {}", helper_path.display()))?
                        .permissions();
                    helper_perms.set_mode(0o755);
                    fs::set_permissions(&helper_path, helper_perms)
                        .await
                        .with_context(|| format!("chmod {}", helper_path.display()))?;
                }
            }
        }
    }
    Ok(runtime_bin)
}

async fn download_managed_artifact(url: &str, dest: &Path) -> Result<()> {
    let Some(parent) = dest.parent() else {
        anyhow::bail!("download destination missing parent: {}", dest.display());
    };
    fs::create_dir_all(parent)
        .await
        .with_context(|| format!("creating {}", parent.display()))?;
    let client = reqwest::Client::builder()
        .timeout(PODMAN_LOAD_TIMEOUT)
        .connect_timeout(Duration::from_secs(20))
        .build()
        .context("building reqwest client for managed artifact download")?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("downloading managed artifact: {url}"))?
        .error_for_status()
        .with_context(|| format!("managed artifact download http error: {url}"))?;
    let mut stream = response.bytes_stream();
    let mut file = fs::File::create(dest)
        .await
        .with_context(|| format!("creating {}", dest.display()))?;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.with_context(|| format!("reading download stream from {url}"))?;
        file.write_all(&chunk)
            .await
            .with_context(|| format!("writing {}", dest.display()))?;
    }
    file.flush()
        .await
        .with_context(|| format!("flushing {}", dest.display()))?;
    Ok(())
}

async fn load_container_image_tar(data_root: &Path, tar: &Path, image: &str) -> Result<()> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("load").arg("-i").arg(tar);
    let output = command_output_with_timeout(cmd, PODMAN_LOAD_TIMEOUT)
        .await
        .with_context(|| format!("podman load failed for {}", tar.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            anyhow::bail!("podman load failed (status: {})", output.status);
        }
        anyhow::bail!("podman load failed: {stderr}");
    }
    if container_image_present(data_root, image).await? {
        return Ok(());
    }
    anyhow::bail!(
        "podman load reported success but image '{}' is still missing",
        image
    );
}

#[derive(Debug, Clone)]
pub struct ContainerImageStatus {
    pub present: bool,
    pub available: bool,
    pub error: Option<String>,
}

pub async fn container_image_status(data_root: &Path, image: &str) -> Result<ContainerImageStatus> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    let output = match podman_command(data_root) {
        Ok(mut cmd) => {
            cmd.arg("image").arg("exists").arg("--").arg(image);
            command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await
        }
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            });
        }
    };
    let output = match output {
        Ok(out) => out,
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            });
        }
    };
    if output.status.success() {
        return Ok(ContainerImageStatus {
            present: true,
            available: true,
            error: None,
        });
    }
    match output.status.code() {
        Some(1) => Ok(ContainerImageStatus {
            present: false,
            available: true,
            error: None,
        }),
        _ => Ok(ContainerImageStatus {
            present: false,
            available: false,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        }),
    }
}

fn container_data_root(data_root: &Path, workspace_id: WorkspaceId) -> PathBuf {
    data_root
        .join("containers")
        .join("workspaces")
        .join(workspace_id.0.to_string())
        .join("data")
}

struct MountPlan {
    mounts: Vec<String>,
    external_mounts: HashSet<String>,
}

fn volume_mount(name: &str, dst: &str, read_only: bool) -> String {
    let mode = if read_only { "ro" } else { "rw" };
    // Podman mount syntax supports type=volume for VM-native storage.
    format!("type=volume,src={name},dst={dst},{mode}")
}

fn build_mounts(
    data_root: &Path,
    workspace: &Workspace,
    _worktree: Option<&Worktree>,
    settings: &ContainerExecutionSettings,
) -> MountPlan {
    let mut mounts = Vec::new();
    let workspace_root = PathBuf::from(&workspace.root_path);

    let worktrees_root = worktrees_root(data_root).join(workspace.id.0.to_string());
    if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
        // Disk-isolated mode: worktrees live in a container-managed volume on the Podman VM disk.
        // We mount that volume at a fixed path inside the container. The daemon mediates access.
        let vol_name = format!("ctx-ws-{}", workspace.id.0);
        mounts.push(volume_mount(&vol_name, CTX_CONTAINER_WORKSPACE_ROOT, false));
    } else {
        ensure_dir(&workspace_root);
        mounts.push(bind_mount(&workspace_root, &workspace_root, false));
        ensure_dir(&worktrees_root);
        mounts.push(bind_mount(&worktrees_root, &worktrees_root, false));
    }

    let container_data = container_data_root(data_root, workspace.id);
    ensure_dir(&container_data);
    mounts.push(bind_mount(&container_data, &container_data, false));

    let agent_servers = data_root.join("providers").join("agent-servers");
    ensure_dir(&agent_servers);
    mounts.push(bind_mount(&agent_servers, &agent_servers, true));

    let runtimes = data_root.join("runtimes");
    ensure_dir(&runtimes);
    mounts.push(bind_mount(&runtimes, &runtimes, true));

    // Bundled assets (desktop) are resolved via absolute paths under `CTX_BUNDLE_DIR`.
    // For container execution, mount this path only when the source path is shareable by the
    // container runtime on this host (for macOS/Windows Podman machine this excludes paths like
    // /Applications that are not VM-shared by default).
    if let Ok(raw) = std::env::var("CTX_BUNDLE_DIR") {
        let bundle_dir = PathBuf::from(raw.trim());
        if bundle_dir.exists() {
            if should_mount_bundle_dir_in_container(&bundle_dir) {
                mounts.push(bind_mount(&bundle_dir, &bundle_dir, true));
            } else {
                tracing::info!(
                    "skipping CTX_BUNDLE_DIR container mount (path not shareable by runtime): {}",
                    bundle_dir.display()
                );
            }
        }
    }

    MountPlan {
        mounts,
        external_mounts: HashSet::new(),
    }
}

fn should_mount_bundle_dir_in_container(bundle_dir: &Path) -> bool {
    if cfg!(target_os = "linux") {
        return true;
    }
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        let home_var = if cfg!(target_os = "windows") {
            "USERPROFILE"
        } else {
            "HOME"
        };
        if let Some(home) = std::env::var_os(home_var).map(PathBuf::from) {
            return bundle_dir.starts_with(home);
        }
        return false;
    }
    true
}

fn bind_mount(src: &Path, dst: &Path, read_only: bool) -> String {
    let mode = if read_only { "ro" } else { "rw" };
    format!(
        "type=bind,src={},dst={},{}",
        src.to_string_lossy(),
        dst.to_string_lossy(),
        mode
    )
}

fn ensure_dir(path: &Path) {
    let _ = std::fs::create_dir_all(path);
}

fn rewrite_daemon_url_for_container(daemon_url: &str, host: &str) -> String {
    if let Ok(mut url) = Url::parse(daemon_url) {
        let _ = url.set_host(Some(host));
        return url.to_string();
    }
    daemon_url.to_string()
}

fn daemon_port_from_url(daemon_url: &str) -> Option<u16> {
    Url::parse(daemon_url).ok()?.port_or_known_default()
}

fn proxy_runtime_root(data_root: &Path) -> PathBuf {
    data_root.join("runtimes").join(EGRESS_PROXY_RUNTIME_ID)
}

fn proxy_runtime_path(data_root: &Path) -> PathBuf {
    proxy_runtime_root(data_root).join(EGRESS_PROXY_BINARY)
}

fn llm_only_proxy_allowlist_entries() -> Vec<String> {
    let mut entries: Vec<String> = network_allowlist::LLM_ALLOWLIST
        .iter()
        .filter_map(|entry| network_allowlist::normalize_allowlist_entry(entry))
        .collect();
    entries.sort();
    entries.dedup();
    entries
}

fn transparent_proxy_policy(
    settings: &ContainerExecutionSettings,
) -> (ContainerNetworkMode, Vec<String>) {
    match settings.network_mode {
        // Use explicit allowlist mode for llm_only so policy is fully driven by daemon-side
        // config and does not depend on baked allowlist constants inside container images.
        ContainerNetworkMode::LlmOnly => (
            ContainerNetworkMode::Allowlist,
            llm_only_proxy_allowlist_entries(),
        ),
        ContainerNetworkMode::Allowlist => {
            (ContainerNetworkMode::Allowlist, settings.allowlist.clone())
        }
        ContainerNetworkMode::All => (ContainerNetworkMode::All, Vec::new()),
    }
}

async fn ensure_egress_proxy_binary(data_root: &Path) -> Result<PathBuf> {
    let runtime_root = proxy_runtime_root(data_root);
    fs::create_dir_all(&runtime_root).await?;
    let dest = proxy_runtime_path(data_root);
    let src = match std::env::var("CTX_EGRESS_PROXY_PATH") {
        Ok(path) => PathBuf::from(path),
        Err(_) => {
            anyhow::bail!(
                "missing CTX_EGRESS_PROXY_PATH; host-injected egress proxy requires an explicit Linux binary path"
            )
        }
    };
    if !src.exists() {
        anyhow::bail!("missing {EGRESS_PROXY_BINARY} binary at {}", src.display());
    }
    if src != dest {
        fs::copy(&src, &dest).await?;
    }
    let mut perms = fs::metadata(&dest).await?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&dest, perms).await?;
    Ok(dest)
}

async fn ensure_egress_proxy_available(data_root: &Path, container_name: &str) -> Result<()> {
    // Validate required tooling inside the container for restricted network modes.
    //
    // This is a hard requirement: without these, we cannot enforce allowlist/llm-only safely.
    let script = format!(
        "set -e; command -v iptables >/dev/null 2>&1; test -x '{EGRESS_PROXY_CONTAINER_PATH}'"
    );
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(container_name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "container missing required egress tooling (iptables and/or {EGRESS_PROXY_CONTAINER_PATH}); status {}",
        output.status
    );
}

async fn write_transparent_proxy_config(
    root: &Path,
    config: TransparentProxyConfig,
) -> Result<PathBuf> {
    fs::create_dir_all(root).await?;
    let path = root.join(EGRESS_PROXY_CONFIG_NAME);
    let raw = serde_json::to_string_pretty(&config)?;
    let mut file = fs::File::create(&path).await?;
    file.write_all(raw.as_bytes()).await?;
    Ok(path)
}

async fn start_transparent_proxy(
    data_root: &Path,
    name: &str,
    bin_path: &Path,
    config_path: &Path,
) -> Result<()> {
    let bin = bin_path.to_string_lossy();
    let config = config_path.to_string_lossy();
    let script = format!(
        r#"
set -e
pid_file="/tmp/ctx-egress-proxy.pid"
if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file" 2>/dev/null || true)"
  if [ -n "$old_pid" ]; then
    kill "$old_pid" || true
  fi
  rm -f "$pid_file"
fi
if command -v nohup >/dev/null 2>&1; then
  nohup '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
elif command -v setsid >/dev/null 2>&1; then
  setsid '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
else
  '{bin}' --config '{config}' >/tmp/ctx-egress-proxy.log 2>&1 &
fi
echo $! > "$pid_file"
exit 0
"#
    );
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "failed to start transparent proxy (status: {})",
            output.status
        );
    }
}

async fn stop_transparent_proxy(data_root: &Path, name: &str) -> Result<()> {
    let script = r#"
pid_file="/tmp/ctx-egress-proxy.pid"
if [ -f "$pid_file" ]; then
  old_pid="$(cat "$pid_file" 2>/dev/null || true)"
  if [ -n "$old_pid" ]; then
    kill "$old_pid" || true
  fi
  rm -f "$pid_file"
fi
exit 0
"#;
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "failed to stop transparent proxy (status: {})",
            output.status
        );
    }
}

async fn configure_transparent_egress_guard(
    data_root: &Path,
    name: &str,
    proxy_port: u16,
    daemon_host: &str,
    daemon_port: u16,
) -> Result<bool> {
    let script = format!(
        r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 43
fi
daemon_ip="$(getent hosts {daemon_host} | awk '{{print $1}}' | head -n1)"
if [ -z "$daemon_ip" ]; then
  exit 44
fi
iptables -t nat -F OUTPUT || true
iptables -F OUTPUT || true
iptables -P OUTPUT DROP
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -p udp --dport 53 -j ACCEPT
iptables -A OUTPUT -p tcp --dport 53 -j ACCEPT
iptables -A OUTPUT -d "$daemon_ip" -p tcp --dport {daemon_port} -j ACCEPT
iptables -A OUTPUT -m owner --uid-owner 0 -j ACCEPT
iptables -t nat -A OUTPUT -m owner --uid-owner 0 -j RETURN
iptables -t nat -A OUTPUT -p tcp --dport 80 -j REDIRECT --to-ports {proxy_port}
iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-ports {proxy_port}
exit 0
"#
    );
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        return Ok(true);
    }
    if let Some(code) = output.status.code() {
        if code == 43 {
            anyhow::bail!("iptables missing in harness container");
        }
        if code == 44 {
            anyhow::bail!("daemon host not resolvable inside harness container");
        }
    }
    anyhow::bail!(
        "failed to configure egress guard (status: {})",
        output.status
    );
}

async fn clear_egress_guard(data_root: &Path, name: &str) -> Result<()> {
    let script = r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 0
fi
iptables -t nat -F OUTPUT || true
iptables -F OUTPUT || true
iptables -P OUTPUT ACCEPT || true
exit 0
"#;
    let mut cmd = podman_command(data_root)?;
    cmd.arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!("failed to clear egress guard (status: {})", output.status);
    }
}

fn podman_available(data_root: &Path) -> bool {
    if cfg!(test) {
        if let Ok(value) = std::env::var("CTX_TEST_PODMAN_AVAILABLE") {
            let value = value.trim().to_ascii_lowercase();
            return matches!(value.as_str(), "1" | "true" | "yes" | "y");
        }
    }
    podman_binary_path(data_root).is_some()
}

fn podman_binary_path(data_root: &Path) -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(PODMAN_PATH_ENV) {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Some(path);
        }
    }
    if let Some(bundled) = bundled_assets::bundled_podman_runtime() {
        return Some(bundled.bin);
    }
    if let Some(source) = managed_podman_runtime_source() {
        let managed_path = managed_podman_runtime_bin_path(data_root, &source);
        if managed_path.exists() {
            return Some(managed_path);
        }
    }
    None
}

#[derive(Debug, Clone)]
pub(crate) struct PodmanInvocation {
    pub(crate) bin: PathBuf,
    pub(crate) env: HashMap<String, String>,
}

pub(crate) fn podman_invocation(data_root: &Path) -> Result<PodmanInvocation> {
    let bin = podman_binary_path(data_root)
        .ok_or_else(|| anyhow::anyhow!("podman binary unavailable"))?;

    // Keep Podman state deterministic and tied to the daemon data_root so wiping ctx state fully
    // resets container execution.
    //
    // Note: Podman expects XDG_RUNTIME_DIR to exist when set.
    let xdg_root = data_root.join("podman").join("xdg");
    let xdg_config = xdg_root.join("config");
    let xdg_data = xdg_root.join("data");
    let xdg_run = podman_runtime_root(data_root).join("run");
    let podman_home = podman_home_root(data_root);
    let podman_tmp_root = podman_temp_root(data_root);
    std::fs::create_dir_all(&xdg_config)
        .with_context(|| format!("create dir {}", xdg_config.display()))?;
    std::fs::create_dir_all(&xdg_data)
        .with_context(|| format!("create dir {}", xdg_data.display()))?;
    std::fs::create_dir_all(&xdg_run)
        .with_context(|| format!("create dir {}", xdg_run.display()))?;
    std::fs::create_dir_all(&podman_home)
        .with_context(|| format!("create dir {}", podman_home.display()))?;
    std::fs::create_dir_all(&podman_tmp_root)
        .with_context(|| format!("create dir {}", podman_tmp_root.display()))?;
    // Tight permissions: runtime dirs may contain sockets and are expected to be user-private.
    let _ = std::fs::set_permissions(&xdg_run, std::fs::Permissions::from_mode(0o700));
    let _ = std::fs::set_permissions(&podman_home, std::fs::Permissions::from_mode(0o700));

    let mut env = HashMap::new();
    env.insert(
        "XDG_CONFIG_HOME".to_string(),
        xdg_config.to_string_lossy().to_string(),
    );
    env.insert(
        "XDG_DATA_HOME".to_string(),
        xdg_data.to_string_lossy().to_string(),
    );
    env.insert(
        "XDG_RUNTIME_DIR".to_string(),
        xdg_run.to_string_lossy().to_string(),
    );
    // Isolate Podman machine key/material paths from host-global HOME to keep daemon-managed
    // runtimes deterministic and avoid cross-daemon key generation collisions.
    env.insert(
        "HOME".to_string(),
        podman_home.to_string_lossy().to_string(),
    );
    let tmp = podman_tmp_root.to_string_lossy().to_string();
    env.insert("TMPDIR".to_string(), tmp.clone());
    env.insert("TMP".to_string(), tmp.clone());
    env.insert("TEMP".to_string(), tmp);

    Ok(PodmanInvocation { bin, env })
}

pub(crate) fn podman_command(data_root: &Path) -> Result<Command> {
    let inv = podman_invocation(data_root)?;
    let mut cmd = Command::new(inv.bin);
    for (key, value) in inv.env {
        cmd.env(key, value);
    }
    Ok(cmd)
}

pub(crate) async fn command_output_with_timeout(
    mut cmd: Command,
    timeout: Duration,
) -> Result<std::process::Output> {
    // Avoid hanging forever when Podman (or its VM connection) wedges.
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let child = cmd.spawn().context("spawning command")?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(res) => Ok(res?),
        Err(_) => anyhow::bail!("command timed out after {}s", timeout.as_secs()),
    }
}

fn podman_machine_required() -> bool {
    cfg!(target_os = "macos") || cfg!(target_os = "windows")
}

pub fn container_runtime_available(data_root: &Path) -> bool {
    podman_available(data_root) || managed_podman_runtime_source().is_some()
}

pub async fn podman_engine_ready(data_root: &Path) -> Result<bool> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("info");
    match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
        Ok(out) => Ok(out.status.success()),
        Err(_) => Ok(false),
    }
}

async fn podman_machine_present(data_root: &Path) -> Result<bool> {
    let machine_name = ctx_podman_machine_name(data_root);
    // `inspect` is the cheapest existence check and avoids JSON schema drift.
    let mut cmd = podman_command(data_root)?;
    cmd.arg("machine").arg("inspect").arg(machine_name);
    let output = command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await?;
    Ok(output.status.success())
}

fn looks_like_missing_machine_error(message_lc: &str) -> bool {
    message_lc.contains("no such")
        || message_lc.contains("not found")
        || message_lc.contains("does not exist")
        || message_lc.contains("no machine")
}

fn looks_like_recoverable_machine_start_error(message_lc: &str) -> bool {
    message_lc.contains("already running")
        || message_lc.contains("already starting")
        || message_lc.contains("already started")
        || message_lc.contains("in progress")
        || message_lc.contains("timed out")
        || message_lc.contains("resource busy")
        || message_lc.contains("another process")
        || message_lc.contains("lock")
        || message_lc.contains("port conflict")
        || message_lc.contains("unable to connect to \"gvproxy\" socket")
        || message_lc.contains("reassigning")
        || message_lc.contains("exited unexpectedly")
        || message_lc.contains("address already in use")
}

fn looks_like_running_but_unreachable_machine_start_error(message_lc: &str) -> bool {
    message_lc.contains("already running")
        || message_lc.contains("already started")
        || message_lc.contains("unable to connect to \"gvproxy\" socket")
}

fn command_output_message(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    format!("{stderr}\n{stdout}").trim().to_string()
}

fn podman_machine_temp_state_paths(data_root: &Path, machine_name: &str) -> Vec<PathBuf> {
    let podman_tmp = podman_temp_root(data_root).join("podman");
    let podman_home = podman_home_root(data_root).join(".podman");
    vec![
        podman_tmp.join("gvproxy.pid"),
        podman_tmp.join(format!("{machine_name}-api.sock")),
        podman_tmp.join(format!("{machine_name}-gvproxy.sock")),
        podman_tmp.join(format!("{machine_name}.sock")),
        podman_home.join(format!("{machine_name}-api.sock")),
        podman_home.join(format!("{machine_name}-gvproxy.sock")),
    ]
}

fn clear_stale_podman_machine_temp_state(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    let mut removed: Vec<String> = Vec::new();
    for path in podman_machine_temp_state_paths(data_root, machine_name) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed.push(path.to_string_lossy().to_string()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "failed to clear stale podman temp state at {}: {err}",
                        path.display()
                    ),
                );
            }
        }
    }
    if !removed.is_empty() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            &format!("cleared stale podman temp state: {}", removed.join(", ")),
        );
    }
}

fn is_ctx_managed_podman_helper_process_command(
    command: &[String],
    data_root: &Path,
    machine_name: &str,
) -> bool {
    let Some(program) = command.first() else {
        return false;
    };
    let helper_name = Path::new(program)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if helper_name != "gvproxy" && helper_name != "vfkit" {
        return false;
    }

    let rendered = command.to_vec().join("\n");
    if !rendered.contains(machine_name) {
        return false;
    }

    let scoped_roots = [
        podman_runtime_root(data_root),
        podman_home_root(data_root),
        podman_temp_root(data_root),
        data_root.join("podman"),
    ];
    scoped_roots.iter().any(|root| {
        let root = root.to_string_lossy();
        rendered.contains(root.as_ref())
    })
}

fn collect_ctx_managed_podman_helper_pids<I>(
    processes: I,
    data_root: &Path,
    machine_name: &str,
) -> Vec<u32>
where
    I: IntoIterator<Item = (u32, Vec<String>)>,
{
    let mut pids = processes
        .into_iter()
        .filter_map(|(pid, command)| {
            is_ctx_managed_podman_helper_process_command(&command, data_root, machine_name)
                .then_some(pid)
        })
        .collect::<Vec<_>>();
    pids.sort_unstable();
    pids.dedup();
    pids
}

fn stale_ctx_managed_podman_helper_pids(data_root: &Path, machine_name: &str) -> Vec<u32> {
    let mut system = System::new();
    system.refresh_processes();
    collect_ctx_managed_podman_helper_pids(
        system
            .processes()
            .iter()
            .map(|(pid, process)| (pid.as_u32(), process.cmd().to_vec())),
        data_root,
        machine_name,
    )
}

fn kill_ctx_managed_podman_helper_processes(data_root: &Path, machine_name: &str) -> Vec<u32> {
    let pids = stale_ctx_managed_podman_helper_pids(data_root, machine_name);
    if pids.is_empty() {
        return Vec::new();
    }
    let mut system = System::new();
    system.refresh_processes();
    let mut killed = Vec::new();
    for pid in pids {
        if let Some(process) = system.process(Pid::from_u32(pid)) {
            if process.kill_with(Signal::Kill).unwrap_or(false) {
                killed.push(pid);
            }
        }
    }
    killed
}

fn cleanup_ctx_managed_podman_helper_processes(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) {
    let killed = kill_ctx_managed_podman_helper_processes(data_root, machine_name);
    if !killed.is_empty() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            &format!(
                "killed stale ctx-managed podman helper process(es) before recovery: {}",
                killed
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
}

async fn best_effort_start_machine_after_init(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let mut start = podman_command(data_root)?;
    start.arg("machine").arg("start").arg(machine_name);
    match command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let combined = format!("{stderr}\n{stdout}").trim().to_string();
            if !combined.is_empty() {
                *last_err = combined.clone();
            }
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("podman machine start after init returned non-zero: {combined}"),
            );
            Ok(())
        }
        Err(err) => {
            *last_err = err.to_string();
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &format!("podman machine start after init failed: {err}"),
            );
            Ok(())
        }
    }
}

async fn wait_for_podman_machine_ready(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
    success_message: &str,
    last_err: &mut String,
) -> Result<bool> {
    let deadline = tokio::time::Instant::now() + podman_machine_ready_timeout();
    while tokio::time::Instant::now() < deadline {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Info,
                    success_message,
                );
                persist_podman_machine_cache_to_shared_best_effort(data_root, observer).await;
                return Ok(true);
            }
            Ok(out) => {
                let combined = command_output_message(&out);
                if !combined.is_empty() {
                    *last_err = combined;
                }
            }
            Err(err) => *last_err = err.to_string(),
        }
        tokio::time::sleep(podman_machine_ready_poll_interval()).await;
    }
    Ok(false)
}

struct PodmanMachineInitOutcome {
    output: std::process::Output,
    continued_after_machine_present: bool,
}

async fn read_child_pipe<R>(mut reader: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).await?;
    Ok(buf)
}

async fn collect_child_output(
    status: std::process::ExitStatus,
    stdout_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<std::process::Output> {
    let stdout = stdout_task
        .await
        .context("joining podman machine init stdout capture")??;
    let stderr = stderr_task
        .await
        .context("joining podman machine init stderr capture")??;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn run_podman_machine_init(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<PodmanMachineInitOutcome> {
    let mut init = podman_command(data_root)?;
    init.arg("machine").arg("init").arg(machine_name);
    init.stdout(Stdio::piped());
    init.stderr(Stdio::piped());
    init.kill_on_drop(true);
    let mut child = init.spawn().context("spawning podman machine init")?;
    let stdout = child
        .stdout
        .take()
        .context("podman machine init stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .context("podman machine init stderr was not captured")?;
    let stdout_task = tokio::spawn(read_child_pipe(stdout));
    let stderr_task = tokio::spawn(read_child_pipe(stderr));
    let deadline = tokio::time::Instant::now() + PODMAN_MACHINE_INIT_TIMEOUT;
    let mut machine_present_since: Option<tokio::time::Instant> = None;

    loop {
        if let Some(status) = child
            .try_wait()
            .context("polling podman machine init process")?
        {
            return Ok(PodmanMachineInitOutcome {
                output: collect_child_output(status, stdout_task, stderr_task).await?,
                continued_after_machine_present: false,
            });
        }

        let machine_present = podman_machine_present(data_root).await.unwrap_or(false);
        if machine_present {
            let now = tokio::time::Instant::now();
            let present_since = machine_present_since.get_or_insert(now);
            if now.duration_since(*present_since) >= podman_machine_init_created_machine_grace() {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "podman machine init created machine state but did not exit; terminating init and continuing with explicit start",
                );
                let _ = child.start_kill();
                let status = child
                    .wait()
                    .await
                    .context("waiting for terminated podman machine init")?;
                return Ok(PodmanMachineInitOutcome {
                    output: collect_child_output(status, stdout_task, stderr_task).await?,
                    continued_after_machine_present: true,
                });
            }
        } else {
            machine_present_since = None;
        }

        if tokio::time::Instant::now() >= deadline {
            let _ = child.start_kill();
            let status = child
                .wait()
                .await
                .context("waiting for timed out podman machine init")?;
            let output = collect_child_output(status, stdout_task, stderr_task).await?;
            if machine_present {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    "podman machine init timed out after machine creation; continuing with explicit start",
                );
                return Ok(PodmanMachineInitOutcome {
                    output,
                    continued_after_machine_present: true,
                });
            }
            anyhow::bail!(
                "podman machine init timed out after {}s",
                PODMAN_MACHINE_INIT_TIMEOUT.as_secs()
            );
        }

        tokio::time::sleep(podman_machine_init_poll_interval()).await;
    }
}

async fn initialize_podman_machine(
    data_root: &Path,
    machine_name: &str,
    observer: Option<&dyn HarnessSetupObserver>,
    last_err: &mut String,
) -> Result<()> {
    let init_outcome = run_podman_machine_init(data_root, machine_name, observer)
        .await
        .context("podman machine init")?;
    let out = init_outcome.output;
    let combined = command_output_message(&out);
    if init_outcome.continued_after_machine_present {
        if !combined.is_empty() {
            *last_err = combined;
        }
    } else if !out.status.success() {
        let combined_lc = combined.to_ascii_lowercase();
        if combined_lc.contains("already exists") {
            let message = if combined.is_empty() {
                "podman machine init reported existing machine; starting it explicitly".to_string()
            } else {
                format!(
                    "podman machine init reported existing machine; starting it explicitly: {combined}"
                )
            };
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Warn,
                &message,
            );
            if !combined.is_empty() {
                *last_err = combined;
            }
        } else if combined.is_empty() {
            anyhow::bail!("podman machine init failed (status: {})", out.status);
        } else {
            anyhow::bail!("podman machine init failed: {combined}");
        }
    }

    best_effort_start_machine_after_init(data_root, machine_name, observer, last_err).await
}

async fn ensure_podman_machine_running_with_observer(
    data_root: &Path,
    observer: Option<&dyn HarnessSetupObserver>,
) -> Result<()> {
    let machine_name = ctx_podman_machine_name(data_root);
    let machine_lock = podman_machine_singleflight_lock(&machine_name);
    let machine_guard = match machine_lock.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "waiting for concurrent podman machine init/start operation",
            );
            machine_lock.lock().await
        }
    };
    if !podman_machine_required() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            "podman machine not required on this platform",
        );
        return Ok(());
    }
    seed_shared_podman_machine_cache_best_effort(data_root, observer).await;

    // If `podman info` works, we have a running engine connection.
    // Otherwise, capture the initial failure to reuse in error messages.
    let mut last_err = {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineCheck,
                    HarnessSetupLogLevel::Info,
                    "podman runtime is already reachable",
                );
                persist_podman_machine_cache_to_shared_best_effort(data_root, observer).await;
                return Ok(());
            }
            Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(err) => err.to_string(),
        }
    };

    observe_phase(
        observer,
        HarnessSetupPhase::MachineStartOrInit,
        "starting or initializing podman machine",
    );
    // Podman may leave stale temp state (gvproxy pid/socket markers) across crashed starts,
    // causing repeated "starting" or connection-refused loops. Clear these markers before start.
    clear_stale_podman_machine_temp_state(data_root, &machine_name, observer);

    // Prefer starting an existing machine; fall back to init when no machine exists.
    let mut wait_after_start = true;
    let mut force_recreate = false;
    let start_out = {
        let mut start = podman_command(data_root)?;
        start.arg("machine").arg("start").arg(&machine_name);
        command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await?
    };
    if start_out.status.success() {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "podman machine start command completed; waiting for readiness",
        );
    } else {
        let combined = command_output_message(&start_out);
        let combined_lc = combined.to_ascii_lowercase();

        let looks_like_missing_machine = looks_like_missing_machine_error(&combined_lc);

        if looks_like_missing_machine {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "podman machine not found; running init then explicit start",
            );
            initialize_podman_machine(data_root, &machine_name, observer, &mut last_err).await?;
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "podman machine initialized; waiting for readiness",
            );
        } else if looks_like_recoverable_machine_start_error(&combined_lc) {
            if looks_like_running_but_unreachable_machine_start_error(&combined_lc) {
                let message = if combined.is_empty() {
                    "podman machine start reported an already-running machine while podman remained unreachable; restarting once"
                        .to_string()
                } else {
                    format!(
                        "podman machine start reported an already-running machine while podman remained unreachable; restarting once: {combined}"
                    )
                };
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &message,
                );
                wait_after_start = false;
                force_recreate = true;
            } else {
                let message = if combined.is_empty() {
                    "podman machine start returned a recoverable error; waiting for readiness"
                        .to_string()
                } else {
                    format!(
                        "podman machine start returned recoverable error; waiting for readiness: {combined}"
                    )
                };
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &message,
                );
            }
            if !combined.is_empty() {
                last_err = combined;
            }
        } else {
            if combined.is_empty() {
                anyhow::bail!(
                    "podman machine start failed with non-zero exit {}",
                    start_out.status
                );
            }
            anyhow::bail!("podman machine start failed: {combined}");
        }
    }

    if wait_after_start {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "waiting for podman machine readiness after start",
        );
        if wait_for_podman_machine_ready(
            data_root,
            observer,
            "podman machine is ready",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    if !force_recreate {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            "podman machine remained unreachable after start; restarting once",
        );
        let stop_out = {
            let mut stop = podman_command(data_root)?;
            stop.arg("machine").arg("stop").arg(&machine_name);
            command_output_with_timeout(stop, PODMAN_MACHINE_START_TIMEOUT).await?
        };
        if !stop_out.status.success() {
            let combined = command_output_message(&stop_out);
            if !combined.is_empty() {
                last_err = combined.clone();
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!("podman machine stop returned non-zero during recovery: {combined}"),
                );
            }
        }
        let restart_out = {
            let mut start = podman_command(data_root)?;
            start.arg("machine").arg("start").arg(&machine_name);
            command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await?
        };
        if !restart_out.status.success() {
            let combined = command_output_message(&restart_out);
            if !combined.is_empty() {
                last_err = combined.clone();
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "podman machine start returned non-zero during restart recovery: {combined}"
                    ),
                );
            } else {
                observe_log(
                    observer,
                    HarnessSetupPhase::MachineStartOrInit,
                    HarnessSetupLogLevel::Warn,
                    &format!(
                        "podman machine start returned non-zero during restart recovery: {}",
                        restart_out.status
                    ),
                );
            }
        }
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Info,
            "waiting for podman machine readiness after restart",
        );
        if wait_for_podman_machine_ready(
            data_root,
            observer,
            "podman machine recovered after restart",
            &mut last_err,
        )
        .await?
        {
            return Ok(());
        }
    }

    // Final recovery: recreate the dedicated ctx machine once if it still cannot be reached.
    let machine_present = podman_machine_present(data_root).await.unwrap_or(false);
    if machine_present || force_recreate {
        observe_log(
            observer,
            HarnessSetupPhase::MachineStartOrInit,
            HarnessSetupLogLevel::Warn,
            if force_recreate {
                "podman machine reported an already-running but unreachable state; recreating machine"
            } else {
                "podman machine still unreachable after restart; recreating machine"
            },
        );
        cleanup_ctx_managed_podman_helper_processes(data_root, &machine_name, observer);
        clear_stale_podman_machine_temp_state(data_root, &machine_name, observer);

        if machine_present {
            let mut rm = podman_command(data_root)?;
            rm.arg("machine").arg("rm").arg("-f").arg(&machine_name);
            let rm_out = command_output_with_timeout(rm, PODMAN_MACHINE_START_TIMEOUT).await?;
            if !rm_out.status.success() {
                let combined = command_output_message(&rm_out);
                if !combined.is_empty() {
                    last_err = format!("podman machine rm -f failed: {combined}");
                }
            }
        }

        if let Err(err) =
            initialize_podman_machine(data_root, &machine_name, observer, &mut last_err).await
        {
            last_err = format!("podman machine init failed after recreate: {err:#}");
        } else {
            observe_log(
                observer,
                HarnessSetupPhase::MachineStartOrInit,
                HarnessSetupLogLevel::Info,
                "waiting for podman machine readiness after recreation",
            );
            if wait_for_podman_machine_ready(
                data_root,
                observer,
                "podman machine recovered after recreation",
                &mut last_err,
            )
            .await?
            {
                return Ok(());
            }
        }
    }

    drop(machine_guard);
    if last_err.trim().is_empty() {
        anyhow::bail!("podman machine remained unreachable after bounded recovery");
    }
    anyhow::bail!(
        "podman machine remained unreachable after bounded recovery: {}",
        last_err.trim()
    );
}

async fn container_exists(data_root: &Path, name: &str) -> Result<bool> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("container").arg("exists").arg(name);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    Ok(output.status.success())
}

async fn container_running(data_root: &Path, name: &str) -> Result<Option<bool>> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("container")
        .arg("inspect")
        .arg("--format")
        .arg("{{.State.Running}}")
        .arg(name);
    let output = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Some(stdout.trim() == "true"))
}

async fn ensure_workspace_volume(data_root: &Path, workspace_id: WorkspaceId) -> Result<String> {
    let name = format!("ctx-ws-{}", workspace_id.0);
    let mut inspect = podman_command(data_root)?;
    inspect.arg("volume").arg("inspect").arg(&name);
    let out = command_output_with_timeout(inspect, PODMAN_OP_TIMEOUT).await?;
    if out.status.success() {
        return Ok(name);
    }
    let mut create = podman_command(data_root)?;
    create.arg("volume").arg("create").arg(&name);
    let out = command_output_with_timeout(create, PODMAN_OP_TIMEOUT).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!(
                "podman volume create failed for {name} (status: {})",
                out.status
            );
        }
        anyhow::bail!("podman volume create failed for {name}: {combined}");
    }
    Ok(name)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PodmanInspectContainer {
    #[serde(default)]
    mounts: Vec<PodmanInspectMount>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PodmanInspectMount {
    #[serde(rename = "Type")]
    mount_type: Option<String>,
    name: Option<String>,
    destination: Option<String>,
}

async fn verify_disk_isolated_container_mounts(
    data_root: &Path,
    workspace: &Workspace,
    container_name: &str,
) -> Result<()> {
    let mut cmd = podman_command(data_root)?;
    cmd.arg("inspect").arg(container_name);
    let out = command_output_with_timeout(cmd, PODMAN_OP_TIMEOUT).await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        if combined.is_empty() {
            anyhow::bail!(
                "podman inspect failed for {container_name} (status: {})",
                out.status
            );
        }
        anyhow::bail!("podman inspect failed for {container_name}: {combined}");
    }

    let inspected: Vec<PodmanInspectContainer> = serde_json::from_slice(&out.stdout)
        .context("failed to parse podman inspect output as JSON")?;
    let container = inspected
        .into_iter()
        .next()
        .context("podman inspect returned empty output")?;

    let expected_vol = format!("ctx-ws-{}", workspace.id.0);
    let has_ws_volume = container.mounts.iter().any(|m| {
        m.mount_type.as_deref() == Some("volume")
            && m.destination.as_deref() == Some(CTX_CONTAINER_WORKSPACE_ROOT)
            && m.name.as_deref() == Some(expected_vol.as_str())
    });
    if !has_ws_volume {
        anyhow::bail!(
            "disk-isolated container {container_name} is missing expected volume mount: volume {expected_vol} -> {}",
            CTX_CONTAINER_WORKSPACE_ROOT
        );
    }

    // Guardrail: disk-isolated must not bind-mount the host workspace/worktrees into the container.
    let host_workspace_root = workspace.root_path.trim();
    let host_worktrees_root = worktrees_root(data_root)
        .join(workspace.id.0.to_string())
        .to_string_lossy()
        .to_string();
    let has_host_bind = container.mounts.iter().any(|m| {
        m.mount_type.as_deref() == Some("bind")
            && (m.destination.as_deref() == Some(host_workspace_root)
                || m.destination.as_deref() == Some(host_worktrees_root.as_str()))
    });
    if has_host_bind {
        anyhow::bail!(
            "disk-isolated container {container_name} unexpectedly bind-mounted host workspace/worktrees"
        );
    }

    Ok(())
}

#[cfg(unix)]
fn container_user() -> Option<String> {
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    Some(format!("{uid}:{gid}"))
}

#[cfg(not(unix))]
fn container_user() -> Option<String> {
    None
}

fn should_use_keep_id_userns() -> bool {
    cfg!(target_os = "linux")
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
    use ctx_core::ids::WorktreeId;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, Duration};

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

    fn env_var_test_lock() -> &'static StdMutex<()> {
        static LOCK: std::sync::OnceLock<StdMutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
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

    #[tokio::test]
    async fn container_mode_errors_when_podman_unavailable() {
        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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

    #[test]
    fn podman_binary_path_uses_env_override() {
        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &path);
        let resolved = podman_binary_path(Path::new("/tmp")).expect("env override should resolve");
        assert_eq!(resolved, tmp.path());
    }

    #[test]
    fn podman_invocation_sets_paths_under_short_runtime_root() {
        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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
        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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
        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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

        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn ensure_podman_machine_running_recreates_immediately_for_already_running_unreachable_machine(
    ) {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[tokio::test]
    async fn ensure_podman_machine_running_fails_fast_on_unknown_start_error() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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

        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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

        let mut last_err = String::new();
        initialize_podman_machine(temp.path(), "ctx-test-machine", None, &mut last_err)
            .await
            .expect("initialize machine");

        let log = std::fs::read_to_string(&log_path).expect("read invocation log");
        assert!(log.contains("machine init ctx-test-machine"));
        assert!(log.contains("machine start ctx-test-machine"));
        assert!(!log.contains("--now"));
        assert!(last_err.is_empty());
    }

    #[tokio::test]
    async fn initialize_podman_machine_terminates_stuck_init_when_machine_is_present() {
        use std::os::unix::fs::PermissionsExt;

        let _serial = env_var_test_lock()
            .lock()
            .unwrap_or_else(|err| err.into_inner());
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
                ensure_managed_default_container_image_tar_with_source(&root_a, &source_a, None)
                    .await
            }),
            tokio::spawn(async move {
                ensure_managed_default_container_image_tar_with_source(&root_b, &source_b, None)
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
