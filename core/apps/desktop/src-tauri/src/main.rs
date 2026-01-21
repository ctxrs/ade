use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, ErrorKind};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri::Emitter;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use url::Url;

fn main() {
    tauri::Builder::default()
        .manage(ConnectionManager::default())
        .manage(DeepLinkTokenStore::default())
        .manage(WorkspaceWindowRegistry::default())
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_app_window(app);
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            desktop_get_connection,
            desktop_disconnect,
            desktop_connect_local,
            desktop_connect_ssh,
            desktop_pick_folder,
            desktop_git_clone,
            desktop_save_text_file,
            desktop_get_editor_settings,
            desktop_update_editor_settings,
            desktop_open_file,
            desktop_open_path,
            desktop_read_file,
            desktop_get_deep_link_token,
            desktop_set_open_workspaces,
            desktop_open_workspace_in_new_window,
            desktop_register_workspace_window,
            desktop_unregister_workspace_window,
            desktop_upload_blob,
            desktop_daemon_request,
        ])
        .setup(|app| {
            open_main_window(&app.handle())?;
            setup_deep_link_listener(&app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let manager = window.state::<ConnectionManager>();
                manager.disconnect();
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let registry = window.state::<WorkspaceWindowRegistry>();
                registry.unregister_window(window.label());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum DesktopConnectionKind {
    None,
    Local,
    Ssh,
}

#[derive(Debug, Clone, Serialize)]
struct DesktopConnectionInfo {
    kind: DesktopConnectionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DaemonAuthFile {
    token: String,
    #[serde(default)]
    daemon_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SshConnectReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default)]
    start_remote: bool,
    #[serde(default)]
    remote_data_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DesktopDaemonRequest {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
struct DesktopHttpResponse {
    status: u16,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DesktopEditorTarget {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "vscode")]
    VsCode,
    #[serde(rename = "vscode_insiders")]
    VsCodeInsiders,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "windsurf")]
    Windsurf,
    #[serde(rename = "antigravity")]
    Antigravity,
    #[serde(rename = "idea")]
    Idea,
    #[serde(rename = "pycharm")]
    Pycharm,
    #[serde(rename = "xcode")]
    Xcode,
    #[serde(rename = "android_studio")]
    AndroidStudio,
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopEditorSettings {
    target: DesktopEditorTarget,
    #[serde(default)]
    custom_command: Option<String>,
    #[serde(default)]
    remote_authority: Option<String>,
}

impl Default for DesktopEditorSettings {
    fn default() -> Self {
        Self {
            target: DesktopEditorTarget::System,
            custom_command: None,
            remote_authority: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DesktopSettings {
    #[serde(default)]
    editor: DesktopEditorSettings,
}

#[derive(Debug, Deserialize)]
struct DesktopOpenFileReq {
    worktree_id: String,
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    col: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct DesktopOpenPathReq {
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    col: Option<u32>,
}

#[derive(Debug, Serialize)]
struct DesktopReadFileResp {
    path: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct DesktopDeepLinkToken {
    token: String,
    expires_at_ms: u64,
}

#[derive(Default)]
struct DeepLinkTokenStore {
    tokens: std::sync::Mutex<HashMap<String, Instant>>,
}

#[derive(Default)]
struct WorkspaceWindowRegistry {
    by_window: std::sync::Mutex<HashMap<String, HashSet<String>>>,
}

const DEEP_LINK_TOKEN_TTL: Duration = Duration::from_secs(600);
const DAEMON_AUTH_FILENAME: &str = "daemon_auth.json";
const DAEMON_AUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);
const DAEMON_AUTH_REMOTE_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_AUTH_RETRY_DELAY: Duration = Duration::from_millis(200);

impl DeepLinkTokenStore {
    fn mint(&self) -> DesktopDeepLinkToken {
        let token = uuid::Uuid::new_v4().to_string();
        let mut tokens = self.tokens.lock().expect("deep link token lock");
        tokens.insert(token.clone(), Instant::now() + DEEP_LINK_TOKEN_TTL);
        let expires_at_ms = SystemTime::now()
            .checked_add(DEEP_LINK_TOKEN_TTL)
            .unwrap_or_else(SystemTime::now)
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        DesktopDeepLinkToken { token, expires_at_ms }
    }

    fn is_valid(&self, token: &str) -> bool {
        let mut tokens = self.tokens.lock().expect("deep link token lock");
        let now = Instant::now();
        tokens.retain(|_, expiry| *expiry > now);
        tokens.get(token).map(|expiry| *expiry > now).unwrap_or(false)
    }
}

impl WorkspaceWindowRegistry {
    fn register(&self, window_label: &str, workspace_id: &str) {
        let mut map = self.by_window.lock().expect("workspace registry lock");
        map.entry(window_label.to_string())
            .or_default()
            .insert(workspace_id.to_string());
    }

    fn unregister_window(&self, window_label: &str) {
        let mut map = self.by_window.lock().expect("workspace registry lock");
        map.remove(window_label);
    }

    fn set_window_workspaces(&self, window_label: &str, workspace_ids: Vec<String>) {
        let mut map = self.by_window.lock().expect("workspace registry lock");
        let mut set = HashSet::new();
        for id in workspace_ids {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                continue;
            }
            set.insert(trimmed.to_string());
        }
        if set.is_empty() {
            map.remove(window_label);
        } else {
            map.insert(window_label.to_string(), set);
        }
    }

    fn window_for_workspace(&self, workspace_id: &str) -> Option<String> {
        let map = self.by_window.lock().expect("workspace registry lock");
        map.iter().find_map(|(label, ids)| {
            if ids.contains(workspace_id) {
                Some(label.clone())
            } else {
                None
            }
        })
    }

    fn workspace_ids(&self) -> Vec<String> {
        let map = self.by_window.lock().expect("workspace registry lock");
        let mut out = HashSet::new();
        for ids in map.values() {
            for id in ids {
                out.insert(id.clone());
            }
        }
        out.into_iter().collect()
    }
}

#[tauri::command]
fn desktop_get_connection(state: tauri::State<ConnectionManager>) -> DesktopConnectionInfo {
    state.info()
}

#[tauri::command]
fn desktop_disconnect(state: tauri::State<ConnectionManager>) -> Result<(), String> {
    state.disconnect();
    Ok(())
}

#[tauri::command]
async fn desktop_pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .and_then(|path| path.into_path().ok())
            .map(|path| path.to_string_lossy().to_string())
    })
    .await
    .map_err(|e| format!("folder picker failed: {e}"))
}

#[tauri::command]
async fn desktop_save_text_file(
    app: tauri::AppHandle,
    suggested_name: Option<String>,
    contents: String,
) -> Result<Option<String>, String> {
    let suggested = suggested_name.unwrap_or_else(|| "conversation.md".to_string());
    let suggested = suggested.trim().to_string();

    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = app
            .dialog()
            .file()
            .add_filter("Markdown", &["md"])
            .set_title("Save Conversation Export");
        if !suggested.is_empty() {
            dialog = dialog.set_file_name(&suggested);
        }
        let picked = dialog
            .blocking_save_file()
            .and_then(|path| path.into_path().ok())
            .map(|path| path.to_string_lossy().to_string());
        let Some(path) = picked else {
            return Ok::<Option<String>, String>(None);
        };

        std::fs::write(&path, contents).map_err(|e| format!("failed to write file: {e}"))?;
        Ok(Some(path))
    })
    .await
    .map_err(|e| format!("save file dialog failed: {e}"))??;
    Ok(picked)
}

#[tauri::command]
fn desktop_get_editor_settings(app: tauri::AppHandle) -> Result<DesktopEditorSettings, String> {
    Ok(load_desktop_settings(&app).editor)
}

