use std::env;
use std::io::{BufRead, BufReader, ErrorKind};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context as AnyhowContext, Result};
use gpui::{AsyncApp, Context, WeakEntity};
use serde::Deserialize;

use ctx_providers::adapters::ProviderHealth;

use super::ShellView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LauncherHostKind {
    Local,
    Ssh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LauncherExecutionMode {
    Host,
    Container,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LauncherStep {
    Host,
    Mode,
    Workspace,
    Review,
    Progress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LauncherProgressStatus {
    Pending,
    Running,
    Done,
    Error,
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LauncherProgressId {
    Daemon,
    Workspace,
    Worktree,
    Harness,
}

#[derive(Clone, Debug)]
pub(crate) struct LauncherProgressItem {
    pub(crate) id: LauncherProgressId,
    pub(crate) label: &'static str,
    pub(crate) status: LauncherProgressStatus,
    pub(crate) detail: Option<String>,
}

pub(crate) const LAUNCHER_STEPS: [LauncherStep; 5] = [
    LauncherStep::Host,
    LauncherStep::Mode,
    LauncherStep::Workspace,
    LauncherStep::Review,
    LauncherStep::Progress,
];

pub(crate) fn default_launcher_progress_items() -> Vec<LauncherProgressItem> {
    vec![
        LauncherProgressItem {
            id: LauncherProgressId::Daemon,
            label: "Start daemon",
            status: LauncherProgressStatus::Pending,
            detail: None,
        },
        LauncherProgressItem {
            id: LauncherProgressId::Workspace,
            label: "Register workspace",
            status: LauncherProgressStatus::Pending,
            detail: None,
        },
        LauncherProgressItem {
            id: LauncherProgressId::Worktree,
            label: "Prepare worktree",
            status: LauncherProgressStatus::Pending,
            detail: None,
        },
        LauncherProgressItem {
            id: LauncherProgressId::Harness,
            label: "Check harnesses",
            status: LauncherProgressStatus::Pending,
            detail: None,
        },
    ]
}

impl LauncherExecutionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LauncherExecutionMode::Host => "host",
            LauncherExecutionMode::Container => "container",
        }
    }

    pub(crate) fn from_str(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "host" => Some(LauncherExecutionMode::Host),
            "container" => Some(LauncherExecutionMode::Container),
            _ => None,
        }
    }

    pub(crate) fn default_for_platform() -> Self {
        if cfg!(target_os = "linux") {
            LauncherExecutionMode::Container
        } else {
            LauncherExecutionMode::Host
        }
    }
}

impl LauncherStep {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LauncherStep::Host => "Host",
            LauncherStep::Mode => "Mode",
            LauncherStep::Workspace => "Workspace",
            LauncherStep::Review => "Review",
            LauncherStep::Progress => "Progress",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            LauncherStep::Host => 0,
            LauncherStep::Mode => 1,
            LauncherStep::Workspace => 2,
            LauncherStep::Review => 3,
            LauncherStep::Progress => 4,
        }
    }
}

impl LauncherProgressStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LauncherProgressStatus::Pending => "Pending",
            LauncherProgressStatus::Running => "Working",
            LauncherProgressStatus::Done => "Done",
            LauncherProgressStatus::Error => "Failed",
            LauncherProgressStatus::Skipped => "Skipped",
        }
    }
}

impl ShellView {
    pub(crate) fn launcher_supports_container(&self) -> bool {
        cfg!(target_os = "linux")
    }

    pub(crate) fn launcher_set_host_kind(
        &mut self,
        kind: LauncherHostKind,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_host_kind == kind {
            return;
        }
        self.launcher_host_kind = kind;
        self.launcher_reset_errors();
        cx.notify();
    }

    pub(crate) fn launcher_set_execution_mode(
        &mut self,
        mode: LauncherExecutionMode,
        cx: &mut Context<Self>,
    ) {
        if self.launcher_execution_mode == mode {
            return;
        }
        self.launcher_execution_mode = mode;
        self.ui_state.set_launcher_execution_mode(mode.as_str());
        self.launcher_reset_errors();
        cx.notify();
    }

