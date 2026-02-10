use std::collections::{HashMap, HashSet};
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, NSObject, Sel};
#[cfg(target_os = "macos")]
use objc2::{msg_send, sel, AnyThread, ClassType, MainThreadMarker};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSBezelStyle, NSButton, NSColor, NSImage, NSImageNamePreferencesGeneral, NSLayoutAttribute,
    NSTitlebarAccessoryViewController, NSWindow,
};
#[cfg(target_os = "macos")]
use objc2_core_foundation::CGFloat;
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row, SqlitePool};
use tauri::Emitter;
use tauri::Manager;
#[cfg(feature = "automation")]
use tauri_plugin_automation::init as automation_init;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::OnceCell;
use url::Url;

fn main() {
    #[cfg(all(target_os = "windows", feature = "stt"))]
    {
        configure_windows_vosk_dll_search_path();
    }

    let mut builder = tauri::Builder::default()
        .manage(ConnectionManager::default())
        .manage(DeepLinkTokenStore::default())
        .manage(WorkspaceWindowRegistry::default())
        .manage(DesktopStorage::default());

    // Keep single-instance behavior for normal desktop usage. Automation builds need
    // isolated instances so tests don't attach to a long-running interactive app.
    #[cfg(not(feature = "automation"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_app_window(app);
        }));
    }

    builder = builder
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            desktop_get_connection,
            desktop_disconnect,
            desktop_connect_local,
            desktop_connect_ssh,
            desktop_list_ssh_hosts,
            desktop_test_ssh,
            desktop_list_ssh_paths,
            desktop_get_git_branch,
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
            desktop_set_titlebar_color,
            desktop_register_workspace_window,
            desktop_unregister_workspace_window,
            desktop_upload_blob,
            desktop_storage_get,
            desktop_storage_batch,
            desktop_daemon_request,
            desktop_start_codex_login_relay,
        ])
        .setup(|app| {
            open_main_window(&app.handle())?;
            schedule_force_launcher(app.handle().clone());
            schedule_startup_workspaces(app.handle().clone());
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
        });

    #[cfg(feature = "automation")]
    {
        builder = builder.plugin(automation_init());
    }

    #[cfg(feature = "stt")]
    {
        builder = builder.plugin(tauri_plugin_stt::init());
    }

    builder
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(all(target_os = "windows", feature = "stt"))]
fn configure_windows_vosk_dll_search_path() {
    use std::env;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
    }

    let Ok(exe) = env::current_exe() else {
        return;
    };
    let Some(exe_dir) = exe.parent().map(PathBuf::from) else {
        return;
    };

    let candidates = [
        exe_dir.clone(),
        exe_dir.join("bin"),
        exe_dir.join("resources").join("bin"),
        exe_dir.join("resources"),
    ];

    let Some(found_dir) = candidates
        .into_iter()
        .find(|dir| dir.join("libvosk.dll").exists())
    else {
        return;
    };

    let wide: Vec<u16> = OsStr::new(found_dir.as_os_str())
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        // Ignore failures here; if this doesn't work, STT will fail when used.
        let _ = SetDllDirectoryW(wide.as_ptr());
    }
}

fn schedule_startup_workspaces(app: tauri::AppHandle) {
    let Ok(raw) = std::env::var("CTX_DESKTOP_START_WORKSPACE_PATHS") else {
        return;
    };
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        return;
    }
    eprintln!("CTX_DESKTOP_START_WORKSPACE_PATHS detected: {raw}");

    // This is used for dev/headless UX verification. We intentionally schedule it after the app
    // starts so `app_data_dir()` and other platform services are ready.
    std::thread::spawn(move || {
        // Give the window time to initialize.
        for _ in 0..30 {
            if app.get_webview_window("main").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        let state = app.state::<ConnectionManager>();
        if let Err(e) = ensure_local_connection(&app, &state) {
            eprintln!("CTX_DESKTOP_START_WORKSPACE_PATHS: failed to ensure local daemon: {e:#}");
            return;
        }
        // Give the daemon a moment to finish bringing up workspace services even after `/health`
        // responds.
        std::thread::sleep(Duration::from_millis(300));

        let mut workspace_ids = Vec::new();
        for part in raw.split(';') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Ok(path) = validate_absolute_path(part) else {
                eprintln!(
                    "CTX_DESKTOP_START_WORKSPACE_PATHS: skipping invalid path (must be absolute): {part}"
                );
                continue;
            };
            let workspace_id = match resolve_or_create_workspace_id(&state, &path) {
                Ok(id) => id,
                Err(e) => {
                    eprintln!(
                        "CTX_DESKTOP_START_WORKSPACE_PATHS: failed to resolve/create workspace for {path}: {e:#}"
                    );
                    continue;
                }
            };
            if workspace_id.trim().is_empty() {
                eprintln!(
                    "CTX_DESKTOP_START_WORKSPACE_PATHS: daemon returned empty workspace id for {path}"
                );
                continue;
            };
            if !workspace_ids.contains(&workspace_id) {
                workspace_ids.push(workspace_id);
            }
        }
        if workspace_ids.is_empty() {
            eprintln!("CTX_DESKTOP_START_WORKSPACE_PATHS: no workspaces resolved; leaving window on launcher.");
            return;
        }

        let first = workspace_ids[0].clone();
        let tabs = workspace_ids
            .into_iter()
            .map(|id| urlencoding::encode(&id).into_owned())
            .collect::<Vec<_>>()
            .join(",");
        let url = format!("/workspaces/{}?ctxTabs={tabs}", urlencoding::encode(&first));
        if let Some(window) = app.get_webview_window("main") {
            eprintln!("CTX_DESKTOP_START_WORKSPACE_PATHS: navigating to {url}");
            let js = format!(
                "window.location.href = {};",
                serde_json::to_string(&url).unwrap_or_else(|_| "\"/workspaces\"".to_string())
            );
            let _ = window.eval(&js);
        }
    });
}

fn should_force_launcher() -> bool {
    if let Ok(start_path) = std::env::var("CTX_DESKTOP_START_PATH") {
        if start_path.trim().starts_with('/') {
            return false;
        }
    }
    if let Ok(raw) = std::env::var("CTX_DESKTOP_START_WORKSPACE_PATHS") {
        if !raw.trim().is_empty() {
            return false;
        }
    }
    true
}