#[tauri::command]
fn desktop_update_editor_settings(
    app: tauri::AppHandle,
    settings: DesktopEditorSettings,
) -> Result<DesktopEditorSettings, String> {
    let mut current = load_desktop_settings(&app);
    current.editor = settings;
    save_desktop_settings(&app, &current).map_err(to_err)?;
    Ok(current.editor)
}

#[tauri::command]
fn desktop_open_file(
    state: tauri::State<ConnectionManager>,
    app: tauri::AppHandle,
    req: DesktopOpenFileReq,
) -> Result<(), String> {
    let worktree_id = req.worktree_id.trim();
    if worktree_id.is_empty() {
        return Err("worktree_id is required".to_string());
    }
    let path = req.path.trim();
    if path.is_empty() {
        return Err("path is required".to_string());
    }

    let worktree_root = resolve_worktree_root(&state, worktree_id).map_err(to_err)?;
    let resolved = resolve_worktree_path(&worktree_root, path).map_err(to_err)?;
    let line = req.line.filter(|v| *v > 0);
    let col = req.col.filter(|v| *v > 0);
    let editor_settings = load_desktop_settings(&app).editor;
    open_in_editor(&editor_settings, &resolved, line, col, state.is_remote()).map_err(to_err)?;
    Ok(())
}

#[tauri::command]
fn desktop_open_path(
    app: tauri::AppHandle,
    req: DesktopOpenPathReq,
) -> Result<(), String> {
    let raw = req.path.trim();
    if raw.is_empty() {
        return Err("path is required".to_string());
    }
    let mut path = expand_tilde(raw).unwrap_or_else(|| PathBuf::from(raw));
    if !path.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    path = normalize_path(&path);
    let line = req.line.filter(|v| *v > 0);
    let col = req.col.filter(|v| *v > 0);
    let editor_settings = load_desktop_settings(&app).editor;
    open_in_editor(&editor_settings, &path, line, col, false).map_err(to_err)?;
    Ok(())
}

#[tauri::command]
fn desktop_read_file(
    req: DesktopOpenPathReq,
) -> Result<DesktopReadFileResp, String> {
    let path = req.path.trim();
    if path.is_empty() {
        return Err("path is required".to_string());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err("path must be absolute".to_string());
    }
    let resolved = std::fs::canonicalize(&path).map_err(|e| format!("invalid path: {e}"))?;
    if !resolved.exists() {
        return Err("path does not exist".to_string());
    }
    let text = std::fs::read_to_string(&resolved).map_err(|e| format!("failed to read file: {e}"))?;
    Ok(DesktopReadFileResp {
        path: resolved.to_string_lossy().to_string(),
        text,
    })
}

#[tauri::command]
fn desktop_get_deep_link_token(
    store: tauri::State<DeepLinkTokenStore>,
) -> Result<DesktopDeepLinkToken, String> {
    Ok(store.mint())
}

#[tauri::command]
fn desktop_set_open_workspaces(
    window: tauri::WebviewWindow,
    registry: tauri::State<WorkspaceWindowRegistry>,
    workspace_ids: Vec<String>,
) -> Result<(), String> {
    registry.set_window_workspaces(window.label(), workspace_ids);
    Ok(())
}

#[tauri::command]
fn desktop_open_workspace_in_new_window(
    app: tauri::AppHandle,
    registry: tauri::State<WorkspaceWindowRegistry>,
    workspace_id: String,
) -> Result<(), String> {
    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        return Err("workspace_id is required".to_string());
    }

    let label = format!("workbench:{}", uuid::Uuid::new_v4());
    let url = format!("/workspaces/{workspace_id}");
    let window = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
        .title("ctx")
        .inner_size(1200.0, 900.0)
        .build()
        .map_err(|e| format!("creating window failed: {e}"))?;
    let _ = window.show();
    let _ = window.set_focus();
    registry.register(&label, workspace_id);
    Ok(())
}

#[tauri::command]
fn desktop_register_workspace_window(
    registry: tauri::State<WorkspaceWindowRegistry>,
    workspace_id: String,
    window_label: String,
) -> Result<(), String> {
    let workspace_id = workspace_id.trim();
    if workspace_id.is_empty() {
        return Err("workspace_id is required".to_string());
    }
    let window_label = window_label.trim();
    if window_label.is_empty() {
        return Err("window_label is required".to_string());
    }
    registry.register(window_label, workspace_id);
    Ok(())
}

#[tauri::command]
fn desktop_unregister_workspace_window(
    registry: tauri::State<WorkspaceWindowRegistry>,
    window_label: String,
) -> Result<(), String> {
    let window_label = window_label.trim();
    if window_label.is_empty() {
        return Err("window_label is required".to_string());
    }
    registry.unregister_window(window_label);
    Ok(())
}

#[tauri::command]
fn desktop_git_clone(repo_url: String, dest_parent: String) -> Result<String, String> {
    let repo_url = repo_url.trim().to_string();
    if repo_url.is_empty() {
        return Err("repo_url is required".to_string());
    }
    let dest_parent = PathBuf::from(dest_parent);
    if !dest_parent.exists() {
        return Err(format!(
            "destination folder does not exist: {}",
            dest_parent.display()
        ));
    }

    let name = derive_repo_name(&repo_url).ok_or_else(|| "could not derive repo name".to_string())?;
    let dest = dest_parent.join(&name);
    if dest.exists() {
        return Err(format!("destination already exists: {}", dest.display()));
    }

    let output = Command::new("git")
        .arg("clone")
        .arg("--")
        .arg(&repo_url)
        .arg(&dest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(dest.to_string_lossy().to_string())
}

#[tauri::command]
fn desktop_connect_local(
    app: tauri::AppHandle,
    state: tauri::State<ConnectionManager>,
) -> Result<DesktopConnectionInfo, String> {
    state.disconnect();
    let data_dir = daemon_data_dir(&app).map_err(to_err)?;
    let (url, child, systemd_scope) = spawn_daemon(&app, &data_dir).map_err(to_err)?;
    let auth = read_daemon_auth_with_retry(&data_dir).map_err(to_err)?;
    state.set_local(url.clone(), auth.token.clone(), child, systemd_scope);
    Ok(state.info())
}

#[tauri::command]
async fn desktop_connect_ssh(
    _app: tauri::AppHandle,
    state: tauri::State<'_, ConnectionManager>,
    req: SshConnectReq,
) -> Result<DesktopConnectionInfo, String> {
    state.disconnect();

    let host = req.host.trim().to_string();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    let remote_port = req.remote_port.unwrap_or(4399);

    let user = req.user.clone();
    let remote_data_dir = req.remote_data_dir.clone();
    let start_remote = req.start_remote;
    let (base_url, token, tunnel) =
        tauri::async_runtime::spawn_blocking(move || -> Result<(String, String, Child)> {
        if start_remote {
            start_remote_daemon_over_ssh(
                &host,
                user.as_deref(),
                remote_port,
                remote_data_dir.as_deref(),
            )?;
        }

        let local_port = pick_unused_local_port()?;
        let (mut tunnel, tunnel_stderr) =
            start_ssh_tunnel(&host, user.as_deref(), local_port, remote_port)?;
        let base_url = format!("http://127.0.0.1:{local_port}");

        let health =
            probe_daemon_health_with_retry(&base_url, local_port, &mut tunnel, &tunnel_stderr);
        if let Err(e) = health {
            let _ = try_kill_child(tunnel);
            return Err(e);
        }
        let auth = read_remote_daemon_auth_with_retry(
            &host,
            user.as_deref(),
            remote_data_dir.as_deref(),
        )?;
        Ok((base_url, auth.token, tunnel))
    })
    .await
    .map_err(|e| format!("failed to reach remote daemon: {e}"))?
    .map_err(|e| format!("failed to reach remote daemon: {e:#}"))?;

    state.set_ssh(base_url, Some(token), tunnel);
    Ok(state.info())
}

fn ensure_local_connection(app: &tauri::AppHandle, state: &ConnectionManager) -> Result<()> {
    if !matches!(state.info().kind, DesktopConnectionKind::None) {
        return Ok(());
    }
    let data_dir = daemon_data_dir(app)?;
    let (url, child, systemd_scope) = spawn_daemon(app, &data_dir)?;
    let auth = read_daemon_auth_with_retry(&data_dir)?;
    state.set_local(url, auth.token, child, systemd_scope);
    Ok(())
}

#[tauri::command]
fn desktop_daemon_request(
    state: tauri::State<ConnectionManager>,
    req: DesktopDaemonRequest,
) -> Result<DesktopHttpResponse, String> {
    state.daemon_request(req).map_err(to_err)
}

#[tauri::command]
fn desktop_upload_blob(
    state: tauri::State<ConnectionManager>,
    bytes: Vec<u8>,
    mime_type: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    state.upload_blob(bytes, mime_type, name).map_err(to_err)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeepLinkOpenWith {
    Ctx,
    Editor,
    System,
}

#[derive(Debug)]
enum DeepLinkAction {
    Open(DeepLinkOpen),
    Reveal(DeepLinkReveal),
    Workspace(DeepLinkWorkspace),
    Focus,
}

#[derive(Debug)]
struct DeepLinkOpen {
    target: DeepLinkTarget,
    line: Option<u32>,
    col: Option<u32>,
    open_with: DeepLinkOpenWith,
    editor_override: Option<DesktopEditorTarget>,
    token: Option<String>,
}

#[derive(Debug)]
struct DeepLinkReveal {
    target: DeepLinkTarget,
    token: Option<String>,
}

#[derive(Debug)]
struct DeepLinkWorkspace {
    workspace_id: Option<String>,
    path: Option<String>,
}

#[derive(Debug)]
enum DeepLinkTarget {
    WorktreeFile { worktree_id: String, file: String },
    Path { path: String },
}

#[derive(Debug)]
struct WorktreeInfo {
    root: PathBuf,
    workspace_id: String,
}

fn setup_deep_link_listener(app: &tauri::AppHandle) {
    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        for url in event.urls() {
            handle_deep_link(handle.clone(), url);
        }
    });

    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            handle_deep_link(app.clone(), url);
        }
    }

    if let Err(err) = app.deep_link().register_all() {
        eprintln!("deep link register skipped: {err}");
    }
}