    pub(crate) fn launcher_next(&mut self, cx: &mut Context<Self>) {
        self.launcher_reset_errors();
        match self.launcher_step {
            LauncherStep::Host => {
                if self.launcher_host_kind != LauncherHostKind::Local {
                    self.launcher_error = Some("Remote hosts are not available yet.".to_string());
                    self.launcher_error_step = Some(LauncherStep::Host);
                    cx.notify();
                    return;
                }
                self.launcher_step = LauncherStep::Mode;
            }
            LauncherStep::Mode => {
                if self.launcher_execution_mode == LauncherExecutionMode::Container
                    && !self.launcher_supports_container()
                {
                    self.launcher_error =
                        Some("Container mode is currently supported only on Linux.".to_string());
                    self.launcher_error_step = Some(LauncherStep::Mode);
                    cx.notify();
                    return;
                }
                self.launcher_step = LauncherStep::Workspace;
            }
            LauncherStep::Workspace => {
                if self.launcher_workspace_path.trim().is_empty() {
                    self.launcher_error =
                        Some("Select a workspace folder to continue.".to_string());
                    self.launcher_error_step = Some(LauncherStep::Workspace);
                    cx.notify();
                    return;
                }
                self.launcher_step = LauncherStep::Review;
            }
            _ => {}
        }
        cx.notify();
    }

    pub(crate) fn launcher_back(&mut self, cx: &mut Context<Self>) {
        self.launcher_reset_errors();
        let index = self.launcher_step.index();
        if index == 0 {
            return;
        }
        if let Some(step) = LAUNCHER_STEPS.get(index.saturating_sub(1)) {
            self.launcher_step = *step;
        }
        cx.notify();
    }

    pub(crate) fn launcher_go_to_step(&mut self, step: LauncherStep, cx: &mut Context<Self>) {
        self.launcher_step = step;
        self.launcher_reset_errors();
        cx.notify();
    }

    pub(crate) fn launcher_switch_to_host_mode(&mut self, cx: &mut Context<Self>) {
        self.launcher_execution_mode = LauncherExecutionMode::Host;
        self.ui_state.set_launcher_execution_mode(self.launcher_execution_mode.as_str());
        self.launcher_step = LauncherStep::Mode;
        self.launcher_reset_errors();
        cx.notify();
    }

    pub(crate) fn launcher_open_workspace(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self.launcher_workspace_id else {
            return;
        };
        self.set_route(super::ShellRoute::Workbench, cx);
        self.load_workspace(workspace_id, cx);
    }