fn schedule_force_launcher(app: tauri::AppHandle) {
    if !should_force_launcher() {
        return;
    }

    std::thread::spawn(move || {
        for _ in 0..30 {
            if let Some(window) = app.get_webview_window("main") {
                let js = r#"
(() => {
  const go = () => {
    try {
      const path = window.location.pathname || "/";
      if (path !== "/") {
        window.location.replace("/");
      }
    } catch {}
  };
  if (document.readyState === "loading") {
    window.addEventListener("DOMContentLoaded", go, { once: true });
  } else {
    go();
  }
})();
"#;
                let _ = window.eval(js);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
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
    #[serde(default = "default_true")]
    start_remote: bool,
    #[serde(default)]
    remote_data_dir: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct DesktopSshTestReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DesktopSshPathReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct DesktopSshPathEntry {
    name: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct DesktopGitBranchReq {
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct DesktopSshHost {
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
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

#[derive(Debug, Deserialize)]
struct DesktopCodexLoginRelayReq {
    login_id: String,
    callback_url: String,
    completion_token: String,
}

#[derive(Debug, Serialize)]
struct DesktopHttpResponse {
    status: u16,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DesktopStorageBatchOp {
    Set {
        key: String,
        value: serde_json::Value,
    },
    Delete {
        key: String,
    },
}

#[derive(Default)]
struct DesktopStorage {
    pool: OnceCell<SqlitePool>,
}

impl DesktopStorage {
    async fn pool(&self, app: &tauri::AppHandle) -> Result<&SqlitePool> {
        self.pool
            .get_or_try_init(|| async {
                let path = desktop_storage_path(app)?;
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                let options = SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(Duration::from_secs(5));
                let pool = SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await
                    .context("opening desktop storage sqlite db")?;
                ensure_ui_kv_schema(&pool).await?;
                Ok(pool)
            })
            .await
    }
}

async fn ensure_ui_kv_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ui_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
    )
    .execute(pool)
    .await
    .context("creating ui_kv table")?;

    let rows = sqlx::query("PRAGMA table_info(ui_kv)")
        .fetch_all(pool)
        .await
        .context("reading ui_kv schema")?;
    let mut has_key = false;
    let mut has_value = false;
    let mut has_updated = false;
    for row in rows {
        let name: String = row.try_get("name").context("reading ui_kv column name")?;
        match name.as_str() {
            "key" => has_key = true,
            "value" => has_value = true,
            "updated_at_ms" => has_updated = true,
            _ => {}
        }
    }

    if !has_key || !has_value {
        sqlx::query("DROP TABLE IF EXISTS ui_kv_legacy")
            .execute(pool)
            .await
            .context("dropping legacy ui_kv table")?;
        sqlx::query("ALTER TABLE ui_kv RENAME TO ui_kv_legacy")
            .execute(pool)
            .await
            .context("renaming legacy ui_kv table")?;
        sqlx::query(
            "CREATE TABLE ui_kv (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at_ms INTEGER NOT NULL)",
        )
        .execute(pool)
        .await
        .context("recreating ui_kv table")?;
        return Ok(());
    }

    if !has_updated {
        sqlx::query("ALTER TABLE ui_kv ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0")
            .execute(pool)
            .await
            .context("migrating ui_kv schema")?;
    }

    Ok(())
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
        DesktopDeepLinkToken {
            token,
            expires_at_ms,
        }
    }

    fn is_valid(&self, token: &str) -> bool {
        let mut tokens = self.tokens.lock().expect("deep link token lock");
        let now = Instant::now();
        tokens.retain(|_, expiry| *expiry > now);
        tokens
            .get(token)
            .map(|expiry| *expiry > now)
            .unwrap_or(false)
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
fn desktop_open_path(app: tauri::AppHandle, req: DesktopOpenPathReq) -> Result<(), String> {
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
fn desktop_read_file(req: DesktopOpenPathReq) -> Result<DesktopReadFileResp, String> {
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
    let text =
        std::fs::read_to_string(&resolved).map_err(|e| format!("failed to read file: {e}"))?;
    Ok(DesktopReadFileResp {
        path: resolved.to_string_lossy().to_string(),
        text,
    })
}

#[tauri::command]
fn desktop_list_ssh_hosts() -> Result<Vec<DesktopSshHost>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for path in ssh_config_paths() {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        for entry in parse_ssh_config(&text) {
            if seen.insert(entry.host.clone()) {
                out.push(entry);
            }
        }
    }
    Ok(out)
}

#[tauri::command]
async fn desktop_list_ssh_paths(
    req: DesktopSshPathReq,
) -> Result<Vec<DesktopSshPathEntry>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = req.host.trim().to_string();
        if host.is_empty() {
            return Err("host is required".to_string());
        }
        let target = match req.user.as_deref() {
            Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
            _ => host,
        };
        let raw = req.path.unwrap_or_default();
        let (parent, prefix) = split_remote_path(&raw);
        let cmd = format!("ls -a1 -p -- {}", remote_path_expr(&parent));
        let remote_cmd = format!("sh -lc {}", shell_escape(&cmd));
        let output = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg(target)
            // NOTE: sshd does not preserve argv boundaries for the remote command; pass as one string.
            .arg(remote_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn ssh: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if stderr.is_empty() {
                return Err("ssh failed to list paths".to_string());
            }
            return Err(format!("ssh failed: {stderr}"));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut entries = Vec::new();
        for line in stdout.lines() {
            let name = line.trim();
            if name.is_empty() {
                continue;
            }
            if !name.ends_with('/') {
                continue;
            }
            let name = name.trim_end_matches('/');
            if name == "." {
                continue;
            }
            if !prefix.is_empty() && !name.starts_with(&prefix) {
                continue;
            }
            let full_path = join_remote_path(&parent, name);
            entries.push(DesktopSshPathEntry {
                name: name.to_string(),
                path: full_path,
            });
        }
        Ok(entries)
    })
    .await
    .map_err(|e| format!("ssh list failed: {e}"))?
}

#[tauri::command]
async fn desktop_test_ssh(req: DesktopSshTestReq) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = req.host.trim().to_string();
        if host.is_empty() {
            return Err("host is required".to_string());
        }
        let target = match req.user.as_deref() {
            Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
            _ => host,
        };
        let output = Command::new("ssh")
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg(target)
            .arg("true")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| format!("failed to spawn ssh: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("ssh failed to connect".to_string());
        }
        Err(format!("ssh failed: {stderr}"))
    })
    .await
    .map_err(|e| format!("ssh check failed: {e}"))?
}

#[tauri::command]
async fn desktop_get_git_branch(req: DesktopGitBranchReq) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let raw = req.path.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        let mut path = expand_tilde(raw).unwrap_or_else(|| PathBuf::from(raw));
        if !path.is_absolute() {
            return Ok(None);
        }
        path = normalize_path(&path);
        let output = Command::new("git")
            .arg("-C")
            .arg(&path)
            .arg("rev-parse")
            .arg("--abbrev-ref")
            .arg("HEAD")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let Ok(output) = output else {
            return Ok(None);
        };
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() || value == "HEAD" {
            return Ok(None);
        }
        Ok(Some(value))
    })
    .await
    .map_err(|e| format!("git branch lookup failed: {e}"))?
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
    let builder =
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(url.into()))
            .title("ctx")
            .inner_size(1200.0, 900.0);
    let window = apply_workbench_titlebar(builder)
        .build()
        .map_err(|e| format!("creating window failed: {e}"))?;
    #[cfg(target_os = "macos")]
    {
        let _ = install_macos_settings_button(&app, &window);
    }
    let _ = window.show();
    let _ = window.set_focus();
    registry.register(&label, workspace_id);
    Ok(())
}

#[derive(Deserialize)]
struct DesktopTitlebarColor {
    r: f64,
    g: f64,
    b: f64,
    a: Option<f64>,
}

#[tauri::command]
fn desktop_set_titlebar_color(
    window: tauri::WebviewWindow,
    color: DesktopTitlebarColor,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let clamp_unit = |value: f64| (value.max(0.0).min(255.0) / 255.0) as CGFloat;
        let alpha = color.a.unwrap_or(1.0).max(0.0).min(1.0) as CGFloat;
        let r = clamp_unit(color.r);
        let g = clamp_unit(color.g);
        let b = clamp_unit(color.b);
        window
            .with_webview(move |webview| unsafe {
                let _mtm = MainThreadMarker::new().expect("titlebar color must be on main thread");
                let ns_window: &NSWindow = &*webview.ns_window().cast();
                ns_window.setTitlebarAppearsTransparent(false);
                let bg = NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, alpha);
                ns_window.setBackgroundColor(Some(&bg));
            })
            .map_err(|e| format!("failed to set titlebar color: {e}"))?;
    }
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
async fn desktop_git_clone(repo_url: String, dest_parent: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
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

        let name =
            derive_repo_name(&repo_url).ok_or_else(|| "could not derive repo name".to_string())?;
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
    })
    .await
    .map_err(|e| format!("git clone failed: {e}"))?
}

#[tauri::command]
async fn desktop_connect_local(app: tauri::AppHandle) -> Result<DesktopConnectionInfo, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        // Idempotent: if we're already connected to a healthy local daemon, keep the connection.
        // The workspace wizard calls connect_local as part of its flow; disconnecting here can
        // kill a just-started daemon and introduce flakiness on cold start.
        let info = state.info();
        if matches!(info.kind, DesktopConnectionKind::Local) {
            if let Some(url) = info.base_url.as_deref() {
                if probe_daemon_health(url).is_ok() {
                    return Ok(info);
                }
            }
        }
        state.disconnect();
        if let Some((url, token)) = resolve_env_local_daemon(&app).map_err(to_err)? {
            probe_daemon_health(&url).map_err(to_err)?;
            state.set_local_external(url, token);
            return Ok(state.info());
        }
        let data_dir = daemon_data_dir(&app).map_err(to_err)?;
        if let Some((url, token)) = resolve_existing_local_daemon(&data_dir).map_err(to_err)? {
            state.set_local_external(url, token);
            return Ok(state.info());
        }
        // Block until the daemon is actually reachable before returning. The workspace wizard
        // applies the connection and navigates immediately after `desktop_connect_local` resolves;
        // returning early causes the workbench to briefly render a "daemon unavailable" overlay.
        let (url, child, systemd_scope) = match spawn_daemon(&app, &data_dir, true) {
            Ok(value) => value,
            Err(err) => {
                if let Ok(Some((url, token))) = resolve_existing_local_daemon(&data_dir) {
                    state.set_local_external(url, token);
                    return Ok(state.info());
                }
                return Err(to_err(err));
            }
        };
        let auth = read_daemon_auth_with_retry(&data_dir).map_err(to_err)?;
        state.set_local(url.clone(), auth.token.clone(), child, systemd_scope);
        Ok(state.info())
    })
    .await
    .map_err(|e| format!("failed to connect to daemon: {e}"))?
}