fn handle_deep_link(app: tauri::AppHandle, url: Url) {
    std::thread::spawn(move || {
        if let Err(err) = handle_deep_link_inner(&app, &url) {
            show_error_dialog(&app, &format!("Deep link failed: {err:#}"));
        }
    });
}

fn handle_deep_link_inner(app: &tauri::AppHandle, url: &Url) -> Result<()> {
    let action = parse_deep_link(url)?;
    let state = app.state::<ConnectionManager>();
    let tokens = app.state::<DeepLinkTokenStore>();
    let registry = app.state::<WorkspaceWindowRegistry>();

    match action {
        DeepLinkAction::Open(req) => handle_open(app, &state, &tokens, &registry, req),
        DeepLinkAction::Reveal(req) => handle_reveal(app, &state, &tokens, &registry, req),
        DeepLinkAction::Workspace(req) => handle_workspace(app, &state, &registry, req),
        DeepLinkAction::Focus => {
            focus_app_window(app);
            Ok(())
        }
    }
}

fn parse_deep_link(url: &Url) -> Result<DeepLinkAction> {
    let scheme = url.scheme();
    if scheme != "ctx" {
        anyhow::bail!("unsupported scheme: {scheme}");
    }
    let action = url.host_str().unwrap_or_default();
    let params: HashMap<String, String> =
        url.query_pairs().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let version = params
        .get("v")
        .map(|v| v.parse::<u32>())
        .transpose()
        .context("invalid version")?
        .unwrap_or(1);
    if version != 1 {
        anyhow::bail!("unsupported version: {version}");
    }

    match action {
        "open" => parse_open(&params).map(DeepLinkAction::Open),
        "reveal" => parse_reveal(&params).map(DeepLinkAction::Reveal),
        "workspace" => parse_workspace(&params).map(DeepLinkAction::Workspace),
        "focus" => Ok(DeepLinkAction::Focus),
        _ => anyhow::bail!("unknown action: {action}"),
    }
}

fn parse_open(params: &HashMap<String, String>) -> Result<DeepLinkOpen> {
    let target = parse_target(params)?;
    let line = parse_optional_positive(params.get("line"));
    let col = parse_optional_positive(params.get("col"));
    let open_with = parse_open_with(params.get("openWith"))?;
    let editor_override = match params.get("editor") {
        Some(value) => Some(parse_editor_target(value)?),
        None => None,
    };
    let token = params.get("token").cloned();
    Ok(DeepLinkOpen {
        target,
        line,
        col,
        open_with,
        editor_override,
        token,
    })
}

fn parse_reveal(params: &HashMap<String, String>) -> Result<DeepLinkReveal> {
    let target = parse_target(params)?;
    let token = params.get("token").cloned();
    Ok(DeepLinkReveal { target, token })
}

fn parse_workspace(params: &HashMap<String, String>) -> Result<DeepLinkWorkspace> {
    let workspace_id = params.get("workspaceId").cloned();
    let path = params.get("path").cloned();
    if workspace_id.is_none() && path.is_none() {
        anyhow::bail!("workspaceId or path is required");
    }
    Ok(DeepLinkWorkspace { workspace_id, path })
}

fn parse_target(params: &HashMap<String, String>) -> Result<DeepLinkTarget> {
    let worktree_id = params.get("worktreeId").cloned();
    let file = params.get("file").cloned();
    let path = params.get("path").cloned();

    if let Some(worktree_id) = worktree_id {
        let file = file.ok_or_else(|| anyhow!("file is required when worktreeId is set"))?;
        let file = validate_relative_file(&file)?;
        return Ok(DeepLinkTarget::WorktreeFile { worktree_id, file });
    }

    let path = path.ok_or_else(|| anyhow!("path is required"))?;
    let path = validate_absolute_path(&path)?;
    Ok(DeepLinkTarget::Path { path })
}

fn parse_open_with(value: Option<&String>) -> Result<DeepLinkOpenWith> {
    match value.map(|v| v.trim().to_lowercase()) {
        None => Ok(DeepLinkOpenWith::Ctx),
        Some(v) if v == "ctx" => Ok(DeepLinkOpenWith::Ctx),
        Some(v) if v == "editor" => Ok(DeepLinkOpenWith::Editor),
        Some(v) if v == "system" => Ok(DeepLinkOpenWith::System),
        Some(v) => anyhow::bail!("unsupported openWith: {v}"),
    }
}

fn parse_editor_target(value: &str) -> Result<DesktopEditorTarget> {
    match value.trim().to_lowercase().as_str() {
        "vscode" => Ok(DesktopEditorTarget::VsCode),
        "vscode_insiders" => Ok(DesktopEditorTarget::VsCodeInsiders),
        "cursor" => Ok(DesktopEditorTarget::Cursor),
        "windsurf" => Ok(DesktopEditorTarget::Windsurf),
        "antigravity" => Ok(DesktopEditorTarget::Antigravity),
        "idea" => Ok(DesktopEditorTarget::Idea),
        "pycharm" => Ok(DesktopEditorTarget::Pycharm),
        "xcode" => Ok(DesktopEditorTarget::Xcode),
        "android_studio" => Ok(DesktopEditorTarget::AndroidStudio),
        "custom" => Ok(DesktopEditorTarget::Custom),
        "system" => Ok(DesktopEditorTarget::System),
        other => anyhow::bail!("unknown editor: {other}"),
    }
}

fn parse_optional_positive(value: Option<&String>) -> Option<u32> {
    value
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| *v > 0)
}

