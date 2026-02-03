use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use tokio::process::Command;
use tokio::sync::Mutex;

use ctx_core::ids::WorkspaceId;
use ctx_core::models::{Workspace, Worktree};
use ctx_fs::worktrees::worktrees_root;
use serde::Serialize;

use crate::bundled_assets;
use crate::egress_proxy::EgressProxy;
use crate::settings::{
    ContainerExecutionSettings, ContainerMountMode, ContainerNetworkMode, ExecutionMode,
    ExecutionSettings,
};
use url::Url;

const DEFAULT_CONTAINER_IMAGE: &str = "ubuntu:22.04";
const PODMAN_PATH_ENV: &str = "CTX_PODMAN_PATH";
const PODMAN_ALLOW_SYSTEM_ENV: &str = "CTX_ALLOW_SYSTEM_PODMAN";

#[derive(Debug, Clone)]
pub enum HarnessRuntimeKind {
    Host,
    Container { name: String },
}

#[derive(Debug, Clone)]
pub struct HarnessExecutionPlan {
    pub runtime: HarnessRuntimeKind,
    pub env_overrides: HashMap<String, String>,
    pub sealed_root: Option<PathBuf>,
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

pub struct HarnessRuntimeManager {
    data_root: PathBuf,
    proxy: Arc<EgressProxy>,
    containers: Mutex<HashMap<WorkspaceId, HarnessContainer>>,
}

impl HarnessRuntimeManager {
    pub fn new(data_root: PathBuf, proxy: Arc<EgressProxy>) -> Self {
        Self {
            data_root,
            proxy,
            containers: Mutex::new(HashMap::new()),
        }
    }

    pub fn spawn_background_podman_machine_download(self: &Arc<Self>) {
        if !podman_machine_required() {
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
        if podman_machine_present().await? {
            return Ok(());
        }
        let status = podman_command()?
            .arg("machine")
            .arg("init")
            .status()
            .await?;
        if status.success() {
            return Ok(());
        }
        anyhow::bail!("podman machine init failed with status {status}");
    }

    pub async fn prepare(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ExecutionSettings,
        daemon_url: &str,
    ) -> Result<HarnessExecutionPlan> {
        let mode = resolve_execution_mode(settings);
        let allow_fallback = matches!(settings.mode, ExecutionMode::Auto);
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
        if matches!(mode, ExecutionMode::Host) {
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::Host,
                env_overrides,
                sealed_root: None,
            });
        }

        if !podman_available() {
            if !allow_fallback {
                anyhow::bail!(
                    "podman unavailable and execution mode is container; refusing host fallback"
                );
            }
            tracing::warn!("podman unavailable; falling back to host runtime (auto mode)");
            return Ok(HarnessExecutionPlan {
                runtime: HarnessRuntimeKind::Host,
                env_overrides,
                sealed_root: None,
            });
        }

        let proxy_addr = self.proxy.addr();
        let proxy_host = "host.containers.internal";
        let container = match self
            .ensure_container(
                workspace,
                worktree,
                &settings.container,
                proxy_host,
                proxy_addr.port(),
            )
            .await
        {
            Ok(container) => container,
            Err(err) => {
                tracing::warn!("failed to start harness container: {err:#}");
                if !allow_fallback {
                    anyhow::bail!(
                        "container runtime failed and execution mode is container: {err:#}"
                    );
                }
                return Ok(HarnessExecutionPlan {
                    runtime: HarnessRuntimeKind::Host,
                    env_overrides,
                    sealed_root: None,
                });
            }
        };

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

        let sealed_root = if matches!(container.mount_mode, ContainerMountMode::Sealed) {
            Some(sealed_worktrees_root(&self.data_root, workspace.id))
        } else {
            None
        };

        Ok(HarnessExecutionPlan {
            runtime: HarnessRuntimeKind::Container {
                name: container.name,
            },
            env_overrides,
            sealed_root,
        })
    }

    pub async fn sync_worktree_to_sealed(
        &self,
        host_root: &Path,
        sealed_root: &Path,
    ) -> Result<()> {
        if !rsync_available() {
            anyhow::bail!("rsync not available for sealed sync");
        }
        tokio::fs::create_dir_all(sealed_root).await.ok();
        let mut cmd = Command::new("rsync");
        cmd.arg("-a")
            .arg("--delete")
            .arg(format!("{}/", host_root.to_string_lossy()))
            .arg(sealed_root);
        let status = cmd.status().await?;
        if !status.success() {
            anyhow::bail!("rsync failed with status {}", status);
        }
        Ok(())
    }