#[tauri::command]
async fn desktop_connect_ssh(
    app: tauri::AppHandle,
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
    let remote_data_dir_for_connect = remote_data_dir.clone();
    let start_remote = req.start_remote;
    let host_for_provision = host.clone();
    let user_for_provision = user.clone();
    let app_for_provision = app.clone();
    let (base_url, token, tunnel) = tauri::async_runtime::spawn_blocking(move || {
        let no_start_remote = env_bool("CTX_DESKTOP_SSH_NO_START_REMOTE", false);

        // Prefer connecting to an already-running daemon. This avoids restarting/touching
        // the remote daemon when users (or tests) already have it running on the target port.
        let mut local_port = pick_unused_local_port()?;
        let (mut tunnel, tunnel_stderr) =
            start_ssh_tunnel(&host, user.as_deref(), local_port, remote_port)?;
        let mut base_url = format!("http://127.0.0.1:{local_port}");

        let mut health =
            probe_daemon_health_with_retry(&base_url, local_port, &mut tunnel, &tunnel_stderr);
        if health.is_err() && start_remote && !no_start_remote {
            let _ = try_kill_child(tunnel);
            start_remote_daemon_over_ssh(
                &host,
                user.as_deref(),
                remote_port,
                remote_data_dir_for_connect.as_deref(),
            )?;

            local_port = pick_unused_local_port()?;
            let (mut tunnel2, tunnel_stderr2) =
                start_ssh_tunnel(&host, user.as_deref(), local_port, remote_port)?;
            base_url = format!("http://127.0.0.1:{local_port}");
            health = probe_daemon_health_with_retry(
                &base_url,
                local_port,
                &mut tunnel2,
                &tunnel_stderr2,
            );
            if let Err(e) = health {
                let _ = try_kill_child(tunnel2);
                return Err(e);
            }
            tunnel = tunnel2;
        } else if let Err(e) = health {
            let _ = try_kill_child(tunnel);
            return Err(anyhow!(
                "{e:#}; remote start skipped (start_remote={start_remote}, no_start_remote={no_start_remote})"
            ));
        }

        let auth = read_remote_daemon_auth_with_retry(
            &host,
            user.as_deref(),
            remote_data_dir_for_connect.as_deref(),
        )?;
        Ok((base_url, auth.token, tunnel))
    })
    .await
    .map_err(|e| format!("failed to reach remote daemon: {e}"))?
    .map_err(|e| format!("failed to reach remote daemon: {e:#}"))?;

    state.set_ssh(base_url, Some(token), tunnel);

    // Best-effort provisioning: ensure the default harness image is present on remote Linux hosts
    // so restricted networking works without relying on registry pulls. We await completion so
    // users can immediately create remote container workspaces without racing image load.
    match tauri::async_runtime::spawn_blocking(move || {
        ensure_remote_ctx_harness_image(
            &app_for_provision,
            &host_for_provision,
            user_for_provision.as_deref(),
            remote_data_dir.as_deref(),
        )
    })
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("remote harness image provisioning skipped/failed: {err:#}"),
        Err(join_err) => eprintln!(
            "remote harness image provisioning task failed to join: {join_err:#}"
        ),
    }
    Ok(state.info())
}

fn ensure_local_connection(app: &tauri::AppHandle, state: &ConnectionManager) -> Result<()> {
    if !matches!(state.info().kind, DesktopConnectionKind::None) {
        return Ok(());
    }
    // Multiple webview requests can race on cold start (overlay pollers, initial data loads, etc.).
    // Serialize the "connect local" path so we don't concurrently spawn the daemon and trip the
    // daemon's lockfile, which can surface as spurious "daemon unavailable" errors in the UI.
    static LOCAL_CONNECT_MUTEX: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    let mutex = LOCAL_CONNECT_MUTEX.get_or_init(|| std::sync::Mutex::new(()));
    let _guard = mutex.lock().expect("local connect mutex poisoned");
    if !matches!(state.info().kind, DesktopConnectionKind::None) {
        return Ok(());
    }
    let data_dir = daemon_data_dir(app)?;
    if let Some((url, token)) = resolve_env_local_daemon(app)? {
        probe_daemon_health(&url)?;
        state.set_local_external(url, token);
        return Ok(());
    }
    if let Some((url, token)) = resolve_existing_local_daemon(&data_dir)? {
        state.set_local_external(url, token);
        return Ok(());
    }
    let (url, child, systemd_scope) = match spawn_daemon(app, &data_dir, true) {
        Ok(value) => value,
        Err(err) => {
            // This can happen if another thread already started the daemon but we raced before
            // the auth file became visible or health was reachable. Retry by waiting for the auth
            // file + health and then attaching as an external local connection.
            let auth = read_daemon_auth_with_retry(&data_dir)
                .with_context(|| format!("spawning local daemon failed: {err:#}"))?;
            let Some(url) = auth.daemon_url.as_deref() else {
                return Err(err).context("spawning local daemon failed (auth file missing daemon_url)");
            };
            probe_local_daemon_health_with_retry(url)?;
            state.set_local_external(url.to_string(), auth.token);
            return Ok(());
        }
    };
    let auth = read_daemon_auth_with_retry(&data_dir)?;
    state.set_local(url, auth.token, child, systemd_scope);
    Ok(())
}

#[tauri::command]
async fn desktop_daemon_request(
    app: tauri::AppHandle,
    req: DesktopDaemonRequest,
) -> Result<DesktopHttpResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        let manager: &ConnectionManager = state.inner();
        // Many UI paths (including initial app load to a workbench route) can issue daemon requests
        // before explicitly calling `desktop_connect_local`. Auto-connect here to avoid spurious
        // "daemon unavailable" overlays on cold start.
        ensure_local_connection(&app, manager).map_err(to_err)?;
        manager.daemon_request(req).map_err(to_err)
    })
    .await
    .map_err(|e| format!("daemon request failed: {e}"))?
}

fn is_loopback_host_name(host: &str) -> bool {
    let value = host.trim().to_ascii_lowercase();
    if value == "localhost" {
        return true;
    }
    value
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn read_http_request_target(stream: &mut TcpStream) -> Result<String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .context("setting relay read timeout")?;
    let mut buf = [0u8; 16384];
    let read = stream.read(&mut buf).context("reading callback request")?;
    if read == 0 {
        anyhow::bail!("empty callback request");
    }
    let request = String::from_utf8_lossy(&buf[..read]);
    let first_line = request
        .lines()
        .next()
        .ok_or_else(|| anyhow!("callback request missing request line"))?;
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" {
        anyhow::bail!("unsupported callback method: {method}");
    }
    if target.trim().is_empty() {
        anyhow::bail!("callback request missing target path");
    }
    Ok(target.trim().to_string())
}

fn write_http_response(stream: &mut TcpStream, status: &str, message: &str) {
    let html = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>ctx Codex Login</title></head><body><h2>{status}</h2><p>{message}</p><p>You can now return to ctx.</p></body></html>"
    );
    let payload = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

fn callback_url_from_target(target: &str, expected_path: &str, expected_port: u16) -> Result<String> {
    let mut url = if target.starts_with("http://") || target.starts_with("https://") {
        Url::parse(target).context("parsing callback request target URL")?
    } else {
        Url::parse(&format!("http://localhost{target}"))
            .context("parsing callback request target path")?
    };
    if url.path() != expected_path {
        anyhow::bail!("callback path mismatch");
    }
    if url.query().is_none() {
        anyhow::bail!("callback query is missing");
    }
    if !is_loopback_host_name(url.host_str().unwrap_or_default()) {
        anyhow::bail!("callback host must be loopback");
    }
    let _ = url.set_scheme("http");
    let _ = url.set_host(Some("127.0.0.1"));
    let _ = url.set_port(Some(expected_port));
    Ok(url.to_string())
}

fn process_codex_login_relay_connection(
    app: &tauri::AppHandle,
    mut stream: TcpStream,
    login_id: &str,
    completion_token: &str,
    expected_path: &str,
    expected_port: u16,
) -> Result<()> {
    let target = match read_http_request_target(&mut stream) {
        Ok(target) => target,
        Err(err) => {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "Invalid callback request. Retry from ctx Settings.",
            );
            return Err(err);
        }
    };
    let callback_url = match callback_url_from_target(&target, expected_path, expected_port) {
        Ok(url) => url,
        Err(err) => {
            write_http_response(
                &mut stream,
                "400 Bad Request",
                "Callback URL validation failed. Retry from ctx Settings.",
            );
            return Err(err);
        }
    };

    let state = app.state::<ConnectionManager>();
    let manager: &ConnectionManager = state.inner();
    ensure_local_connection(app, manager).context("ensuring daemon connection")?;
    let body = serde_json::json!({
        "callback_url": callback_url,
        "completion_token": completion_token,
    })
    .to_string();
    let response = manager.daemon_request(DesktopDaemonRequest {
        method: "POST".to_string(),
        path: format!("/api/providers/codex/accounts/login/{login_id}"),
        body: Some(body),
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
    })?;
    if (200..300).contains(&response.status) {
        write_http_response(
            &mut stream,
            "200 OK",
            "Codex login callback received. Completing sign-in.",
        );
        return Ok(());
    }

    write_http_response(
        &mut stream,
        "502 Bad Gateway",
        "ctx could not complete login relay on the daemon. Use manual callback paste in Settings.",
    );
    anyhow::bail!("daemon callback completion failed with status {}", response.status)
}