fn validate_relative_file(path: &str) -> Result<String> {
    if path.trim().is_empty() {
        anyhow::bail!("file is empty");
    }
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        anyhow::bail!("file must be relative");
    }
    if path.contains(':') {
        anyhow::bail!("file must be a relative path");
    }
    if candidate.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        anyhow::bail!("file must not contain ..");
    }
    Ok(path.to_string())
}

fn validate_absolute_path(path: &str) -> Result<String> {
    if path.trim().is_empty() {
        anyhow::bail!("path is empty");
    }
    let candidate = Path::new(path);
    if !candidate.is_absolute() {
        anyhow::bail!("path must be absolute");
    }
    Ok(path.to_string())
}

fn handle_open(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    tokens: &DeepLinkTokenStore,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkOpen,
) -> Result<()> {
    if matches!(req.target, DeepLinkTarget::WorktreeFile { .. })
        && matches!(state.info().kind, DesktopConnectionKind::None)
    {
        ensure_local_connection(app, state)?;
    }

    if matches!(req.target, DeepLinkTarget::WorktreeFile { .. })
        && state.is_remote()
        && req.open_with != DeepLinkOpenWith::Ctx
    {
        anyhow::bail!("cannot open remote worktree paths in local editors");
    }

    let token_ok = req
        .token
        .as_deref()
        .map(|t| tokens.is_valid(t))
        .unwrap_or(false);
    let in_open_workspace = is_target_in_open_workspace(state, registry, &req.target).unwrap_or(false);
    let needs_prompt = if req.open_with == DeepLinkOpenWith::System {
        true
    } else if token_ok {
        false
    } else {
        !in_open_workspace
    };
    if needs_prompt && !confirm_action(app, "Open this file from an external link?") {
        return Ok(());
    }

    match req.open_with {
        DeepLinkOpenWith::Ctx => open_in_ctx(app, &req.target, req.line, req.col),
        DeepLinkOpenWith::Editor => {
            let settings = load_desktop_settings(app).editor;
            let target = resolve_editor_target(&settings, req.editor_override.as_ref())?;
            let Some(target) = target else {
                offer_editor_settings(app);
                return Ok(());
            };
            open_in_editor_with_target(
                &settings,
                target,
                &resolve_target_path(state, &req.target)?,
                req.line,
                req.col,
                false,
            )
        }
        DeepLinkOpenWith::System => {
            let path = resolve_target_path(state, &req.target)?;
            open_with_system(path.to_string_lossy().as_ref())
        }
    }
}

fn handle_reveal(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    tokens: &DeepLinkTokenStore,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkReveal,
) -> Result<()> {
    if matches!(req.target, DeepLinkTarget::WorktreeFile { .. })
        && matches!(state.info().kind, DesktopConnectionKind::None)
    {
        ensure_local_connection(app, state)?;
    }

    if matches!(req.target, DeepLinkTarget::WorktreeFile { .. }) && state.is_remote() {
        anyhow::bail!("cannot reveal remote worktree paths on this device");
    }

    let token_ok = req
        .token
        .as_deref()
        .map(|t| tokens.is_valid(t))
        .unwrap_or(false);
    let in_open_workspace = is_target_in_open_workspace(state, registry, &req.target).unwrap_or(false);
    let needs_prompt = !token_ok && !in_open_workspace;
    if needs_prompt && !confirm_action(app, "Reveal this path from an external link?") {
        return Ok(());
    }

    let path = resolve_target_path(state, &req.target)?;
    reveal_in_file_manager(&path)
}

fn handle_workspace(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkWorkspace,
) -> Result<()> {
    if matches!(state.info().kind, DesktopConnectionKind::None) {
        ensure_local_connection(app, state)?;
    }

    let workspace_id = if let Some(id) = req.workspace_id {
        id
    } else if let Some(path) = req.path {
        resolve_or_create_workspace_id(state, &path)?
    } else {
        anyhow::bail!("workspaceId or path is required");
    };
    open_workspace_window(app, registry, &workspace_id)
}

fn open_in_ctx(app: &tauri::AppHandle, target: &DeepLinkTarget, line: Option<u32>, col: Option<u32>) -> Result<()> {
    let url = build_file_preview_url(target, line, col);
    let label = format!("file:{}", uuid::Uuid::new_v4());
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("ctx")
        .inner_size(1000.0, 780.0)
        .build()
        .context("creating file preview window")?;
    Ok(())
}

fn build_file_preview_url(target: &DeepLinkTarget, line: Option<u32>, col: Option<u32>) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    match target {
        DeepLinkTarget::WorktreeFile { worktree_id, file } => {
            serializer.append_pair("worktreeId", worktree_id);
            serializer.append_pair("file", file);
        }
        DeepLinkTarget::Path { path } => {
            serializer.append_pair("path", path);
        }
    }
    if let Some(line) = line {
        serializer.append_pair("line", &line.to_string());
    }
    if let Some(col) = col {
        serializer.append_pair("col", &col.to_string());
    }
    format!("/file?{}", serializer.finish())
}

fn resolve_editor_target(
    settings: &DesktopEditorSettings,
    override_target: Option<&DesktopEditorTarget>,
) -> Result<Option<DesktopEditorTarget>> {
    let target = override_target.cloned().unwrap_or_else(|| settings.target.clone());
    let has_custom = settings
        .custom_command
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if matches!(target, DesktopEditorTarget::System) {
        return Ok(None);
    }
    if matches!(target, DesktopEditorTarget::Custom) && !has_custom {
        return Ok(None);
    }
    Ok(Some(target))
}

fn offer_editor_settings(app: &tauri::AppHandle) {
    let should_open = app
        .dialog()
        .message("No editor is configured. Open Settings to pick one?")
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Open Settings".into(),
            "Cancel".into(),
        ))
        .blocking_show();
    if should_open {
        let _ = open_settings_window(app);
    }
}

fn open_settings_window(app: &tauri::AppHandle) -> Result<()> {
    let label = "settings";
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("/settings".into()))
        .title("ctx settings")
        .inner_size(1000.0, 780.0)
        .build()
        .context("creating settings window")?;
    Ok(())
}

fn open_in_editor_with_target(
    settings: &DesktopEditorSettings,
    target: DesktopEditorTarget,
    path: &Path,
    line: Option<u32>,
    col: Option<u32>,
    remote: bool,
) -> Result<()> {
    let adjusted = DesktopEditorSettings {
        target,
        custom_command: settings.custom_command.clone(),
        remote_authority: settings.remote_authority.clone(),
    };
    open_in_editor(&adjusted, path, line, col, remote)
}

fn resolve_target_path(state: &ConnectionManager, target: &DeepLinkTarget) -> Result<PathBuf> {
    match target {
        DeepLinkTarget::WorktreeFile { worktree_id, file } => {
            let info = resolve_worktree_info(state, worktree_id)?;
            resolve_worktree_path(&info.root, file)
        }
        DeepLinkTarget::Path { path } => {
            let path = PathBuf::from(path);
            let resolved = std::fs::canonicalize(&path)
                .with_context(|| format!("resolving {}", path.display()))?;
            if !resolved.exists() {
                anyhow::bail!("path does not exist");
            }
            Ok(resolved)
        }
    }
}

fn is_target_in_open_workspace(
    state: &ConnectionManager,
    registry: &WorkspaceWindowRegistry,
    target: &DeepLinkTarget,
) -> Result<bool> {
    let workspace_ids = registry.workspace_ids();
    if workspace_ids.is_empty() {
        return Ok(false);
    }

    match target {
        DeepLinkTarget::WorktreeFile { worktree_id, .. } => {
            let info = resolve_worktree_info(state, worktree_id)?;
            Ok(workspace_ids.iter().any(|id| id == &info.workspace_id))
        }
        DeepLinkTarget::Path { path } => {
            let candidate = std::fs::canonicalize(PathBuf::from(path))?;
            for ws_id in workspace_ids {
                if let Ok(root) = resolve_workspace_root(state, &ws_id) {
                    if candidate.starts_with(&root) {
                        return Ok(true);
                    }
                }
            }
            Ok(false)
        }
    }
}