    pub(crate) fn launcher_start(&mut self, cx: &mut Context<Self>) {
        if self.launcher_busy {
            return;
        }
        self.launcher_reset_errors();
        if self.launcher_host_kind != LauncherHostKind::Local {
            self.launcher_error = Some("Remote hosts are not available yet.".to_string());
            self.launcher_error_step = Some(LauncherStep::Host);
            self.launcher_step = LauncherStep::Host;
            cx.notify();
            return;
        }
        if self.launcher_execution_mode == LauncherExecutionMode::Container
            && !self.launcher_supports_container()
        {
            self.launcher_error =
                Some("Container mode is currently supported only on Linux.".to_string());
            self.launcher_error_step = Some(LauncherStep::Mode);
            self.launcher_step = LauncherStep::Mode;
            cx.notify();
            return;
        }
        let workspace_path_raw = self.launcher_workspace_path.trim().to_string();
        if workspace_path_raw.is_empty() {
            self.launcher_error = Some("Select a workspace folder to continue.".to_string());
            self.launcher_error_step = Some(LauncherStep::Workspace);
            self.launcher_step = LauncherStep::Workspace;
            cx.notify();
            return;
        }
        let normalized_workspace_root = match normalize_workspace_root(&workspace_path_raw) {
            Ok(root) => root,
            Err(err) => {
                self.launcher_error = Some(err.to_string());
                self.launcher_error_step = Some(LauncherStep::Workspace);
                self.launcher_step = LauncherStep::Workspace;
                cx.notify();
                return;
            }
        };
        let workspace_path = normalized_workspace_root.to_string_lossy().to_string();

        self.launcher_busy = true;
        self.launcher_step = LauncherStep::Progress;
        self.launcher_progress = default_launcher_progress_items();
        self.launcher_workspace_id = None;
        self.ui_state
            .set_launcher_workspace_path(Some(&workspace_path));
        self.ui_state
            .set_launcher_execution_mode(self.launcher_execution_mode.as_str());
        self.update_launcher_progress(
            LauncherProgressId::Daemon,
            LauncherProgressStatus::Running,
            Some(if self.launcher_execution_mode == LauncherExecutionMode::Container {
                "Launching containerized daemon".to_string()
            } else {
                "Launching daemon".to_string()
            }),
        );
        cx.notify();

        let execution_mode = self.launcher_execution_mode;
        cx.spawn(move |this: WeakEntity<ShellView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                let workspace_path_for_connect = workspace_path.clone();
                let connect_join = tokio::task::spawn_blocking(move || {
                    let roots = if execution_mode == LauncherExecutionMode::Container {
                        Some(vec![normalize_workspace_root(&workspace_path_for_connect)?])
                    } else {
                        None
                    };
                    desktop_connect_local(DesktopConnectLocalReq {
                        launch_mode: execution_mode,
                        workspace_roots: roots,
                    })
                })
                .await;
                let connect_result = match connect_join {
                    Ok(result) => result,
                    Err(err) => Err(anyhow!("daemon start task cancelled: {err}")),
                };

                let info = match connect_result {
                    Ok(info) => info,
                    Err(err) => {
                        let message = err.to_string();
                        let _ = this.update(&mut cx, |view, cx| {
                            view.update_launcher_progress(
                                LauncherProgressId::Daemon,
                                LauncherProgressStatus::Error,
                                Some(message.clone()),
                            );
                            view.launcher_error = Some(message);
                            view.launcher_error_step = Some(LauncherStep::Mode);
                            view.launcher_busy = false;
                            cx.notify();
                        });
                        return;
                    }
                };

                let _ = this.update(&mut cx, |view, cx| {
                    view.base_url = info.base_url.clone();
                    view.update_launcher_progress(
                        LauncherProgressId::Daemon,
                        LauncherProgressStatus::Done,
                        Some(format!("Connected to {}", info.base_url)),
                    );
                    view.update_launcher_progress(
                        LauncherProgressId::Workspace,
                        LauncherProgressStatus::Running,
                        None,
                    );
                    cx.notify();
                });

                let workspace_result = create_or_open_workspace_by_path(&workspace_path).await;

                let workspace = match workspace_result {
                    Ok(workspace) => workspace,
                    Err(err) => {
                        let message = err.to_string();
                        let _ = this.update(&mut cx, |view, cx| {
                            view.update_launcher_progress(
                                LauncherProgressId::Workspace,
                                LauncherProgressStatus::Error,
                                Some(message.clone()),
                            );
                            view.launcher_error = Some(message);
                            view.launcher_error_step = Some(LauncherStep::Workspace);
                            view.launcher_busy = false;
                            cx.notify();
                        });
                        return;
                    }
                };

                let workspace_id = workspace.id;
                let workspace_root = workspace.root_path.clone();

                let _ = this.update(&mut cx, |view, cx| {
                    if !view
                        .workspaces
                        .iter()
                        .any(|entry| entry.id == workspace_id)
                    {
                        view.workspaces.push(super::workspace::WorkspaceItem {
                            id: workspace_id,
                            name: workspace.name.clone(),
                            root_path: workspace.root_path.clone(),
                        });
                    }
                    view.launcher_workspace_id = Some(workspace_id);
                    view.selected_workspace = Some(workspace_id);
                    view.start_data_load(cx);
                    view.update_launcher_progress(
                        LauncherProgressId::Workspace,
                        LauncherProgressStatus::Done,
                        Some(workspace_root),
                    );
                    view.update_launcher_progress(
                        LauncherProgressId::Worktree,
                        LauncherProgressStatus::Done,
                        Some("Created when you start your first task.".to_string()),
                    );
                    view.update_launcher_progress(
                        LauncherProgressId::Harness,
                        LauncherProgressStatus::Running,
                        None,
                    );
                    cx.notify();
                });

                let provider_result = match ctx_client::resolve_daemon_config()
                    .and_then(ctx_client::Client::new)
                {
                    Ok(client) => client.list_providers().await,
                    Err(err) => Err(err),
                };

                let _ = this.update(&mut cx, |view, cx| {
                    match provider_result {
                        Ok(providers) => {
                            let needs_setup = providers
                                .iter()
                                .filter(|provider| {
                                    let install_supported = provider
                                        .details
                                        .get("install_supported")
                                        .map(|value| value == "true")
                                        .unwrap_or(false);
                                    install_supported
                                        && (!provider.installed
                                            || !matches!(provider.health, ProviderHealth::Ok))
                                })
                                .count();
                            let detail = if needs_setup > 0 {
                                format!(
                                    "{} harness{} need setup in Settings.",
                                    needs_setup,
                                    if needs_setup == 1 { "" } else { "es" }
                                )
                            } else {
                                "All harnesses ready.".to_string()
                            };
                            view.update_launcher_progress(
                                LauncherProgressId::Harness,
                                LauncherProgressStatus::Done,
                                Some(detail),
                            );
                        }
                        Err(_) => {
                            view.update_launcher_progress(
                                LauncherProgressId::Harness,
                                LauncherProgressStatus::Skipped,
                                Some("Check harnesses later in Settings.".to_string()),
                            );
                        }
                    }
                    view.launcher_busy = false;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn launcher_reset_errors(&mut self) {
        self.launcher_error = None;
        self.launcher_error_step = None;
    }

    fn update_launcher_progress(
        &mut self,
        id: LauncherProgressId,
        status: LauncherProgressStatus,
        detail: Option<String>,
    ) {
        for item in &mut self.launcher_progress {
            if item.id == id {
                item.status = status;
                item.detail = detail;
                return;
            }
        }
    }
}

#[derive(Debug)]
struct DesktopConnectLocalReq {
    launch_mode: LauncherExecutionMode,
    workspace_roots: Option<Vec<PathBuf>>,
}

#[derive(Debug)]
struct DesktopConnectionInfo {
    base_url: String,
    #[allow(dead_code)]
    token: String,
}

#[derive(Debug, Deserialize)]
struct DaemonAuthFile {
    token: String,
    #[serde(default)]
    daemon_url: Option<String>,
}

const DAEMON_AUTH_FILENAME: &str = "daemon_auth.json";
const DAEMON_AUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);
const DAEMON_AUTH_RETRY_DELAY: Duration = Duration::from_millis(200);

fn desktop_connect_local(req: DesktopConnectLocalReq) -> Result<DesktopConnectionInfo> {
    let data_dir = daemon_data_dir()?;
    let launch_mode = req.launch_mode;
    let (url, _child) = spawn_daemon(&data_dir, launch_mode, req.workspace_roots)?;
    let auth = read_daemon_auth_with_retry(&data_dir)?;
    let base_url = auth
        .daemon_url
        .as_ref()
        .map(|raw| raw.trim())
        .filter(|raw| !raw.is_empty())
        .map(|raw| raw.to_string())
        .unwrap_or(url);
    Ok(DesktopConnectionInfo {
        base_url,
        token: auth.token,
    })
}

async fn create_or_open_workspace_by_path(
    root_path: &str,
) -> Result<ctx_core::models::Workspace> {
    let root_path = root_path.trim();
    if root_path.is_empty() {
        return Err(anyhow!("root_path is required"));
    }
    let normalized = normalize_workspace_root(root_path)?;
    let normalized_owned = normalized.to_string_lossy().to_string();

    let config = ctx_client::resolve_daemon_config()?;
    let client = ctx_client::Client::new(config)?;
    let workspaces = client.list_workspaces().await?;
    if let Some(hit) = workspaces
        .iter()
        .find(|ws| ws.root_path == normalized_owned || ws.root_path == root_path)
    {
        return Ok(hit.clone());
    }
    client
        .create_workspace(&ctx_client::CreateWorkspaceRequest {
            root_path: normalized_owned,
            name: None,
        })
        .await
}

fn daemon_data_dir() -> Result<PathBuf> {
    if let Ok(value) = env::var("CTX_DATA_DIR") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    let base = directories::BaseDirs::new().context("resolving home directory")?;
    Ok(base.home_dir().join(".ctx"))
}

fn spawn_daemon(
    data_dir: &Path,
    launch_mode: LauncherExecutionMode,
    workspace_roots: Option<Vec<PathBuf>>,
) -> Result<(String, Child)> {
    match launch_mode {
        LauncherExecutionMode::Host => spawn_daemon_host(data_dir),
        LauncherExecutionMode::Container => {
            #[cfg(target_os = "linux")]
            {
                spawn_daemon_container(data_dir, workspace_roots)
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = workspace_roots;
                Err(anyhow!("container launch mode is only supported on Linux"))
            }
        }
    }
}

fn spawn_daemon_host(data_dir: &Path) -> Result<(String, Child)> {
    let ctx_bin = resolve_ctx_bin()?;
    let mut cmd = Command::new(ctx_bin);
    cmd.arg("serve")
        .arg("--bind")
        .arg("127.0.0.1:0")
        .arg("--data-dir")
        .arg(data_dir.to_string_lossy().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut child = cmd.spawn().context("spawning ctx daemon")?;
    let stdout = child.stdout.take().context("capturing daemon stdout")?;
    let url = wait_for_daemon_listening(stdout)?;
    Ok((url, child))
}

#[cfg(target_os = "linux")]
fn spawn_daemon_container(
    data_dir: &Path,
    workspace_roots: Option<Vec<PathBuf>>,
) -> Result<(String, Child)> {
    ensure_container_runtime()?;
    stop_daemon_container();
    std::fs::create_dir_all(data_dir).context("creating daemon data dir")?;

    let ctx_bin = resolve_ctx_bin()?;
    if !ctx_bin.is_absolute() {
        anyhow::bail!("container launch requires an absolute path to the ctx binary");
    }
    let mounts = container_mounts(data_dir, workspace_roots)?;
    let (uid, gid) = current_user_ids();
    let port = pick_unused_local_port()?;
    let bind = format!("0.0.0.0:{port}");

    let mut cmd = Command::new("docker");
    cmd.arg("run")
        .arg("--rm")
        .arg("--name")
        .arg(DAEMON_CONTAINER_NAME)
        .arg("--publish")
        .arg(format!("127.0.0.1:{port}:{port}"))
        .arg("--user")
        .arg(format!("{uid}:{gid}"))
        .arg("--workdir")
        .arg("/")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    for mount in &mounts {
        cmd.arg("--mount").arg(bind_mount_arg(mount)?);
    }
    cmd.arg("--tmpfs").arg("/tmp");
    cmd.arg("--env").arg("CTX_DAEMON_CONTAINER=1");

    cmd.arg(daemon_container_image())
        .arg(ctx_bin.to_string_lossy().to_string())
        .arg("serve")
        .arg("--bind")
        .arg(bind)
        .arg("--data-dir")
        .arg(data_dir.to_string_lossy().to_string());

    let mut child = cmd.spawn().context("spawning ctx daemon container")?;
    let stdout = child.stdout.take().context("capturing daemon stdout")?;
    let url = wait_for_daemon_listening(stdout)?;
    Ok((url, child))
}

fn wait_for_daemon_listening(stdout: impl std::io::Read) -> Result<String> {
    let mut reader = BufReader::new(stdout).lines();
    let mut url: Option<String> = None;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if let Some(line) = reader.next() {
            let line = line?;
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                if value.get("event").and_then(|event| event.as_str()) == Some("listening") {
                    if let Some(url_value) = value.get("url").and_then(|url| url.as_str()) {
                        url = Some(url_value.to_string());
                        break;
                    }
                }
            }
        } else {
            break;
        }
    }
    url.context("daemon did not emit listening URL")
}

fn read_daemon_auth_with_retry(data_dir: &Path) -> Result<DaemonAuthFile> {
    let path = data_dir.join(DAEMON_AUTH_FILENAME);
    let deadline = Instant::now() + DAEMON_AUTH_READ_TIMEOUT;
    while Instant::now() < deadline {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let auth: DaemonAuthFile = serde_json::from_slice(&bytes).with_context(|| {
                    format!("parsing daemon auth file {}", path.display())
                })?;
                if auth.token.trim().is_empty() {
                    anyhow::bail!(
                        "daemon auth file {} contains empty token",
                        path.display()
                    );
                }
                return Ok(auth);
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {
                std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("reading daemon auth file {}", path.display()));
            }
        }
    }
    Err(anyhow!(
        "timed out waiting for daemon auth file {}",
        path.display()
    ))
}