#[tauri::command]
async fn desktop_start_codex_login_relay(
    app: tauri::AppHandle,
    req: DesktopCodexLoginRelayReq,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let login_id = req.login_id.trim().to_string();
        if login_id.is_empty() {
            return Err(anyhow!("login_id is required"));
        }
        let completion_token = req.completion_token.trim().to_string();
        if completion_token.is_empty() {
            return Err(anyhow!("completion_token is required"));
        }
        let callback_url = Url::parse(req.callback_url.trim())
            .context("invalid callback_url for relay listener")?;
        if callback_url.scheme() != "http" {
            anyhow::bail!("callback_url must use http");
        }
        let host = callback_url
            .host_str()
            .ok_or_else(|| anyhow!("callback_url missing host"))?
            .to_string();
        if !is_loopback_host_name(&host) {
            anyhow::bail!("callback_url host must be loopback");
        }
        let port = callback_url
            .port()
            .ok_or_else(|| anyhow!("callback_url missing explicit port"))?;
        let expected_path = callback_url.path().to_string();
        if !expected_path.starts_with("/auth/callback") {
            anyhow::bail!("callback_url path must start with /auth/callback");
        }

        let listener = TcpListener::bind((host.as_str(), port))
            .with_context(|| format!("binding codex callback relay on {host}:{port}"))?;
        listener
            .set_nonblocking(true)
            .context("setting callback relay nonblocking")?;

        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5 * 60);
            loop {
                if Instant::now() >= deadline {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _addr)) => {
                        if let Err(err) = process_codex_login_relay_connection(
                            &app,
                            stream,
                            &login_id,
                            &completion_token,
                            &expected_path,
                            port,
                        ) {
                            eprintln!("codex login relay failed: {err:#}");
                        }
                        break;
                    }
                    Err(err) if err.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    Err(err) => {
                        eprintln!("codex login relay accept failed: {err}");
                        break;
                    }
                }
            }
        });
        Ok(true)
    })
    .await
    .map_err(|e| format!("starting codex relay failed: {e}"))?
    .map_err(to_err)
}

#[tauri::command]
async fn desktop_upload_blob(
    app: tauri::AppHandle,
    bytes: Vec<u8>,
    mime_type: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ConnectionManager>();
        state.upload_blob(bytes, mime_type, name).map_err(to_err)
    })
    .await
    .map_err(|e| format!("blob upload failed: {e}"))?
}

#[tauri::command]
async fn desktop_storage_get(
    app: tauri::AppHandle,
    storage: tauri::State<'_, DesktopStorage>,
    key: String,
) -> Result<Option<serde_json::Value>, String> {
    let pool = storage.pool(&app).await.map_err(to_err)?;
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM ui_kv WHERE key = ?1")
        .bind(&key)
        .fetch_optional(pool)
        .await
        .map_err(to_err)?;
    if let Some((value,)) = row {
        match serde_json::from_str(&value) {
            Ok(parsed) => Ok(Some(parsed)),
            Err(err) => {
                let _ = sqlx::query("DELETE FROM ui_kv WHERE key = ?1")
                    .bind(&key)
                    .execute(pool)
                    .await;
                eprintln!("dropping corrupt ui_kv value for key {key}: {err}");
                Ok(None)
            }
        }
    } else {
        Ok(None)
    }
}

#[tauri::command]
async fn desktop_storage_batch(
    app: tauri::AppHandle,
    storage: tauri::State<'_, DesktopStorage>,
    ops: Vec<DesktopStorageBatchOp>,
) -> Result<(), String> {
    if ops.is_empty() {
        return Ok(());
    }
    let pool = storage.pool(&app).await.map_err(to_err)?;
    let mut tx = pool.begin().await.map_err(to_err)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    for op in ops {
        match op {
            DesktopStorageBatchOp::Set { key, value } => {
                let value_json = serde_json::to_string(&value).map_err(to_err)?;
                sqlx::query(
                    "INSERT INTO ui_kv (key, value, updated_at_ms) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at_ms=excluded.updated_at_ms",
                )
                .bind(&key)
                .bind(&value_json)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(to_err)?;
            }
            DesktopStorageBatchOp::Delete { key } => {
                sqlx::query("DELETE FROM ui_kv WHERE key = ?1")
                    .bind(&key)
                    .execute(&mut *tx)
                    .await
                    .map_err(to_err)?;
            }
        }
    }
    tx.commit().await.map_err(to_err)?;
    Ok(())
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
    let params: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
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
    value.and_then(|v| v.parse::<u32>().ok()).filter(|v| *v > 0)
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
    if candidate
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
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
    let in_open_workspace =
        is_target_in_open_workspace(state, registry, &req.target).unwrap_or(false);
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
    let in_open_workspace =
        is_target_in_open_workspace(state, registry, &req.target).unwrap_or(false);
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

fn open_in_ctx(
    app: &tauri::AppHandle,
    target: &DeepLinkTarget,
    line: Option<u32>,
    col: Option<u32>,
) -> Result<()> {
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
    let target = override_target
        .cloned()
        .unwrap_or_else(|| settings.target.clone());
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
        headers: vec![("content-type".to_string(), "application/json".to_string())],
    })?;
    if resp.status != 200 && resp.status != 201 {
        anyhow::bail!(
            "failed to create workspace ({status}): {body}",
            status = resp.status,
            body = resp.body
        );
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing workspace response")?;
    parse_id_value(&value["id"]).ok_or_else(|| anyhow!("workspace id missing"))
}