fn resolve_or_create_workspace_id(state: &ConnectionManager, root_path: &str) -> Result<String> {
    if let Some(existing) = resolve_workspace_id_by_path(state, root_path)? {
        return Ok(existing);
    }

    let body = serde_json::json!({ "root_path": root_path });
    let resp = state.daemon_request(DesktopDaemonRequest {
        method: "POST".to_string(),
        path: "/api/workspaces".to_string(),
        body: Some(body.to_string()),
        headers: vec![],
    })?;
    if resp.status != 200 && resp.status != 201 {
        anyhow::bail!("failed to create workspace ({status}): {body}", status = resp.status, body = resp.body);
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing workspace response")?;
    parse_id_value(&value["id"]).ok_or_else(|| anyhow!("workspace id missing"))
}

fn resolve_workspace_id_by_path(state: &ConnectionManager, root_path: &str) -> Result<Option<String>> {
    let resp = state.daemon_request(DesktopDaemonRequest {
        method: "GET".to_string(),
        path: "/api/workspaces".to_string(),
        body: None,
        headers: vec![],
    })?;
    if resp.status != 200 {
        anyhow::bail!("failed to list workspaces ({status}): {body}", status = resp.status, body = resp.body);
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing workspaces response")?;
    let arr = value.as_array().ok_or_else(|| anyhow!("workspaces response is not a list"))?;
    for entry in arr {
        let ws_root = entry.get("root_path").and_then(|v| v.as_str()).unwrap_or_default();
        if ws_root == root_path {
            if let Some(id) = entry.get("id").and_then(parse_id_value) {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}

fn resolve_workspace_root(state: &ConnectionManager, workspace_id: &str) -> Result<PathBuf> {
    let resp = state.daemon_request(DesktopDaemonRequest {
        method: "GET".to_string(),
        path: format!("/api/workspaces/{workspace_id}"),
        body: None,
        headers: vec![],
    })?;
    if resp.status != 200 {
        anyhow::bail!("failed to load workspace ({status}): {body}", status = resp.status, body = resp.body);
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing workspace response")?;
    let root = value
        .get("root_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("workspace root_path missing"))?;
    Ok(PathBuf::from(root))
}

fn resolve_worktree_info(state: &ConnectionManager, worktree_id: &str) -> Result<WorktreeInfo> {
    let resp = state.daemon_request(DesktopDaemonRequest {
        method: "GET".to_string(),
        path: format!("/api/worktrees/{worktree_id}"),
        body: None,
        headers: vec![],
    })?;
    if resp.status != 200 {
        anyhow::bail!("failed to load worktree ({status}): {body}", status = resp.status, body = resp.body);
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing worktree response")?;
    let root = value
        .get("root_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("worktree root_path missing"))?;
    let workspace_id = value
        .get("workspace_id")
        .and_then(parse_id_value)
        .ok_or_else(|| anyhow!("worktree workspace_id missing"))?;
    Ok(WorktreeInfo {
        root: PathBuf::from(root),
        workspace_id,
    })
}

fn parse_id_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| value.get("0").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .or_else(|| value.get(0).and_then(|v| v.as_str()).map(|s| s.to_string()))
}

fn open_workspace_window(
    app: &tauri::AppHandle,
    registry: &WorkspaceWindowRegistry,
    workspace_id: &str,
) -> Result<()> {
    if let Some(window_label) = registry.window_for_workspace(workspace_id) {
        if let Some(window) = app.get_webview_window(&window_label) {
            let _ = window.show();
            let _ = window.set_focus();
            let url = format!("/workspaces/{workspace_id}");
            let js = format!(
                "window.location.href = {};",
                serde_json::to_string(&url).unwrap_or_else(|_| "\"/workspaces\"".to_string())
            );
            let _ = window.eval(&js);
            let _ = window.emit("workspace:open", workspace_id.to_string());
            registry.register(&window_label, workspace_id);
            return Ok(());
        }
        registry.unregister_window(&window_label);
    }

    open_main_window(app)?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| anyhow!("main window not available"))?;
    let _ = window.show();
    let _ = window.set_focus();

    let url = format!("/workspaces/{workspace_id}");
    let js = format!(
        "window.location.href = {};",
        serde_json::to_string(&url).unwrap_or_else(|_| "\"/workspaces\"".to_string())
    );
    let _ = window.eval(&js);
    let _ = window.emit("workspace:open", workspace_id.to_string());
    registry.register("main", workspace_id);
    Ok(())
}

fn focus_app_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    if let Some(window) = app.webview_windows().values().next() {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn confirm_action(app: &tauri::AppHandle, message: &str) -> bool {
    app.dialog()
        .message(message)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Open".into(), "Cancel".into()))
        .blocking_show()
}

fn show_error_dialog(app: &tauri::AppHandle, message: &str) {
    app.dialog()
        .message(message)
        .kind(MessageDialogKind::Error)
        .buttons(MessageDialogButtons::Ok)
        .show(|_| {});
}

fn reveal_in_file_manager(path: &Path) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let status = Command::new("open").arg("-R").arg(path).status()?;
        if !status.success() {
            anyhow::bail!("failed to reveal file (exit={status})");
        }
        return Ok(());
    }

    #[cfg(target_os = "windows")]
    {
        let target = path.to_string_lossy();
        let status = Command::new("explorer")
            .arg("/select,")
            .arg(target.as_ref())
            .status()?;
        if !status.success() {
            anyhow::bail!("failed to reveal file (exit={status})");
        }
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        let dir = path.parent().unwrap_or(path);
        let status = Command::new("xdg-open").arg(dir).status()?;
        if !status.success() {
            anyhow::bail!("failed to reveal file (exit={status})");
        }
        return Ok(());
    }
}

fn open_main_window(app: &tauri::AppHandle) -> Result<()> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
        .title("ctx")
        .inner_size(1200.0, 900.0)
        .build()
        .context("creating window")?;
    Ok(())
}

#[derive(Default)]
struct ConnectionManager(std::sync::Mutex<ConnectionState>);

#[derive(Default)]
struct ConnectionState {
    active: Option<ActiveConnection>,
}

enum ActiveConnection {
    Local(LocalConnection),
    Ssh(SshConnection),
}

struct LocalConnection {
    base_url: String,
    token: String,
    child: Child,
    systemd_scope: bool,
}

struct SshConnection {
    base_url: String,
    token: Option<String>,
    tunnel: Child,
}

impl ConnectionManager {
    fn info(&self) -> DesktopConnectionInfo {
        let guard = self.0.lock().ok();
        let Some(guard) = guard.as_ref() else {
            return DesktopConnectionInfo { kind: DesktopConnectionKind::None, base_url: None, token: None };
        };
        match &guard.active {
            None => DesktopConnectionInfo { kind: DesktopConnectionKind::None, base_url: None, token: None },
            Some(ActiveConnection::Local(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                token: Some(c.token.clone()),
            },
            Some(ActiveConnection::Ssh(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Ssh,
                base_url: Some(c.base_url.clone()),
                token: c.token.clone(),
            },
        }
    }

    fn is_remote(&self) -> bool {
        let guard = self.0.lock().ok();
        matches!(guard.as_ref().and_then(|g| g.active.as_ref()), Some(ActiveConnection::Ssh(_)))
    }

    fn disconnect(&self) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Some(active) = guard.active.take() {
            match active {
                ActiveConnection::Local(c) => {
                    if c.systemd_scope {
                        stop_systemd_scope();
                    }
                    let _ = try_kill_child(c.child);
                }
                ActiveConnection::Ssh(c) => {
                    let _ = try_kill_child(c.tunnel);
                }
            }
        }
    }

    fn set_local(&self, base_url: String, token: String, child: Child, systemd_scope: bool) {
        let mut guard = self.0.lock().expect("connection manager lock");
        guard.active = Some(ActiveConnection::Local(LocalConnection {
            base_url,
            token,
            child,
            systemd_scope,
        }));
    }

    fn set_ssh(&self, base_url: String, token: Option<String>, tunnel: Child) {
        let mut guard = self.0.lock().expect("connection manager lock");
        guard.active = Some(ActiveConnection::Ssh(SshConnection { base_url, token, tunnel }));
    }

    fn daemon_request(&self, req: DesktopDaemonRequest) -> Result<DesktopHttpResponse> {
        if !req.path.starts_with("/api/") {
            return Err(anyhow!("only /api/* paths are supported"));
        }

        let (base_url, token) = {
            let guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::Ssh(c) => (c.base_url.clone(), c.token.clone()),
            }
        };

        let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;

        let method = req.method.trim().to_uppercase();
        let mut builder = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "DELETE" => client.delete(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            other => return Err(anyhow!("unsupported method: {other}")),
        };

        if let Some(t) = token.as_deref() {
            if !t.trim().is_empty() {
                builder = builder.bearer_auth(t);
            }
        }
        for (k, v) in req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        let res = builder.send().context("sending request")?;
        let status = res.status().as_u16();
        let content_type = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = res.text().unwrap_or_default();
        Ok(DesktopHttpResponse { status, body, content_type })
    }

    fn upload_blob(
        &self,
        bytes: Vec<u8>,
        mime_type: String,
        name: Option<String>,
    ) -> Result<serde_json::Value> {
        let (base_url, token) = {
            let guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::Ssh(c) => (c.base_url.clone(), c.token.clone()),
            }
        };

        let url = format!("{}/api/blobs", base_url.trim_end_matches('/'));
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("building http client")?;

        let mut part = reqwest::blocking::multipart::Part::bytes(bytes);
        if let Some(n) = name.as_deref().filter(|s| !s.trim().is_empty()) {
            part = part.file_name(n.to_string());
        }
        part = part
            .mime_str(&mime_type)
            .context("invalid mime_type for multipart")?;

        let form = reqwest::blocking::multipart::Form::new().part("file", part);
        let mut req = client.post(url).multipart(form);
        if let Some(t) = token.as_deref() {
            if !t.trim().is_empty() {
                req = req.bearer_auth(t);
            }
        }
        let res = req.send().context("uploading blob")?;
        let status = res.status();
        let body = res.text().unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("blob upload failed ({status}): {body}"));
        }
        Ok(serde_json::from_str(&body).context("parsing blob upload response")?)
    }
}

fn to_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn desktop_settings_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = app
        .path()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("desktop-settings.json"))
}