fn pick_unused_local_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding to ephemeral port")?;
    Ok(listener
        .local_addr()
        .context("reading local address")?
        .port())
}

fn resolve_ctx_bin() -> Result<PathBuf> {
    if let Ok(raw) = env::var("CTX_BIN") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }
    if let Some(path) = find_bin_on_path("ctx") {
        return Ok(path);
    }
    if let Some(path) = dev_bin("ctx") {
        return Ok(path);
    }
    Err(anyhow!("ctx binary not found; ensure it is on PATH"))
}

fn find_bin_on_path(name: &str) -> Option<PathBuf> {
    let path_var = env::var("PATH").ok()?;
    for part in env::split_paths(&path_var) {
        let candidate = part.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
        #[cfg(target_os = "windows")]
        {
            let candidate = part.join(format!("{name}.exe"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

fn dev_bin(name: &str) -> Option<PathBuf> {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    root.pop();
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let candidate = root.join("target").join(profile).join(name);
    if candidate.exists() {
        Some(candidate)
    } else {
        None
    }
}

fn expand_tilde(raw: &str) -> Option<PathBuf> {
    if raw == "~" || raw.starts_with("~/") {
        let base = directories::BaseDirs::new()?;
        if raw == "~" {
            Some(base.home_dir().to_path_buf())
        } else {
            Some(base.home_dir().join(raw.trim_start_matches("~/")))
        }
    } else {
        None
    }
}

fn normalize_workspace_root(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        anyhow::bail!("workspace root is required");
    }
    let mut root = expand_tilde(trimmed).unwrap_or_else(|| PathBuf::from(trimmed));
    if !root.is_absolute() {
        root = std::fs::canonicalize(&root).unwrap_or(root);
    }
    if !root.is_absolute() {
        anyhow::bail!("workspace root must be absolute: {}", trimmed);
    }
    if root == Path::new("/") {
        anyhow::bail!("workspace root cannot be '/'");
    }
    if !root.exists() {
        anyhow::bail!("workspace root does not exist: {}", root.display());
    }
    if !root.is_dir() {
        anyhow::bail!("workspace root is not a directory: {}", root.display());
    }
    Ok(root)
}

#[cfg(target_os = "linux")]
const DAEMON_CONTAINER_NAME: &str = "ctx-daemon";

#[cfg(target_os = "linux")]
const DEFAULT_DAEMON_CONTAINER_IMAGE: &str = "ubuntu:24.04";

#[cfg(target_os = "linux")]
fn daemon_container_image() -> String {
    std::env::var("CTX_DAEMON_CONTAINER_IMAGE")
        .ok()
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
        .unwrap_or_else(|| DEFAULT_DAEMON_CONTAINER_IMAGE.to_string())
}

#[cfg(target_os = "linux")]
fn ensure_container_runtime() -> Result<()> {
    let output = Command::new("docker")
        .arg("info")
        .arg("--format")
        .arg("json")
        .output();
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = if stderr.trim().is_empty() {
                "Docker engine is not available.".to_string()
            } else {
                format!("Docker engine is not available: {}", stderr.trim())
            };
            Err(anyhow!(
                "{detail} Install/start Docker or switch to host mode."
            ))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => Err(anyhow!(
            "Docker runtime not found. Install Docker or switch to host mode."
        )),
        Err(err) => Err(anyhow!("failed to check Docker runtime: {err}")),
    }
}

#[cfg(target_os = "linux")]
fn stop_daemon_container() {
    let _ = Command::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(DAEMON_CONTAINER_NAME)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(target_os = "linux")]
fn current_user_ids() -> (u32, u32) {
    unsafe { (libc::geteuid(), libc::getegid()) }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct BindMount {
    source: PathBuf,
    target: PathBuf,
    readonly: bool,
}

#[cfg(target_os = "linux")]
fn container_mounts(
    data_dir: &Path,
    workspace_roots: Option<Vec<PathBuf>>,
) -> Result<Vec<BindMount>> {
    if !data_dir.is_absolute() {
        anyhow::bail!(
            "container launch requires an absolute data dir: {}",
            data_dir.display()
        );
    }
    if data_dir == Path::new("/") {
        anyhow::bail!("container launch requires a non-root data dir");
    }

    let mut mounts = vec![BindMount {
        source: data_dir.to_path_buf(),
        target: data_dir.to_path_buf(),
        readonly: false,
    }];

    if let Some(roots) = workspace_roots {
        for root in roots {
            if root == Path::new("/") {
                anyhow::bail!("container launch refused to mount '/' as read-write");
            }
            mounts.push(BindMount {
                source: root.clone(),
                target: root,
                readonly: false,
            });
        }
    }

    Ok(mounts)
}

#[cfg(target_os = "linux")]
fn bind_mount_arg(mount: &BindMount) -> Result<String> {
    let src = mount.source.to_string_lossy();
    let dst = mount.target.to_string_lossy();
    if src.contains(',') || dst.contains(',') {
        anyhow::bail!("container mount path contains ',': src={} dst={}", src, dst);
    }
    let mut arg = format!("type=bind,src={src},dst={dst}");
    if mount.readonly {
        arg.push_str(",readonly");
    }
    arg.push_str(",bind-propagation=rshared");
    Ok(arg)
}