fn resolve_workspace_id_by_path(
    state: &ConnectionManager,
    root_path: &str,
) -> Result<Option<String>> {
    let resp = state.daemon_request(DesktopDaemonRequest {
        method: "GET".to_string(),
        path: "/api/workspaces".to_string(),
        body: None,
        headers: vec![],
    })?;
    if resp.status != 200 {
        anyhow::bail!(
            "failed to list workspaces ({status}): {body}",
            status = resp.status,
            body = resp.body
        );
    }
    let value: serde_json::Value =
        serde_json::from_str(&resp.body).context("parsing workspaces response")?;
    let arr = value
        .as_array()
        .ok_or_else(|| anyhow!("workspaces response is not a list"))?;
    for entry in arr {
        let ws_root = entry
            .get("root_path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
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
        anyhow::bail!(
            "failed to load workspace ({status}): {body}",
            status = resp.status,
            body = resp.body
        );
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
        anyhow::bail!(
            "failed to load worktree ({status}): {body}",
            status = resp.status,
            body = resp.body
        );
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
        .or_else(|| {
            value
                .get("0")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
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
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Open".into(),
            "Cancel".into(),
        ))
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

#[cfg(target_os = "macos")]
static SETTINGS_BUTTON_APP: OnceLock<tauri::AppHandle> = OnceLock::new();
#[cfg(target_os = "macos")]
static SETTINGS_BUTTON_CLASS: Once = Once::new();

#[cfg(target_os = "macos")]
extern "C" fn settings_button_clicked(_this: &AnyObject, _cmd: Sel, _sender: *mut AnyObject) {
    if let Some(app) = SETTINGS_BUTTON_APP.get() {
        emit_settings_inplace(app, _this);
    }
}

#[cfg(target_os = "macos")]
fn emit_settings_inplace(app: &tauri::AppHandle, target: &AnyObject) {
    const WINDOW_LABEL_IVAR: &[u8] = b"ctxWindowLabel\0";
    let ivar = settings_button_target_class()
        .instance_variable(CStr::from_bytes_with_nul(WINDOW_LABEL_IVAR).unwrap());
    if let Some(ivar) = ivar {
        let label_ptr = unsafe { *ivar.load::<*const std::ffi::c_char>(target) };
        if !label_ptr.is_null() {
            let label = unsafe { CStr::from_ptr(label_ptr) }
                .to_string_lossy()
                .into_owned();
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.eval(
                    "(() => { let t = '/settings'; const p = window.location.pathname || ''; \
                     if (p.startsWith('/workspaces/')) { const ws = p.split('/')[2]; if (ws) { t = `/settings?ws=${encodeURIComponent(ws)}`; } } \
                     window.location.assign(t); })();",
                );
                return;
            }
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval("window.location.assign('/settings');");
        return;
    }
    let _ = app.emit("desktop_open_settings", ());
}

#[cfg(target_os = "macos")]
fn settings_button_target_class() -> &'static AnyClass {
    const CLASS_NAME: &[u8] = b"CtxSettingsButtonTarget\0";
    const WINDOW_LABEL_IVAR: &[u8] = b"ctxWindowLabel\0";
    SETTINGS_BUTTON_CLASS.call_once(|| {
        let class_name = CStr::from_bytes_with_nul(CLASS_NAME)
            .expect("settings button class name should be valid");
        let mut builder = ClassBuilder::new(class_name, NSObject::class())
            .expect("settings button class should be registerable");
        let ivar_name = CStr::from_bytes_with_nul(WINDOW_LABEL_IVAR)
            .expect("settings button ivar name should be valid");
        builder.add_ivar::<*const std::ffi::c_char>(ivar_name);
        unsafe {
            let open_settings: extern "C" fn(&'static AnyObject, Sel, *mut AnyObject) =
                settings_button_clicked;
            builder.add_method(sel!(openSettings:), open_settings);
        }
        builder.register();
    });
    let class_name =
        CStr::from_bytes_with_nul(CLASS_NAME).expect("settings button class name should be valid");
    AnyClass::get(class_name).expect("settings button class should be registered")
}

#[cfg(target_os = "macos")]
fn install_macos_settings_button(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) -> Result<()> {
    SETTINGS_BUTTON_APP.get_or_init(|| app.clone());
    let window_label = window.label().to_string();
    let icon_path = app.path().resource_dir().ok().and_then(|dir| {
        dir.join("bundles/lucide-settings.svg")
            .to_str()
            .map(str::to_string)
    });
    window.with_webview(move |webview| unsafe {
        let mtm = MainThreadMarker::new().expect("titlebar button should be on main thread");
        let ns_window: &NSWindow = &*webview.ns_window().cast();
        let image = icon_path
            .as_deref()
            .and_then(|path| load_lucide_settings_icon(path))
            .or_else(|| NSImage::imageNamed(NSImageNamePreferencesGeneral));
        let Some(image) = image else {
            return;
        };
        let cls = settings_button_target_class();
        let target: Retained<AnyObject> = msg_send![cls, new];
        let target = &*Retained::into_raw(target);
        let label_ptr = CString::new(window_label.as_str())
            .expect("window label should be valid")
            .into_raw();
        let ivar = settings_button_target_class()
            .instance_variable(CStr::from_bytes_with_nul(b"ctxWindowLabel\0").unwrap())
            .expect("settings button ivar should exist");
        ivar.load_ptr::<*const std::ffi::c_char>(target)
            .write(label_ptr as *const std::ffi::c_char);
        let button = NSButton::buttonWithImage_target_action(
            &image,
            Some(target),
            Some(sel!(openSettings:)),
            mtm,
        );
        button.setBezelStyle(NSBezelStyle::Toolbar);

        let accessory = NSTitlebarAccessoryViewController::new(mtm);
        accessory.setView(button.as_ref());
        accessory.setLayoutAttribute(NSLayoutAttribute::Trailing);
        accessory.setAutomaticallyAdjustsSize(true);
        ns_window.addTitlebarAccessoryViewController(&accessory);
    })?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn load_lucide_settings_icon(icon_path: &str) -> Option<Retained<NSImage>> {
    let ns_path = NSString::from_str(icon_path);
    let image = NSImage::initWithContentsOfFile(NSImage::alloc(), &ns_path)?;
    image.setTemplate(true);
    Some(image)
}

fn apply_workbench_titlebar<'a, R: tauri::Runtime, M: tauri::Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    #[cfg(target_os = "macos")]
    {
        return builder
            .title_bar_style(tauri::TitleBarStyle::Visible)
            .hidden_title(false);
    }
    #[cfg(not(target_os = "macos"))]
    {
        return builder.decorations(false);
    }
}

fn open_main_window(app: &tauri::AppHandle) -> Result<()> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    let start_url = match std::env::var("CTX_DESKTOP_START_PATH") {
        Ok(v) if v.trim().starts_with('/') => tauri::WebviewUrl::App(v.trim().into()),
        _ => tauri::WebviewUrl::App("index.html".into()),
    };
    let mut builder = tauri::WebviewWindowBuilder::new(app, "main", start_url).title("ctx");
    if let Ok(Some(monitor)) = app.primary_monitor() {
        let size = monitor.size();
        let width = (size.width as f64 * 0.9).round().max(1200.0);
        let height = (size.height as f64 * 0.9).round().max(900.0);
        builder = builder.inner_size(width, height);
    } else {
        builder = builder.inner_size(1200.0, 900.0);
    }
    let builder = apply_workbench_titlebar(builder);
    let window = builder.build().context("creating window")?;
    #[cfg(target_os = "macos")]
    {
        let _ = install_macos_settings_button(app, &window);
    }
    let _ = window.show();
    let _ = window.set_focus();
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
    LocalExternal(LocalExternalConnection),
    Ssh(SshConnection),
}

struct LocalConnection {
    base_url: String,
    token: String,
    child: Child,
    systemd_scope: bool,
}

struct LocalExternalConnection {
    base_url: String,
    token: String,
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
            return DesktopConnectionInfo {
                kind: DesktopConnectionKind::None,
                base_url: None,
                token: None,
            };
        };
        match &guard.active {
            None => DesktopConnectionInfo {
                kind: DesktopConnectionKind::None,
                base_url: None,
                token: None,
            },
            Some(ActiveConnection::Local(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                token: Some(c.token.clone()),
            },
            Some(ActiveConnection::LocalExternal(c)) => DesktopConnectionInfo {
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
        matches!(
            guard.as_ref().and_then(|g| g.active.as_ref()),
            Some(ActiveConnection::Ssh(_))
        )
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
                        stop_systemd_scope("ctx-daemon");
                        if let Some(scope) = systemd_scope_for_local_daemon_url(&c.base_url) {
                            stop_systemd_scope(&scope);
                        }
                    }
                    let _ = try_kill_child(c.child);
                }
                ActiveConnection::LocalExternal(_) => {}
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

    fn set_local_external(&self, base_url: String, token: String) {
        let mut guard = self.0.lock().expect("connection manager lock");
        guard.active = Some(ActiveConnection::LocalExternal(LocalExternalConnection {
            base_url,
            token,
        }));
    }

    fn set_ssh(&self, base_url: String, token: Option<String>, tunnel: Child) {
        let mut guard = self.0.lock().expect("connection manager lock");
        guard.active = Some(ActiveConnection::Ssh(SshConnection {
            base_url,
            token,
            tunnel,
        }));
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
                ActiveConnection::LocalExternal(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::Ssh(c) => (c.base_url.clone(), c.token.clone()),
            }
        };

        let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
        // Some daemon operations (notably container provisioning on first run) can legitimately
        // take minutes. Keep a short connect timeout so a dead daemon fails fast, but allow
        // long-running requests to complete.
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10 * 60))
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
        Ok(DesktopHttpResponse {
            status,
            body,
            content_type,
        })
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
                ActiveConnection::LocalExternal(c) => (c.base_url.clone(), Some(c.token.clone())),
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

fn desktop_storage_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = app
        .path()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("web").join("ui_state.sqlite"))
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

fn ssh_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = expand_tilde("~/.ssh/config") {
        paths.push(path);
    }
    let system_config = PathBuf::from("/etc/ssh/ssh_config");
    paths.push(system_config.clone());
    let system_dir = PathBuf::from("/etc/ssh/ssh_config.d");
    if let Ok(entries) = std::fs::read_dir(system_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths
}

fn parse_ssh_config(text: &str) -> Vec<DesktopSshHost> {
    let mut out = Vec::new();
    let mut current_hosts: Vec<String> = Vec::new();
    let mut current_user: Option<String> = None;
    let mut current_host_name: Option<String> = None;
    let mut current_port: Option<u16> = None;

    let mut flush = |hosts: &Vec<String>,
                     user: &Option<String>,
                     host_name: &Option<String>,
                     port: &Option<u16>,
                     out: &mut Vec<DesktopSshHost>| {
        if hosts.is_empty() {
            return;
        }
        for host in hosts {
            if is_ssh_pattern(host) {
                continue;
            }
            out.push(DesktopSshHost {
                host: host.to_string(),
                user: user.clone(),
                host_name: host_name.clone(),
                port: *port,
            });
        }
    };

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let key = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        if key.eq_ignore_ascii_case("host") {
            flush(
                &current_hosts,
                &current_user,
                &current_host_name,
                &current_port,
                &mut out,
            );
            current_hosts = rest.iter().map(|v| v.to_string()).collect();
            current_user = None;
            current_host_name = None;
            current_port = None;
            continue;
        }
        if current_hosts.is_empty() {
            continue;
        }
        if key.eq_ignore_ascii_case("user") {
            current_user = rest.first().map(|v| v.to_string());
        } else if key.eq_ignore_ascii_case("hostname") {
            current_host_name = rest.first().map(|v| v.to_string());
        } else if key.eq_ignore_ascii_case("port") {
            current_port = rest.first().and_then(|v| v.parse::<u16>().ok());
        }
    }

    flush(
        &current_hosts,
        &current_user,
        &current_host_name,
        &current_port,
        &mut out,
    );
    out
}