fn load_desktop_settings(app: &tauri::AppHandle) -> DesktopSettings {
    let path = match desktop_settings_path(app) {
        Ok(path) => path,
        Err(_) => return DesktopSettings::default(),
    };
    let data = match std::fs::read_to_string(&path) {
        Ok(data) => data,
        Err(_) => return DesktopSettings::default(),
    };
    serde_json::from_str::<DesktopSettings>(&data).unwrap_or_default()
}

fn save_desktop_settings(app: &tauri::AppHandle, settings: &DesktopSettings) -> Result<()> {
    let path = desktop_settings_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(settings)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn resolve_worktree_root(state: &ConnectionManager, worktree_id: &str) -> Result<PathBuf> {
    let resp = state.daemon_request(DesktopDaemonRequest {
        method: "GET".to_string(),
        path: format!("/api/worktrees/{worktree_id}"),
        body: None,
        headers: vec![],
    })?;
    if resp.status != 200 {
        return Err(anyhow!(
            "failed to load worktree ({status}): {body}",
            status = resp.status,
            body = resp.body
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing worktree response")?;
    let root = value
        .get("root_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("worktree root_path missing"))?;
    Ok(PathBuf::from(root))
}


fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::Prefix(p) => out.push(p.as_os_str()),
            std::path::Component::RootDir => out.push(std::path::MAIN_SEPARATOR_STR),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::Normal(x) => out.push(x),
        }
    }
    out
}

fn expand_tilde(raw: &str) -> Option<PathBuf> {
    if raw == "~" || raw.starts_with("~/") {
        let base = directories::BaseDirs::new()?;
        let home = base.home_dir();
        if raw == "~" {
            Some(home.to_path_buf())
        } else {
            Some(home.join(raw.trim_start_matches("~/")))
        }
    } else {
        None
    }
}

fn resolve_worktree_path(worktree_root: &Path, raw_path: &str) -> Result<PathBuf> {
    let root = std::fs::canonicalize(worktree_root)
        .with_context(|| format!("canonicalizing {}", worktree_root.display()))?;
    let mut candidate = expand_tilde(raw_path).unwrap_or_else(|| PathBuf::from(raw_path));
    if !candidate.is_absolute() {
        candidate = root.join(candidate);
    }
    let candidate = normalize_path(&candidate);
    if !candidate.starts_with(&root) {
        return Err(anyhow!("path is outside the worktree root"));
    }
    Ok(candidate)
}

fn open_in_editor(
    settings: &DesktopEditorSettings,
    path: &Path,
    line: Option<u32>,
    col: Option<u32>,
    remote: bool,
) -> Result<()> {
    let remote_authority = settings
        .remote_authority
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    if remote {
        match settings.target {
            DesktopEditorTarget::VsCode => {
                let authority = remote_authority.ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("vscode", authority, path, line, col))
            }
            DesktopEditorTarget::VsCodeInsiders => {
                let authority = remote_authority.ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("vscode-insiders", authority, path, line, col))
            }
            DesktopEditorTarget::Cursor => {
                let authority = remote_authority.ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("cursor", authority, path, line, col))
            }
            DesktopEditorTarget::Windsurf => {
                let authority = remote_authority.ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("windsurf", authority, path, line, col))
            }
            DesktopEditorTarget::Antigravity => {
                let authority = remote_authority.ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("antigravity", authority, path, line, col))
            }
            DesktopEditorTarget::Custom => {
                let cmd = settings
                    .custom_command
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("custom command is not configured"))?;
                open_custom_command(cmd, path, line, col)
            }
            _ => anyhow::bail!("remote editor target does not support remote paths"),
        }
    } else {
        match settings.target {
            DesktopEditorTarget::System => open_with_system(path.to_string_lossy().as_ref()),
            DesktopEditorTarget::VsCode => open_with_system(&vscode_uri("vscode", path, line, col)),
            DesktopEditorTarget::VsCodeInsiders => {
                open_with_system(&vscode_uri("vscode-insiders", path, line, col))
            }
            DesktopEditorTarget::Cursor => open_with_system(&vscode_uri("cursor", path, line, col)),
            DesktopEditorTarget::Windsurf => open_with_system(&vscode_uri("windsurf", path, line, col)),
            DesktopEditorTarget::Antigravity => {
                open_with_system(&vscode_uri("antigravity", path, line, col))
            }
            DesktopEditorTarget::Idea => open_with_system(&jetbrains_uri("idea", path, line)),
            DesktopEditorTarget::Pycharm => open_with_system(&jetbrains_uri("pycharm", path, line)),
            DesktopEditorTarget::Xcode => open_xcode(path, line),
            DesktopEditorTarget::AndroidStudio => open_android_studio(path, line),
            DesktopEditorTarget::Custom => {
                let cmd = settings
                    .custom_command
                    .as_ref()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| anyhow!("custom command is not configured"))?;
                open_custom_command(cmd, path, line, col)
            }
        }
    }
}

fn open_with_system(target: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut cmd = Command::new("explorer");

    cmd.arg(target);
    let status = cmd.status().context("spawning open command")?;
    if !status.success() {
        anyhow::bail!("open command failed (exit={status})");
    }
    Ok(())
}