    pub async fn sync_sealed_to_host(&self, sealed_root: &Path, host_root: &Path) -> Result<()> {
        if !rsync_available() {
            anyhow::bail!("rsync not available for sealed sync");
        }
        tokio::fs::create_dir_all(host_root).await.ok();
        let mut cmd = Command::new("rsync");
        cmd.arg("-a")
            .arg("--delete")
            .arg(format!("{}/", sealed_root.to_string_lossy()))
            .arg(host_root);
        let status = cmd.status().await?;
        if !status.success() {
            anyhow::bail!("rsync failed with status {}", status);
        }
        Ok(())
    }

    pub async fn container_status(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<HarnessContainerStatus>> {
        let name = format!("ctx-harness-{}", workspace_id.0);
        if !container_exists(&name).await? {
            return Ok(None);
        }
        let running = container_running(&name).await?.unwrap_or(false);
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
        if !container_exists(&name).await? {
            return Ok(false);
        }
        let mut containers = self.containers.lock().await;
        containers.remove(&workspace_id);
        let status = podman_command()?
            .arg("rm")
            .arg("-f")
            .arg(&name)
            .status()
            .await?;
        if status.success() {
            Ok(true)
        } else {
            anyhow::bail!("podman rm failed for {name}");
        }
    }

    async fn ensure_container(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        settings: &ContainerExecutionSettings,
        proxy_host: &str,
        proxy_port: u16,
    ) -> Result<HarnessContainer> {
        let name = format!("ctx-harness-{}", workspace.id.0);
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
        } else if matches!(settings.mount_mode, ContainerMountMode::Sealed)
            && container_exists(&name).await.unwrap_or(false)
        {
            recreate = true;
        }

        if recreate {
            if let Ok(mut cmd) = podman_command() {
                let _ = cmd.arg("rm").arg("-f").arg(&name).status().await;
            }
        }

        let exists = if recreate {
            false
        } else {
            container_exists(&name).await?
        };
        if exists {
            let running = container_running(&name).await?.unwrap_or(false);
            if !running {
                let status = podman_command()?.arg("start").arg(&name).status().await?;
                if !status.success() {
                    anyhow::bail!("podman start failed for {name}");
                }
            }
        } else {
            let mut cmd = podman_command()?;
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
            cmd.arg(resolve_container_image(settings));
            cmd.arg("/bin/sh")
                .arg("-c")
                .arg("while true; do sleep 100000; done");
            let status = cmd.status().await?;
            if !status.success() {
                anyhow::bail!("podman run failed for {name}");
            }
        }

        let egress_guard = if matches!(settings.network_mode, ContainerNetworkMode::All) {
            if let Err(err) = clear_egress_guard(&name).await {
                tracing::warn!("failed to clear egress guard: {err:#}");
            }
            false
        } else {
            configure_egress_guard(&name, proxy_host, proxy_port).await?
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

fn resolve_execution_mode(settings: &ExecutionSettings) -> ExecutionMode {
    match &settings.mode {
        ExecutionMode::Auto => {
            if cfg!(target_os = "linux") {
                ExecutionMode::Container
            } else {
                ExecutionMode::Host
            }
        }
        other => other.clone(),
    }
}

fn resolve_container_image(settings: &ContainerExecutionSettings) -> String {
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

fn container_data_root(data_root: &Path, workspace_id: WorkspaceId) -> PathBuf {
    data_root
        .join("containers")
        .join("workspaces")
        .join(workspace_id.0.to_string())
        .join("data")
}

fn sealed_worktrees_root(data_root: &Path, workspace_id: WorkspaceId) -> PathBuf {
    data_root
        .join("containers")
        .join("workspaces")
        .join(workspace_id.0.to_string())
        .join("sealed-worktrees")
}

struct MountPlan {
    mounts: Vec<String>,
    external_mounts: HashSet<String>,
}

fn build_mounts(
    data_root: &Path,
    workspace: &Workspace,
    worktree: &Worktree,
    settings: &ContainerExecutionSettings,
) -> MountPlan {
    let mut mounts = Vec::new();
    let mut external_mounts = HashSet::new();
    let workspace_root = PathBuf::from(&workspace.root_path);

    let worktrees_root = worktrees_root(data_root).join(workspace.id.0.to_string());
    if matches!(settings.mount_mode, ContainerMountMode::Sealed) {
        let sealed_root = sealed_worktrees_root(data_root, workspace.id);
        ensure_dir(&sealed_root);
        mounts.push(bind_mount(&sealed_root, &worktrees_root, false));

        let worktree_root = PathBuf::from(&worktree.root_path);
        if !worktree_root.starts_with(&worktrees_root) {
            let sealed_worktree = sealed_root.join(worktree.id.0.to_string());
            ensure_dir(&sealed_worktree);
            let mount = bind_mount(&sealed_worktree, &worktree_root, false);
            external_mounts.insert(mount.clone());
            mounts.push(mount);
        }
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

    MountPlan {
        mounts,
        external_mounts,
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

async fn configure_egress_guard(name: &str, proxy_host: &str, proxy_port: u16) -> Result<bool> {
    let script = format!(
        r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 43
fi
proxy_ip="$(getent hosts {proxy_host} | awk '{{print $1}}' | head -n1)"
if [ -z "$proxy_ip" ]; then
  exit 44
fi
iptables -F OUTPUT || true
iptables -P OUTPUT DROP
iptables -A OUTPUT -d "$proxy_ip" -p tcp --dport {proxy_port} -j ACCEPT
iptables -A OUTPUT -d 127.0.0.1/8 -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT
exit 0
"#
    );
    let status = podman_command()?
        .arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script)
        .status()
        .await?;
    if status.success() {
        return Ok(true);
    }
    if let Some(code) = status.code() {
        if code == 43 {
            anyhow::bail!("iptables missing in harness container");
        }
        if code == 44 {
            anyhow::bail!("proxy host not resolvable inside harness container");
        }
    }
    anyhow::bail!("failed to configure egress guard (status: {status})");
}

async fn clear_egress_guard(name: &str) -> Result<()> {
    let script = r#"
set -e
if ! command -v iptables >/dev/null 2>&1; then
  exit 0
fi
iptables -F OUTPUT || true
iptables -P OUTPUT ACCEPT || true
exit 0
"#;
    let status = podman_command()?
        .arg("exec")
        .arg("--user")
        .arg("0")
        .arg(name)
        .arg("sh")
        .arg("-c")
        .arg(script)
        .status()
        .await?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("failed to clear egress guard (status: {status})");
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

fn podman_command() -> Result<Command> {
    let path = podman_binary_path().ok_or_else(|| anyhow::anyhow!("podman binary unavailable"))?;
    Ok(Command::new(path))
}

fn podman_machine_required() -> bool {
    cfg!(target_os = "macos") || cfg!(target_os = "windows")
}

async fn podman_machine_present() -> Result<bool> {
    let output = podman_command()?
        .arg("machine")
        .arg("list")
        .arg("--format")
        .arg("json")
        .output()
        .await?;
    if !output.status.success() {
        return Ok(false);
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    Ok(value.as_array().map(|v| !v.is_empty()).unwrap_or(false))
}

async fn container_exists(name: &str) -> Result<bool> {
    let status = podman_command()?
        .arg("container")
        .arg("exists")
        .arg(name)
        .status()
        .await?;
    Ok(status.success())
}

async fn container_running(name: &str) -> Result<Option<bool>> {
    let output = podman_command()?
        .arg("container")
        .arg("inspect")
        .arg("--format")
        .arg("{{.State.Running}}")
        .arg(name)
        .output()
        .await?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Some(stdout.trim() == "true"))
}

fn rsync_available() -> bool {
    which::which("rsync").is_ok()
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
        let ops_events = crate::ops_events::OpsEvents::new(tmp.path().to_path_buf());
        let proxy = Arc::new(crate::egress_proxy::EgressProxy::spawn(ops_events).unwrap());
        HarnessRuntimeManager::new(tmp.path().to_path_buf(), proxy)
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

    #[tokio::test]
    async fn auto_mode_falls_back_to_host_when_podman_unavailable() {
        let _guard = EnvGuard::set("CTX_TEST_PODMAN_AVAILABLE", "0");
        let tmp = tempfile::tempdir().unwrap();
        let manager = runtime_manager(&tmp).await;
        let workspace = sample_workspace(&tmp);
        let worktree = sample_worktree(&tmp, workspace.id);
        let settings = ExecutionSettings {
            mode: ExecutionMode::Auto,
            container: ContainerExecutionSettings::default(),
        };

        let plan = manager
            .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:9999")
            .await
            .expect("auto mode should fall back to host");
        assert!(matches!(plan.runtime, HarnessRuntimeKind::Host));
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