fn is_ssh_pattern(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty()
        || trimmed.starts_with('!')
        || trimmed.contains('*')
        || trimmed.contains('?')
        || trimmed.contains('[')
        || trimmed.contains(']')
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
                let authority = remote_authority
                    .ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("vscode", authority, path, line, col))
            }
            DesktopEditorTarget::VsCodeInsiders => {
                let authority = remote_authority
                    .ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri(
                    "vscode-insiders",
                    authority,
                    path,
                    line,
                    col,
                ))
            }
            DesktopEditorTarget::Cursor => {
                let authority = remote_authority
                    .ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("cursor", authority, path, line, col))
            }
            DesktopEditorTarget::Windsurf => {
                let authority = remote_authority
                    .ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri("windsurf", authority, path, line, col))
            }
            DesktopEditorTarget::Antigravity => {
                let authority = remote_authority
                    .ok_or_else(|| anyhow!("remote authority is not configured"))?;
                open_with_system(&vscode_remote_uri(
                    "antigravity",
                    authority,
                    path,
                    line,
                    col,
                ))
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
            DesktopEditorTarget::Windsurf => {
                open_with_system(&vscode_uri("windsurf", path, line, col))
            }
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
    let mut uri = format!(
        "{scheme}://vscode-remote/{authority}{}",
        encode_uri_path(path)
    );
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
const LOCAL_DAEMON_HEALTH_RETRIES: usize = 20;
const LOCAL_DAEMON_HEALTH_BASE_DELAY_MS: u64 = 100;
const DESKTOP_DAEMON_DATA_DIR_ENV: &str = "CTX_DESKTOP_DAEMON_DATA_DIR";
const DESKTOP_REMOTE_CTX_BIN_ENV: &str = "CTX_DESKTOP_REMOTE_CTX_BIN";
const DAEMON_ENV_PASSTHROUGH: &[&str] = &[
    "CTX_ALLOW_SYSTEM_PODMAN",
    "CTX_PODMAN_PATH",
    "CTX_PODMAN_MACHINE_PREFETCH",
];

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
        .arg("ConnectTimeout=15")
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

    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let log_dir = format!("{}/logs", data_dir.trim_end_matches('/'));
    let log_dir_expr = remote_path_expr(&log_dir);
    let log_file = format!("{}/daemon.log", log_dir.trim_end_matches('/'));
    let log_file_expr = remote_path_expr(&log_file);
    let configured_ctx_bin = std::env::var(DESKTOP_REMOTE_CTX_BIN_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    // Remote daemon should be able to use host Podman when available (same behavior expected in
    // launcher container modes on Linux remotes). Pass through optional podman tuning envs.
    let mut daemon_env = vec!["CTX_ALLOW_SYSTEM_PODMAN=1".to_string()];
    if let Ok(v) = std::env::var("CTX_PODMAN_PATH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            daemon_env.push(format!("CTX_PODMAN_PATH={}", shell_escape(trimmed)));
        }
    }
    if let Ok(v) = std::env::var("CTX_PODMAN_MACHINE_PREFETCH") {
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            daemon_env.push(format!(
                "CTX_PODMAN_MACHINE_PREFETCH={}",
                shell_escape(trimmed)
            ));
        }
    }
    let daemon_env_prefix = if daemon_env.is_empty() {
        String::new()
    } else {
        format!("env {} ", daemon_env.join(" "))
    };
    let exec_cmd = if let Some(ctx_bin) = configured_ctx_bin {
        let ctx_bin_expr = remote_path_expr(&ctx_bin);
        format!(
            "if [ -x {ctx_bin} ]; then {env}{ctx_bin} serve --bind 127.0.0.1:{remote_port} --data-dir {dir}; else echo 'ctx not executable at configured remote path' >&2; exit 127; fi",
            env = daemon_env_prefix,
            ctx_bin = ctx_bin_expr,
            dir = remote_path_expr(data_dir),
        )
    } else {
        format!(
            "if command -v ctx >/dev/null 2>&1; then {env}ctx serve --bind 127.0.0.1:{remote_port} --data-dir {dir}; else echo 'ctx not found on PATH' >&2; exit 127; fi",
            env = daemon_env_prefix,
            dir = remote_path_expr(data_dir),
        )
    };
    let log_cmd = format!(
        "mkdir -p {log_dir} && {exec_cmd} > {log_file} 2>&1",
        log_dir = log_dir_expr,
        log_file = log_file_expr,
    );
    // Always start the remote daemon via nohup so the SSH command returns immediately.
    // systemd-run --scope for a long-lived daemon can keep the SSH session open indefinitely.
    let remote_cmd = format!(
        "mkdir -p {log_dir} && nohup /bin/sh -lc {cmd} >/dev/null 2>&1 < /dev/null &",
        log_dir = log_dir_expr,
        cmd = shell_escape(&log_cmd),
    );

    let output = Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        // NOTE: sshd does not preserve argv boundaries for the remote command; pass as one string.
        .arg(format!("sh -lc {}", shell_escape(&remote_cmd)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("starting remote daemon over ssh")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "ssh start failed: {stderr}; remote_cmd={remote_cmd}"
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

fn split_remote_path(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ("~".to_string(), String::new());
    }
    if trimmed == "~" || trimmed.ends_with('/') {
        return (trimmed.to_string(), String::new());
    }
    if let Some((parent, suffix)) = trimmed.rsplit_once('/') {
        if parent.is_empty() {
            return ("/".to_string(), suffix.to_string());
        }
        return (parent.to_string(), suffix.to_string());
    }
    ("~".to_string(), trimmed.to_string())
}

fn join_remote_path(parent: &str, name: &str) -> String {
    if parent == "~" || parent == "~/" {
        return format!("~/{name}");
    }
    if parent == "/" {
        return format!("/{name}");
    }
    format!("{}/{}", parent.trim_end_matches('/'), name)
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
                    anyhow::Error::new(err)
                        .context(format!("reading daemon auth file {}", path.display())),
                );
            }
        }
        if Instant::now() > deadline {
            return Err(last_err
                .unwrap_or_else(|| anyhow!("daemon auth file not found at {}", path.display())));
        }
        std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
    }
}

fn read_daemon_auth_if_present(data_dir: &Path) -> Result<Option<DaemonAuthFile>> {
    let path = data_dir.join(DAEMON_AUTH_FILENAME);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(Some(parse_daemon_auth(&bytes, &path)?)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(anyhow::Error::new(err)
                .context(format!("reading daemon auth file {}", path.display())))
        }
    }
}

fn resolve_env_local_daemon(app: &tauri::AppHandle) -> Result<Option<(String, String)>> {
    let url = match std::env::var("CTX_DESKTOP_DAEMON_URL") {
        Ok(v) => v.trim().to_string(),
        Err(_) => return Ok(None),
    };
    if url.is_empty() {
        return Ok(None);
    }
    let token = match std::env::var("CTX_DESKTOP_DAEMON_TOKEN") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            let data_dir = daemon_data_dir(app)?;
            match read_daemon_auth_if_present(&data_dir)? {
                Some(auth) if auth.daemon_url.as_deref() == Some(url.as_str()) => auth.token,
                _ => {
                    anyhow::bail!(
                        "CTX_DESKTOP_DAEMON_URL is set but no matching token found (set CTX_DESKTOP_DAEMON_TOKEN)"
                    );
                }
            }
        }
    };
    Ok(Some((url, token)))
}

fn resolve_existing_local_daemon(data_dir: &Path) -> Result<Option<(String, String)>> {
    let Some(auth) = read_daemon_auth_if_present(data_dir)? else {
        return Ok(None);
    };
    let Some(url) = auth.daemon_url.as_deref() else {
        return Ok(None);
    };
    match probe_daemon_health(url) {
        Ok(()) => Ok(Some((url.to_string(), auth.token))),
        Err(_) => Ok(None),
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
    let auth_path = format!(
        "{}/{}",
        data_dir.trim_end_matches('/'),
        DAEMON_AUTH_FILENAME
    );
    let cmd = format!("cat -- {}", remote_path_expr(&auth_path));
    let remote_cmd = format!("sh -lc {}", shell_escape(&cmd));

    let output = Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=15")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=2")
        .arg(target)
        // NOTE: sshd does not preserve argv boundaries for the remote command; pass as one string.
        .arg(remote_cmd)
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
            return Err(
                last_err.unwrap_or_else(|| anyhow!("timed out reading daemon auth file over ssh"))
            );
        }
        std::thread::sleep(DAEMON_AUTH_RETRY_DELAY);
    }
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledAssetsManifest {
    #[allow(dead_code)]
    pub version: u32,
    #[serde(default)]
    pub images: Vec<DesktopBundledImage>,
}

#[derive(Debug, Clone, Deserialize)]
struct DesktopBundledImage {
    pub id: String,
    pub os: String,
    pub arch: String,
    pub tar: String,
    pub image: String,
}