fn open_xcode(path: &Path, line: Option<u32>) -> Result<()> {
    if !cfg!(target_os = "macos") {
        anyhow::bail!("Xcode is only available on macOS");
    }
    let mut cmd = Command::new("xcrun");
    cmd.arg("xed");
    if let Some(line) = line {
        cmd.arg("-l").arg(line.to_string());
    }
    cmd.arg(path);
    let status = cmd.status().context("launching xed")?;
    if !status.success() {
        anyhow::bail!("xed failed (exit={status})");
    }
    Ok(())
}

fn open_android_studio(path: &Path, _line: Option<u32>) -> Result<()> {
    if cfg!(target_os = "macos") {
        let status = Command::new("open")
            .arg("-a")
            .arg("Android Studio")
            .arg(path)
            .status()
            .context("opening Android Studio")?;
        if !status.success() {
            anyhow::bail!("failed to open Android Studio (exit={status})");
        }
        return Ok(());
    }

    let status = Command::new("studio")
        .arg(path)
        .status()
        .context("launching Android Studio")?;
    if !status.success() {
        anyhow::bail!("studio command failed (exit={status})");
    }
    Ok(())
}

fn open_custom_command(
    command: &str,
    path: &Path,
    line: Option<u32>,
    col: Option<u32>,
) -> Result<()> {
    let path_str = path.to_string_lossy();
    let line_str = line.map(|v| v.to_string()).unwrap_or_default();
    let col_str = col.map(|v| v.to_string()).unwrap_or_default();
    let rendered = command
        .replace("{path}", &path_str)
        .replace("{line}", &line_str)
        .replace("{col}", &col_str);
    let parts = shell_words::split(&rendered).context("parsing custom command")?;
    if parts.is_empty() {
        anyhow::bail!("custom command is empty");
    }
    let mut cmd = Command::new(&parts[0]);
    if parts.len() > 1 {
        cmd.args(&parts[1..]);
    }
    let status = cmd.status().context("launching custom command")?;
    if !status.success() {
        anyhow::bail!("custom command failed (exit={status})");
    }
    Ok(())
}


fn vscode_remote_uri(
    scheme: &str,
    authority: &str,
    path: &Path,
    line: Option<u32>,
    col: Option<u32>,
) -> String {
    let mut uri = format!("{scheme}://vscode-remote/{authority}{}", encode_uri_path(path));
    if let Some(line) = line {
        uri.push(':');
        uri.push_str(&line.to_string());
        if let Some(col) = col {
            uri.push(':');
            uri.push_str(&col.to_string());
        }
    }
    uri
}

fn vscode_uri(scheme: &str, path: &Path, line: Option<u32>, col: Option<u32>) -> String {
    let mut uri = format!("{scheme}://file/{}", encode_uri_path(path));
    if let Some(line) = line {
        uri.push(':');
        uri.push_str(&line.to_string());
        if let Some(col) = col {
            uri.push(':');
            uri.push_str(&col.to_string());
        }
    }
    uri
}

fn jetbrains_uri(scheme: &str, path: &Path, line: Option<u32>) -> String {
    let raw = path.to_string_lossy();
    let encoded = urlencoding::encode(&raw);
    let mut uri = format!("{scheme}://open?file={encoded}");
    if let Some(line) = line {
        uri.push_str(&format!("&line={line}"));
    }
    uri
}

fn encode_uri_path(path: &Path) -> String {
    let mut raw = path.to_string_lossy().replace('\\', "/");
    if cfg!(target_os = "windows") {
        if raw.len() >= 2 && raw.as_bytes().get(1) == Some(&b':') {
            raw = format!("/{raw}");
        }
    }
    percent_encode_path(&raw)
}

fn percent_encode_path(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        let ch = b as char;
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~' | '/' | ':');
        if keep {
            out.push(ch);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn derive_repo_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit('/').next()?;
    let name = last.trim_end_matches(".git").trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn pick_unused_local_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding ephemeral port")?;
    let port = listener.local_addr().context("reading local addr")?.port();
    Ok(port)
}

const SSH_TUNNEL_LOG_BYTES: usize = 4096;
const SSH_TUNNEL_HEALTH_RETRIES: usize = 12;
const SSH_TUNNEL_HEALTH_BASE_DELAY_MS: u64 = 150;

fn start_ssh_tunnel(
    host: &str,
    user: Option<&str>,
    local_port: u16,
    remote_port: u16,
) -> Result<(Child, std::sync::Arc<std::sync::Mutex<String>>)> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let mut cmd = Command::new("ssh");
    cmd.arg("-N")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!("{local_port}:127.0.0.1:{remote_port}"))
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().context("spawning ssh tunnel")?;
    let stderr_log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let stderr_log = std::sync::Arc::clone(&stderr_log);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buf = String::new();
            loop {
                buf.clear();
                let read = match reader.read_line(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(_) => break,
                };
                if read == 0 {
                    break;
                }
                let mut log = match stderr_log.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };
                if log.len() + buf.len() > SSH_TUNNEL_LOG_BYTES {
                    let excess = (log.len() + buf.len()) - SSH_TUNNEL_LOG_BYTES;
                    log.drain(..excess);
                }
                log.push_str(&buf);
            }
        });
    }
    Ok((child, stderr_log))
}

