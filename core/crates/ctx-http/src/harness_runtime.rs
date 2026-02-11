use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::{fs, io::AsyncWriteExt};

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::worktrees_root;
use serde::{Deserialize, Serialize};

use crate::bundled_assets;
use crate::settings::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ExecutionMode,
    ExecutionSettings,
};
use url::Url;

// Default container image for ctx-managed execution.
//
// This must include:
// - iptables (for restricted egress enforcement)
// - /usr/local/bin/ctx-egress-proxy (Linux binary executed inside the container)
const DEFAULT_CONTAINER_IMAGE: &str = "ghcr.io/ctxrs/ctx-harness:ubuntu-24.04";
const PODMAN_PATH_ENV: &str = "CTX_PODMAN_PATH";
const PODMAN_ALLOW_SYSTEM_ENV: &str = "CTX_ALLOW_SYSTEM_PODMAN";
const EGRESS_PROXY_BINARY: &str = "ctx-egress-proxy";
const EGRESS_PROXY_RUNTIME_ID: &str = "ctx-egress-proxy";
const EGRESS_PROXY_CONFIG_NAME: &str = "egress-proxy.json";
const TRANSPARENT_PROXY_PORT: u16 = 15001;
const EGRESS_PROXY_CONTAINER_PATH: &str = "/usr/local/bin/ctx-egress-proxy";
// Dedicated Podman machine name for ctx-managed container execution on macOS/Windows.
//
// We intentionally do not use the user's default machine name to avoid collisions and to keep
// ctx-managed behavior deterministic.
const CTX_PODMAN_MACHINE_NAME: &str = "ctx";
// In-container root for disk-isolated workspaces (Podman volume mounted here).
pub(crate) const CTX_CONTAINER_WORKSPACE_ROOT: &str = "/ctx/ws";
const PODMAN_INFO_TIMEOUT: Duration = Duration::from_secs(5);
const PODMAN_MACHINE_START_TIMEOUT: Duration = Duration::from_secs(180);
const PODMAN_MACHINE_INIT_TIMEOUT: Duration = Duration::from_secs(30 * 60);
// First boot can be slow on fresh installs (image download + provisioning).
const PODMAN_MACHINE_READY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const PODMAN_OP_TIMEOUT: Duration = Duration::from_secs(60);
const PODMAN_LOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
        if !podman_available() {
            anyhow::bail!("podman unavailable");
        }
        if podman_machine_present(&self.data_root).await? {
            return Ok(());
        }
        let mut cmd = podman_command(&self.data_root)?;
        cmd.arg("machine").arg("init").arg(CTX_PODMAN_MACHINE_NAME);
        let output = command_output_with_timeout(cmd, PODMAN_MACHINE_INIT_TIMEOUT).await?;
        if output.status.success() {
            return Ok(());
        }
        anyhow::bail!(
            "podman machine init failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
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
        if let Some(path) = podman_binary_path() {
            env_overrides.insert(
                PODMAN_PATH_ENV.to_string(),
                path.to_string_lossy().to_string(),
            );
        }
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::Host,
                env_overrides,
            });
        }

        if !podman_available() {
            anyhow::bail!("podman unavailable and execution mode is container");
        }

        let proxy_host = "host.containers.internal";
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let container = self
            .ensure_container(
                workspace,
                Some(worktree),
                &settings.container,
                proxy_host,
                daemon_port,
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
        if matches!(settings.mode, ExecutionMode::Host) {
            return Ok(());
        }
        if !podman_available() {
            anyhow::bail!("podman unavailable and execution mode is container");
        }
        let proxy_host = "host.containers.internal";
        let daemon_port = daemon_port_from_url(daemon_url).unwrap_or(4399);
        let _ = self
            .ensure_container(
                workspace,
                None,
                &settings.container,
                proxy_host,
                daemon_port,
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
    ) -> Result<HarnessContainer> {
        let name = format!("ctx-harness-{}", workspace.id.0);
        let image = resolve_container_image(settings);
        // On macOS/Windows the engine is a VM; ensure it is running before we run any podman ops.
        ensure_podman_machine_running(&self.data_root).await?;
        if matches!(settings.mount_mode, ContainerMountMode::DiskIsolated) {
            let _ = ensure_workspace_volume(&self.data_root, workspace.id).await?;
        }
        let mount_plan = build_mounts(&self.data_root, workspace, worktree, settings);
        let mut containers = self.containers.lock().await;
        let mut recreate = false;
        if let Some(container) = containers.get(&workspace.id) {
            if container.mount_mode != settings.mount_mode
                || container.external_mounts != mount_plan.external_mounts
            {
                recreate = true;
            } else {
                return Ok(container.clone());
            }
        }

        if recreate {
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
            }
        } else {
            ensure_container_image_available(&self.data_root, &image).await?;
            let mut cmd = podman_command(&self.data_root)?;
            cmd.arg("run").arg("-d").arg("--name").arg(&name);
            cmd.arg("--userns=keep-id");
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

        let egress_guard = if matches!(settings.network_mode, ContainerNetworkMode::All) {
            if let Err(err) = stop_transparent_proxy(&self.data_root, &name).await {
                tracing::warn!("failed to stop transparent proxy: {err:#}");
            }
            if let Err(err) = clear_egress_guard(&self.data_root, &name).await {
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
            let proxy_config = TransparentProxyConfig {
                listen: format!("127.0.0.1:{TRANSPARENT_PROXY_PORT}"),
                mode: settings.network_mode.clone(),
                allowlist: settings.allowlist.clone(),
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

pub async fn prefetch_container_image(data_root: &Path, image: &str) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }
    // On macOS/Windows `podman` is a remote client; image ops require a running machine.
    ensure_podman_machine_running(data_root).await?;
    ensure_container_image_available(data_root, image).await
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

async fn ensure_container_image_available(data_root: &Path, image: &str) -> Result<()> {
    let image = image.trim();
    if image.is_empty() {
        anyhow::bail!("image is required");
    }

    if container_image_present(data_root, image).await? {
        return Ok(());
    }

    // Restricted networking is security-relevant; ctx must not silently fall back to pulling from
    // a registry. For the default harness image, we only support deterministic loading from a
    // bundled tarball.
    if image == DEFAULT_CONTAINER_IMAGE {
        let tar = bundled_assets::bundled_ctx_harness_image_tar(image).ok_or_else(|| {
            anyhow::anyhow!(
                "default harness image '{}' is not present and no bundled image tar was found (set CTX_BUNDLE_DIR or pre-load the image into podman)",
                image
            )
        })?;
        let mut cmd = podman_command(data_root)?;
        cmd.arg("load").arg("-i").arg(&tar);
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

    anyhow::bail!(
        "container image '{}' is not present; registry pulls are disabled, so the image must already exist in podman",
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
            })
        }
    };
    let output = match output {
        Ok(out) => out,
        Err(err) => {
            return Ok(ContainerImageStatus {
                present: false,
                available: false,
                error: Some(err.to_string()),
            })
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
    // When providers are spawned via `podman exec` (container execution), those paths must be
    // visible inside the harness container as well.
    if let Ok(raw) = std::env::var("CTX_BUNDLE_DIR") {
        let bundle_dir = PathBuf::from(raw.trim());
        if bundle_dir.exists() {
            mounts.push(bind_mount(&bundle_dir, &bundle_dir, true));
        }
    }

    MountPlan {
        mounts,
        external_mounts: HashSet::new(),
    }
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

fn podman_available() -> bool {
    if cfg!(test) {
        if let Ok(value) = std::env::var("CTX_TEST_PODMAN_AVAILABLE") {
            let value = value.trim().to_ascii_lowercase();
            return matches!(value.as_str(), "1" | "true" | "yes" | "y");
        }
    }
    podman_binary_path().is_some()
}

fn podman_binary_path() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var(PODMAN_PATH_ENV) {
        let path = PathBuf::from(raw.trim());
        if path.exists() {
            return Some(path);
        }
    }
    if let Some(bundled) = bundled_assets::bundled_podman_runtime() {
        return Some(bundled.bin);
    }
    if allow_system_podman() {
        return which::which("podman").ok();
    }
    None
}

fn allow_system_podman() -> bool {
    std::env::var(PODMAN_ALLOW_SYSTEM_ENV)
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("yes"))
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
pub(crate) struct PodmanInvocation {
    pub(crate) bin: PathBuf,
    pub(crate) env: HashMap<String, String>,
}

pub(crate) fn podman_invocation(data_root: &Path) -> Result<PodmanInvocation> {
    let bin = podman_binary_path().ok_or_else(|| anyhow::anyhow!("podman binary unavailable"))?;

    // Keep Podman state deterministic and tied to the daemon data_root so wiping ctx state fully
    // resets container execution.
    //
    // Note: Podman expects XDG_RUNTIME_DIR to exist when set.
    let xdg_root = data_root.join("podman").join("xdg");
    let xdg_config = xdg_root.join("config");
    let xdg_data = xdg_root.join("data");
    let xdg_run = xdg_root.join("run");
    std::fs::create_dir_all(&xdg_config)
        .with_context(|| format!("create dir {}", xdg_config.display()))?;
    std::fs::create_dir_all(&xdg_data)
        .with_context(|| format!("create dir {}", xdg_data.display()))?;
    std::fs::create_dir_all(&xdg_run)
        .with_context(|| format!("create dir {}", xdg_run.display()))?;
    // Tight permissions: runtime dirs may contain sockets and are expected to be user-private.
    let _ = std::fs::set_permissions(&xdg_run, std::fs::Permissions::from_mode(0o700));

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

async fn podman_machine_present(data_root: &Path) -> Result<bool> {
    // `inspect` is the cheapest existence check and avoids JSON schema drift.
    let mut cmd = podman_command(data_root)?;
    cmd.arg("machine")
        .arg("inspect")
        .arg(CTX_PODMAN_MACHINE_NAME);
    let output = command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await?;
    Ok(output.status.success())
}

async fn ensure_podman_machine_running(data_root: &Path) -> Result<()> {
    if !podman_machine_required() {
        return Ok(());
    }

    // If `podman info` works, we have a running engine connection.
    // Otherwise, capture the initial failure to reuse in error messages.
    let mut last_err = {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(err) => err.to_string(),
        }
    };

    // Prefer starting an existing machine; fall back to init when no machine exists.
    let start_out = {
        let mut start = podman_command(data_root)?;
        start
            .arg("machine")
            .arg("start")
            .arg(CTX_PODMAN_MACHINE_NAME);
        command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await?
    };
    if !start_out.status.success() {
        let start_stderr = String::from_utf8_lossy(&start_out.stderr)
            .trim()
            .to_string();
        let start_stdout = String::from_utf8_lossy(&start_out.stdout)
            .trim()
            .to_string();
        let combined = format!("{start_stderr}\n{start_stdout}").trim().to_string();
        let combined_lc = combined.to_ascii_lowercase();

        let looks_like_missing_machine = combined_lc.contains("no such")
            || combined_lc.contains("not found")
            || combined_lc.contains("does not exist")
            || combined_lc.contains("no machine");

        if looks_like_missing_machine {
            let mut init = podman_command(data_root)?;
            init.arg("machine")
                .arg("init")
                .arg("--now")
                .arg(CTX_PODMAN_MACHINE_NAME);
            let out = command_output_with_timeout(init, PODMAN_MACHINE_INIT_TIMEOUT).await?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let combined = format!("{stderr}\n{stdout}").trim().to_string();
                let init_lc = combined.to_ascii_lowercase();
                if init_lc.contains("already exists") {
                    // Race / stale detection: machine exists after all, retry start once.
                    let mut start2 = podman_command(data_root)?;
                    start2
                        .arg("machine")
                        .arg("start")
                        .arg(CTX_PODMAN_MACHINE_NAME);
                    let out = command_output_with_timeout(start2, PODMAN_MACHINE_START_TIMEOUT)
                        .await
                        .context("podman machine start (after init already exists)")?;
                    if !out.status.success() {
                        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        let combined = format!("{stderr}\n{stdout}").trim().to_string();
                        anyhow::bail!("podman machine start failed: {combined}");
                    }
                } else {
                    anyhow::bail!("podman machine init --now failed: {combined}");
                }
            }
        } else {
            anyhow::bail!("podman machine start failed: {combined}");
        }
    }

    // Wait for the engine connection to become healthy (bounded by PODMAN_MACHINE_READY_TIMEOUT).
    let deadline = tokio::time::Instant::now() + PODMAN_MACHINE_READY_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => last_err = String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(err) => last_err = err.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // Recovery: if the machine is in a wedged "running but unreachable" state, a stop/start can
    // re-establish the socket.
    let _ = {
        let mut stop = podman_command(data_root)?;
        stop.arg("machine").arg("stop").arg(CTX_PODMAN_MACHINE_NAME);
        command_output_with_timeout(stop, PODMAN_MACHINE_START_TIMEOUT).await
    };
    let _ = {
        let mut start = podman_command(data_root)?;
        start
            .arg("machine")
            .arg("start")
            .arg(CTX_PODMAN_MACHINE_NAME);
        command_output_with_timeout(start, PODMAN_MACHINE_START_TIMEOUT).await
    };
    let deadline = tokio::time::Instant::now() + PODMAN_MACHINE_READY_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        let mut cmd = podman_command(data_root)?;
        cmd.arg("info");
        match command_output_with_timeout(cmd, PODMAN_INFO_TIMEOUT).await {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => last_err = String::from_utf8_lossy(&out.stderr).trim().to_string(),
            Err(err) => last_err = err.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    if last_err.trim().is_empty() {
        anyhow::bail!("podman machine start completed but podman is still unreachable");
    }
    anyhow::bail!(
        "podman machine start completed but podman is still unreachable: {}",
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use ctx_core::ids::WorktreeId;
    use tempfile::TempDir;

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
            bootstrap_config_path: None,
            bootstrap_config_key: None,
            bootstrap_command: None,
            bootstrap_script_path: None,
        }
    }

    async fn runtime_manager(tmp: &TempDir) -> HarnessRuntimeManager {
        HarnessRuntimeManager::new(tmp.path().to_path_buf())
    }

    #[tokio::test]
    async fn container_mode_errors_when_podman_unavailable() {
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
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_string_lossy().to_string();
        let _guard = EnvGuard::set("CTX_PODMAN_PATH", &path);
        let resolved = podman_binary_path().expect("env override should resolve");
        assert_eq!(resolved, tmp.path());
    }
}