fn normalize_arch_token(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "x86_64" | "amd64" => Some("x86_64"),
        "aarch64" | "arm64" => Some("aarch64"),
        _ => None,
    }
}

fn read_bundled_ctx_harness_image(app: &tauri::AppHandle, arch: &str) -> Result<(PathBuf, String)> {
    let bundle_dir = desktop_bundle_dir(app).ok_or_else(|| anyhow!("bundle dir not found"))?;
    let manifest_path = bundle_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest: DesktopBundledAssetsManifest =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", manifest_path.display()))?;
    let entry = manifest
        .images
        .iter()
        .find(|img| img.id == "ctx-harness" && img.os == "linux" && img.arch == arch)
        .ok_or_else(|| anyhow!("bundled ctx-harness image tar not found for linux/{arch}"))?;
    let tar = bundle_dir.join(&entry.tar);
    if !tar.exists() {
        anyhow::bail!(
            "bundled ctx-harness image tar missing at {}",
            tar.display()
        );
    }
    Ok((tar, entry.image.clone()))
}

fn ssh_target(host: &str, user: Option<&str>) -> String {
    match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    }
}

fn ssh_output(target: &str, cmd: &str) -> Result<std::process::Output> {
    let remote_cmd = format!("sh -lc {}", shell_escape(cmd));
    let output = Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-o")
        .arg("ConnectionAttempts=1")
        .arg("-o")
        .arg("ServerAliveInterval=5")
        .arg("-o")
        .arg("ServerAliveCountMax=1")
        .arg(target)
        .arg(remote_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("ssh: {cmd}"))?;
    Ok(output)
}

fn ensure_remote_ctx_harness_image(
    app: &tauri::AppHandle,
    host: &str,
    user: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<()> {
    let target = ssh_target(host, user);
    let data_dir = remote_data_dir
        .filter(|d| !d.trim().is_empty())
        .unwrap_or("~/.ctx");
    let podman_xdg_root = format!("{}/podman/xdg", data_dir.trim_end_matches('/'));
    let podman_xdg_config = format!("{}/config", podman_xdg_root);
    let podman_xdg_data = format!("{}/data", podman_xdg_root);
    let podman_xdg_run = format!("{}/run", podman_xdg_root);
    let podman_env_prefix = format!(
        "XDG_CONFIG_HOME={} XDG_DATA_HOME={} XDG_RUNTIME_DIR={}",
        remote_path_expr(&podman_xdg_config),
        remote_path_expr(&podman_xdg_data),
        remote_path_expr(&podman_xdg_run),
    );
    let podman_prepare_cmd = format!(
        "mkdir -p {} {} {} && chmod 700 {} >/dev/null 2>&1 || true",
        remote_path_expr(&podman_xdg_config),
        remote_path_expr(&podman_xdg_data),
        remote_path_expr(&podman_xdg_run),
        remote_path_expr(&podman_xdg_run),
    );

    // Only provision the Linux container image on Linux hosts.
    let os_out = ssh_output(&target, "uname -s")?;
    if !os_out.status.success() {
        anyhow::bail!(
            "ssh uname failed: {}",
            String::from_utf8_lossy(&os_out.stderr).trim()
        );
    }
    let os = String::from_utf8_lossy(&os_out.stdout).trim().to_string();
    if os != "Linux" {
        return Ok(());
    }

    let arch_out = ssh_output(&target, "uname -m")?;
    if !arch_out.status.success() {
        anyhow::bail!(
            "ssh uname -m failed: {}",
            String::from_utf8_lossy(&arch_out.stderr).trim()
        );
    }
    let arch_raw = String::from_utf8_lossy(&arch_out.stdout).trim().to_string();
    let Some(arch) = normalize_arch_token(&arch_raw) else {
        anyhow::bail!("unsupported remote architecture: {arch_raw}");
    };

    // If the remote doesn't have podman, don't block ssh connection (container mode just won't work).
    let podman_out = ssh_output(&target, "command -v podman >/dev/null 2>&1")?;
    if !podman_out.status.success() {
        return Ok(());
    }
    let prep_out = ssh_output(&target, &podman_prepare_cmd)?;
    if !prep_out.status.success() {
        anyhow::bail!(
            "ssh podman xdg setup failed: {}",
            String::from_utf8_lossy(&prep_out.stderr).trim()
        );
    }

    let (tar, image) = read_bundled_ctx_harness_image(app, arch)?;

    // Check if the image is already present.
    let exists_out = ssh_output(
        &target,
        &format!("{podman_env_prefix} podman image exists -- {}", shell_escape(&image)),
    )?;
    if exists_out.status.success() {
        return Ok(());
    }
    if exists_out.status.code() != Some(1) {
        anyhow::bail!(
            "remote podman image exists failed: {}",
            String::from_utf8_lossy(&exists_out.stderr).trim()
        );
    }

    // Stream tar to podman load over SSH.
    let remote_cmd = format!(
        "sh -lc {}",
        shell_escape(&format!("{podman_prepare_cmd} && {podman_env_prefix} podman load"))
    );
    let mut child = Command::new("ssh")
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-o")
        .arg("ConnectionAttempts=1")
        .arg("-o")
        .arg("ServerAliveInterval=5")
        .arg("-o")
        .arg("ServerAliveCountMax=1")
        .arg(&target)
        .arg(remote_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning ssh for podman load")?;

    {
        let mut file = std::fs::File::open(&tar)
            .with_context(|| format!("opening {}", tar.display()))?;
        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("ssh stdin unavailable"))?;
        std::io::copy(&mut file, &mut stdin).context("streaming image tar to ssh")?;
    }

    let output = child.wait_with_output().context("waiting for ssh podman load")?;
    if !output.status.success() {
        anyhow::bail!(
            "remote podman load failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let exists_after = ssh_output(
        &target,
        &format!("{podman_env_prefix} podman image exists -- {}", shell_escape(&image)),
    )?;
    if !exists_after.status.success() {
        anyhow::bail!(
            "remote podman load completed but image is still missing for daemon storage: {}",
            image
        );
    }

    Ok(())
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

fn probe_local_daemon_health_with_retry(base_url: &str) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..LOCAL_DAEMON_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
        let delay = LOCAL_DAEMON_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
        std::thread::sleep(Duration::from_millis(delay));
    }
    Err(last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed")))
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

fn truncate_tail_chars(value: &str, max_chars: usize) -> String {
    let total = value.chars().count();
    if total <= max_chars {
        return value.to_string();
    }
    let skip = total - max_chars;
    let mut idx = 0;
    let mut seen = 0;
    for (i, _) in value.char_indices() {
        if seen == skip {
            idx = i;
            break;
        }
        seen += 1;
    }
    value[idx..].to_string()
}

fn daemon_stderr_snippet(path: Option<&Path>) -> String {
    let Some(path) = path else {
        return String::new();
    };
    let Ok(bytes) = std::fs::read(path) else {
        return String::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        truncate_tail_chars(trimmed, 1200)
    }
}

fn daemon_data_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
    if let Ok(raw) = std::env::var(DESKTOP_DAEMON_DATA_DIR_ENV) {
        let raw = raw.trim();
        if !raw.is_empty() {
            let p = PathBuf::from(raw);
            if !p.is_absolute() {
                anyhow::bail!("{DESKTOP_DAEMON_DATA_DIR_ENV} must be an absolute path");
            }
            return Ok(p);
        }
    }
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
        let bin_ext = if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        };
        let arch = current_arch_token();
        let prefix = format!("{name}-{arch}");
        for base in [res.join("bin"), res.clone()] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            let mut paths: Vec<PathBuf> =
                entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
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

        // Fall back to generic names only after we try arch-specific candidates.
        candidates.push(res.join("bin").join(format!("{name}{bin_ext}")));
        candidates.push(res.join(format!("{name}{bin_ext}")));
    }

    for c in candidates {
        if c.exists() && path_matches_current_platform_binary(&c) {
            return Some(c);
        }
    }
    None
}

fn dev_bin(name: &str) -> Option<PathBuf> {
    let bin_ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        let candidate = PathBuf::from(target_dir)
            .join("debug")
            .join(format!("{name}{bin_ext}"));
        if candidate.exists() && path_matches_current_platform_binary(&candidate) {
            return Some(candidate);
        }
    }

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())?
        .to_path_buf(); // core/
    let candidate = root
        .join("target")
        .join("debug")
        .join(format!("{name}{bin_ext}"));
    if candidate.exists() && path_matches_current_platform_binary(&candidate) {
        return Some(candidate);
    }
    None
}