fn start_remote_daemon_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    remote_data_dir: Option<&str>,
) -> Result<()> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let data_dir = remote_data_dir.unwrap_or("~/.ctx");
    let exec_cmd = format!(
        "if command -v ctx >/dev/null 2>&1; then ctx serve --bind 127.0.0.1:{remote_port} --data-dir {dir}; else echo 'ctx not found on PATH' >&2; exit 127; fi",
        dir = remote_path_expr(data_dir),
    );
    let log_cmd = format!("{exec_cmd} > ~/.ctx/logs/daemon.log 2>&1");
    let systemd_cmd = format!(
        "systemd-run --user --scope --unit ctx-daemon --no-block /bin/sh -lc {}",
        shell_escape(&log_cmd)
    );
    let nohup_cmd = format!("nohup {log_cmd} &");
    let remote_cmd = format!(
        "if command -v systemd-run >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then {}; else {}; fi",
        systemd_cmd, nohup_cmd
    );

    let output = Command::new("ssh")
        .arg(target)
        .arg("sh")
        .arg("-lc")
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("starting remote daemon over ssh")?;
    if !output.status.success() {
        return Err(anyhow!(
            "ssh start failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn shell_escape(s: &str) -> String {
    // Minimal POSIX shell escaping: wrap in single quotes and escape inner single quotes.
    let inner = s.replace('\'', "'\"'\"'");
    format!("'{}'", inner)
}

fn escape_for_double_quotes(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

fn remote_path_expr(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "~" {
        return "\"$HOME\"".to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("~/") {
        let escaped = escape_for_double_quotes(rest);
        if escaped.is_empty() {
            return "\"$HOME\"".to_string();
        }
        return format!("\"$HOME/{}\"", escaped);
    }
    shell_escape(trimmed)
}

fn parse_daemon_auth(bytes: &[u8], path: &Path) -> Result<DaemonAuthFile> {
    let auth: DaemonAuthFile =
        serde_json::from_slice(bytes).with_context(|| format!("parsing {}", path.display()))?;
    if auth.token.trim().is_empty() {
        anyhow::bail!("daemon auth file {} contains empty token", path.display());
    }
    Ok(auth)
}

fn read_daemon_auth_with_retry(data_dir: &Path) -> Result<DaemonAuthFile> {
    let path = data_dir.join(DAEMON_AUTH_FILENAME);
    let deadline = Instant::now() + DAEMON_AUTH_READ_TIMEOUT;
    let mut last_err: Option<anyhow::Error> = None;
    loop {
        match std::fs::read(&path) {
            Ok(bytes) => return parse_daemon_auth(&bytes, &path),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                last_err = Some(anyhow!("daemon auth file not found at {}", path.display()));
            }
            Err(err) => {
                last_err = Some(
                    err.context(format!("reading daemon auth file {}", path.display())),
                );
            }
        }
        if Instant::now() > deadline {
            return Err(last_err.unwrap_or_else(|| {
                anyhow!("daemon auth file not found at {}", path.display())
            }));
        }
        std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
    }
}

fn read_remote_daemon_auth(
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<DaemonAuthFile> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let auth_path = format!("{}/{}", data_dir.trim_end_matches('/'), DAEMON_AUTH_FILENAME);
    let cmd = format!("cat -- {}", remote_path_expr(&auth_path));

    let output = Command::new("ssh")
        .arg(target)
        .arg("sh")
        .arg("-lc")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("reading daemon auth file over ssh")?;

    if !output.status.success() {
        return Err(anyhow!(
            "ssh read failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    parse_daemon_auth(&output.stdout, Path::new(&auth_path))
}

fn read_remote_daemon_auth_with_retry(
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<DaemonAuthFile> {
    let deadline = Instant::now() + DAEMON_AUTH_REMOTE_TIMEOUT;
    let mut last_err: Option<anyhow::Error> = None;
    loop {
        match read_remote_daemon_auth(host, user, remote_data_dir) {
            Ok(auth) => return Ok(auth),
            Err(err) => last_err = Some(err),
        }
        if Instant::now() > deadline {
            return Err(last_err.unwrap_or_else(|| {
                anyhow!("timed out reading daemon auth file over ssh")
            }));
        }
        std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
    }
}

fn probe_daemon_health(base_url: &str) -> Result<()> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building http client")?;
    let res = client.get(url).send().context("requesting /api/health")?;
    res.error_for_status().context("health status")?;
    Ok(())
}

fn probe_daemon_health_with_retry(
    base_url: &str,
    local_port: u16,
    tunnel: &mut Child,
    stderr_log: &std::sync::Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..SSH_TUNNEL_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if let Ok(Some(status)) = tunnel.try_wait() {
                    let stderr = ssh_log_snippet(stderr_log);
                    if stderr.is_empty() {
                        return Err(anyhow!("ssh tunnel exited ({status})"));
                    }
                    return Err(anyhow!("ssh tunnel exited ({status}): {stderr}"));
                }
            }
        }
        let delay = SSH_TUNNEL_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
        std::thread::sleep(Duration::from_millis(delay));
    }
    let err = last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed"));
    let stderr = ssh_log_snippet(stderr_log);
    let tunnel_state = match tunnel.try_wait() {
        Ok(Some(status)) => format!("ssh tunnel exited ({status})"),
        Ok(None) => "ssh tunnel still running".to_string(),
        Err(e) => format!("ssh tunnel state unknown ({e})"),
    };
    let mut details = format!("{tunnel_state}; local port {local_port}");
    if !stderr.is_empty() {
        details.push_str(&format!("; ssh stderr: {stderr}"));
    }
    Err(anyhow!("{err:#}; {details}"))
}

fn ssh_log_snippet(stderr_log: &std::sync::Arc<std::sync::Mutex<String>>) -> String {
    let log = stderr_log.lock().ok();
    let Some(log) = log.as_ref() else {
        return String::new();
    };
    let snippet = log.trim();
    if snippet.is_empty() {
        String::new()
    } else {
        snippet.to_string()
    }
}

fn daemon_data_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = app
        .path()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("daemon"))
}

fn current_arch_token() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        std::env::consts::ARCH
    }
}

fn resource_bin(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(res) = app.path().resource_dir().ok() {
        let bin_ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
        candidates.push(res.join("bin").join(format!("{name}{bin_ext}")));
        candidates.push(res.join(format!("{name}{bin_ext}")));

        let arch = current_arch_token();
        let prefix = format!("{name}-{arch}");
        for base in [res.join("bin"), res.clone()] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for p in paths {
                if !p.is_file() {
                    continue;
                }
                let Some(file_name) = p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !file_name.starts_with(&prefix) {
                    continue;
                }
                if !bin_ext.is_empty() && !file_name.ends_with(bin_ext) {
                    continue;
                }
                candidates.push(p);
            }
        }
    }

    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    None
}

fn dev_bin(name: &str) -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())?
        .to_path_buf(); // core/
    let candidate = root.join("target").join("debug").join(name);
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

fn dev_web_dist() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())?
        .join("web")
        .join("dist"); // core/apps/web/dist
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn systemd_run_available() -> bool {
    match Command::new("systemd-run").arg("--version").status() {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn systemd_user_available() -> bool {
    match Command::new("systemctl")
        .arg("--user")
        .arg("show-environment")
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => false,
    }
}

#[cfg(target_os = "linux")]
fn should_use_systemd_scope() -> bool {
    systemd_run_available() && systemd_user_available()
}

#[cfg(not(target_os = "linux"))]
fn should_use_systemd_scope() -> bool {
    false
}

fn stop_systemd_scope() {
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("systemctl")
            .arg("--user")
            .arg("stop")
            .arg("ctx-daemon.scope")
            .status();
    }
}

fn spawn_daemon(app: &tauri::AppHandle, data_dir: &Path) -> Result<(String, Child, bool)> {
    let ctx_bin = resource_bin(app, "ctx")
        .or_else(|| dev_bin("ctx"))
        .unwrap_or_else(|| PathBuf::from("ctx"));

    let mcp_bin = resource_bin(app, "ctx-mcp")
        .or_else(|| dev_bin("ctx-mcp"));

    let web_dist = app
        .path()
        .resource_dir()
        .ok()
        .and_then(|p| {
            let candidates = [p.join("web").join("dist"), p.join("web-dist"), p.join("dist")];
            candidates.into_iter().find(|c| c.exists())
        })
        .or_else(dev_web_dist);

    let use_systemd_scope = should_use_systemd_scope();
    if use_systemd_scope {
        stop_systemd_scope();
    }
    let mut cmd = if use_systemd_scope {
        let mut cmd = Command::new("systemd-run");
        cmd.arg("--user")
            .arg("--scope")
            .arg("--unit")
            .arg("ctx-daemon")
            .arg("--same-dir");
        if let Some(dist) = web_dist.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_WEB_DIST={}", dist.to_string_lossy()));
        }
        if let Some(mcp) = mcp_bin.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_MCP_COMMAND={}", mcp.to_string_lossy()));
        }
        if let Ok(appimage) = std::env::var("APPIMAGE") {
            cmd.arg("--setenv")
                .arg(format!("CTX_APPIMAGE_PATH={appimage}"));
        }
        cmd.arg(&ctx_bin);
        cmd
    } else {
        let mut cmd = Command::new(&ctx_bin);
        if let Some(dist) = web_dist.as_ref() {
            cmd.env("CTX_WEB_DIST", dist.to_string_lossy().to_string());
        }
        if let Some(mcp) = mcp_bin.as_ref() {
            cmd.env("CTX_MCP_COMMAND", mcp.to_string_lossy().to_string());
        }
        if let Ok(appimage) = std::env::var("APPIMAGE") {
            cmd.env("CTX_APPIMAGE_PATH", appimage.clone());
        }
        cmd
    };

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
    let mut reader = BufReader::new(stdout).lines();

    let mut url: Option<String> = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if let Some(line) = reader.next() {
            let line = line?;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v.get("event").and_then(|e| e.as_str()) == Some("listening") {
                    if let Some(u) = v.get("url").and_then(|u| u.as_str()) {
                        url = Some(u.to_string());
                        break;
                    }
                }
            }
        } else {
            break;
        }
    }

    let url = url.context("daemon did not emit listening URL")?;
    Ok((url, child, use_systemd_scope))
}

#[allow(dead_code)]
fn is_executable(path: &Path) -> bool {
    path.exists()
}

fn try_kill_child(mut child: Child) -> Result<()> {
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