fn manifest_bin(name: &str) -> Option<PathBuf> {
    let bin_ext = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bin");
    for candidate in [
        base.join(format!(
            "{name}-{arch}{bin_ext}",
            arch = current_arch_token()
        )),
        base.join(format!("{name}{bin_ext}")),
    ] {
        if candidate.exists() && path_matches_current_platform_binary(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn path_matches_current_platform_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0u8; 4];
    if file.read_exact(&mut header).is_err() {
        return false;
    }

    if cfg!(target_os = "linux") {
        header == [0x7f, b'E', b'L', b'F']
    } else if cfg!(target_os = "windows") {
        header[0..2] == [b'M', b'Z']
    } else if cfg!(target_os = "macos") {
        matches!(
            header,
            // Mach-O 32-bit / 64-bit
            [0xFE, 0xED, 0xFA, 0xCE]
                | [0xCE, 0xFA, 0xED, 0xFE]
                | [0xFE, 0xED, 0xFA, 0xCF]
                | [0xCF, 0xFA, 0xED, 0xFE]
                // Fat (universal) binaries
                | [0xCA, 0xFE, 0xBA, 0xBE]
                | [0xBE, 0xBA, 0xFE, 0xCA]
        )
    } else {
        true
    }
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

fn dev_bundle_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bundles");
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

fn desktop_bundle_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path()
        .resource_dir()
        .ok()
        .map(|p| p.join("bundles"))
        .filter(|p| p.exists())
        .or_else(dev_bundle_dir)
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

fn env_bool(name: &str, default: bool) -> bool {
    let Ok(raw) = std::env::var(name) else {
        return default;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn stop_systemd_scope(_unit: &str) {
    #[cfg(target_os = "linux")]
    {
        let scope = if _unit.ends_with(".scope") {
            _unit.to_string()
        } else {
            format!("{_unit}.scope")
        };
        let _ = Command::new("systemctl")
            .arg("--user")
            .arg("stop")
            .arg(&scope)
            .status();
        let _ = Command::new("systemctl")
            .arg("--user")
            .arg("reset-failed")
            .arg(&scope)
            .status();
    }
}

fn systemd_scope_for_local_daemon_url(base_url: &str) -> Option<String> {
    let url = Url::parse(base_url).ok()?;
    let port = url.port()?;
    Some(format!("ctx-daemon-{port}"))
}

fn spawn_daemon(
    app: &tauri::AppHandle,
    data_dir: &Path,
    wait_for_health: bool,
) -> Result<(String, Child, bool)> {
    let prefer_systemd_scope = should_use_systemd_scope();
    if prefer_systemd_scope {
        match spawn_daemon_with_mode(app, data_dir, true, wait_for_health) {
            Ok(v) => return Ok(v),
            Err(e) => {
                // Fall back to a direct child process when systemd user services are unavailable
                // (common in headless/dev environments).
                return spawn_daemon_with_mode(app, data_dir, false, wait_for_health)
                    .with_context(|| format!("spawning ctx daemon via systemd-run failed: {e:#}"));
            }
        }
    }
    spawn_daemon_with_mode(app, data_dir, false, wait_for_health)
}

fn spawn_daemon_with_mode(
    app: &tauri::AppHandle,
    data_dir: &Path,
    use_systemd_scope: bool,
    wait_for_health: bool,
) -> Result<(String, Child, bool)> {
    let ctx_bin = resource_bin(app, "ctx")
        .or_else(|| manifest_bin("ctx"))
        .or_else(|| dev_bin("ctx"))
        .unwrap_or_else(|| PathBuf::from("ctx"));

    let mcp_bin = resource_bin(app, "ctx-mcp")
        .or_else(|| manifest_bin("ctx-mcp"))
        .or_else(|| dev_bin("ctx-mcp"));

    let web_dist = app
        .path()
        .resource_dir()
        .ok()
        .and_then(|p| {
            let candidates = [
                p.join("web").join("dist"),
                p.join("web-dist"),
                p.join("dist"),
            ];
            candidates.into_iter().find(|c| c.exists())
        })
        .or_else(dev_web_dist);
    let bundle_dir = app
        .path()
        .resource_dir()
        .ok()
        .map(|p| p.join("bundles"))
        .filter(|p| p.exists())
        .or_else(dev_bundle_dir);

    // Container-mode Codex sessions need a CODEX_HOME available inside the Linux harness.
    // The daemon can seed `~/.codex/auth.json` into a daemon-managed location, but only if the
    // host auth file actually exists (otherwise enabling seeding would create hard errors).
    let seed_codex_auth = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("auth.json").exists())
        .unwrap_or(false);

    let local_port = pick_unused_local_port()?;
    let base_url = format!("http://127.0.0.1:{local_port}");
    let systemd_unit = format!("ctx-daemon-{local_port}");

    if use_systemd_scope {
        // Best-effort cleanup in case a prior run left stale units around.
        stop_systemd_scope("ctx-daemon");
        stop_systemd_scope(&systemd_unit);
    }
    let mut cmd = if use_systemd_scope {
        let mut cmd = Command::new("systemd-run");
        cmd.arg("--user")
            .arg("--scope")
            .arg("--unit")
            .arg(&systemd_unit)
            .arg("--same-dir");
        if let Some(dist) = web_dist.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_WEB_DIST={}", dist.to_string_lossy()));
        }
        if let Some(mcp) = mcp_bin.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_MCP_COMMAND={}", mcp.to_string_lossy()));
        }
        if let Some(bundle) = bundle_dir.as_ref() {
            cmd.arg("--setenv")
                .arg(format!("CTX_BUNDLE_DIR={}", bundle.to_string_lossy()));
        }
        if seed_codex_auth {
            cmd.arg("--setenv").arg("CTX_SEED_CODEX_AUTH_FROM_HOST=1");
        }
        if let Ok(appimage) = std::env::var("APPIMAGE") {
            cmd.arg("--setenv")
                .arg(format!("CTX_APPIMAGE_PATH={appimage}"));
        }
        for key in DAEMON_ENV_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                cmd.arg("--setenv").arg(format!("{key}={value}"));
            }
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
        if let Some(bundle) = bundle_dir.as_ref() {
            cmd.env("CTX_BUNDLE_DIR", bundle.to_string_lossy().to_string());
        }
        if seed_codex_auth {
            cmd.env("CTX_SEED_CODEX_AUTH_FROM_HOST", "1");
        }
        if let Ok(appimage) = std::env::var("APPIMAGE") {
            cmd.env("CTX_APPIMAGE_PATH", appimage.clone());
        }
        for key in DAEMON_ENV_PASSTHROUGH {
            if let Ok(value) = std::env::var(key) {
                cmd.env(key, value);
            }
        }
        cmd
    };

    cmd.arg("serve")
        .arg("--bind")
        .arg(format!("127.0.0.1:{local_port}"))
        .arg("--data-dir")
        .arg(data_dir.to_string_lossy().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null());

    let mut stderr_path: Option<PathBuf> = None;
    if use_systemd_scope {
        cmd.stderr(Stdio::inherit());
    } else {
        let log_dir = data_dir.join("logs");
        if let Err(err) = std::fs::create_dir_all(&log_dir) {
            eprintln!(
                "failed to create daemon log dir {}: {err}",
                log_dir.display()
            );
            cmd.stderr(Stdio::inherit());
        } else {
            let path = log_dir.join("desktop-daemon-stderr.log");
            match OpenOptions::new().create(true).append(true).open(&path) {
                Ok(file) => {
                    stderr_path = Some(path);
                    cmd.stderr(file);
                }
                Err(err) => {
                    eprintln!("failed to open daemon stderr log: {err}");
                    cmd.stderr(Stdio::inherit());
                }
            }
        }
    }

    let mut child = cmd.spawn().context("spawning ctx daemon")?;
    if wait_for_health {
        if let Err(err) = probe_local_daemon_health_with_retry(&base_url) {
            if let Ok(Some(status)) = child.try_wait() {
                let stderr = daemon_stderr_snippet(stderr_path.as_deref());
                let mut msg = format!("{err:#}; daemon exited ({status})");
                if !stderr.is_empty() {
                    msg.push_str(&format!("; stderr: {stderr}"));
                }
                return Err(anyhow!(msg));
            }
            let stderr = daemon_stderr_snippet(stderr_path.as_deref());
            if !stderr.is_empty() {
                return Err(anyhow!("{err:#}; stderr: {stderr}"));
            }
            return Err(err).context("waiting for daemon health");
        }
    } else {
        let base_url = base_url.clone();
        let stderr_path = stderr_path.clone();
        std::thread::spawn(move || {
            if let Err(err) = probe_local_daemon_health_with_retry(&base_url) {
                let stderr = daemon_stderr_snippet(stderr_path.as_deref());
                if stderr.is_empty() {
                    eprintln!("ctx daemon health check failed after spawn: {err:#}");
                } else {
                    eprintln!(
                        "ctx daemon health check failed after spawn: {err:#}; stderr: {stderr}"
                    );
                }
            }
        });
    }
    Ok((base_url, child, use_systemd_scope))
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
