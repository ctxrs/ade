use super::*;
pub(super) use ctx_desktop_ipc::{
    DesktopEditorSettings, DesktopEditorTarget, DesktopGitCloneReq, DesktopOpenFileReq,
    DesktopOpenPathReq, DesktopReadBinaryFileResp, DesktopReadFileResp, DesktopSaveTextFileReq,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct DesktopSettings {
    #[serde(default)]
    pub(super) editor: DesktopEditorSettings,
}

#[tauri::command]
pub(super) async fn desktop_pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
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
pub(super) async fn desktop_save_text_file(
    app: tauri::AppHandle,
    req: DesktopSaveTextFileReq,
) -> Result<Option<String>, String> {
    let suggested = req
        .suggested_name
        .unwrap_or_else(|| "conversation.md".to_string());
    let suggested = suggested.trim().to_string();
    let contents = req.contents;

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
pub(super) fn desktop_get_editor_settings(
    app: tauri::AppHandle,
) -> Result<DesktopEditorSettings, String> {
    Ok(load_desktop_settings(&app).editor)
}

#[tauri::command]
pub(super) fn desktop_update_editor_settings(
    app: tauri::AppHandle,
    req: DesktopEditorSettings,
) -> Result<DesktopEditorSettings, String> {
    let mut current = load_desktop_settings(&app);
    current.editor = req;
    save_desktop_settings(&app, &current).map_err(to_err)?;
    Ok(current.editor)
}

#[tauri::command]
pub(super) fn desktop_open_file(
    state: tauri::State<ConnectionManager>,
    app: tauri::AppHandle,
    window: tauri::Window,
    req: DesktopOpenFileReq,
) -> Result<(), String> {
    let scope = window.label().to_string();
    let worktree_id = req.worktree_id.trim();
    if worktree_id.is_empty() {
        return Err("worktree_id is required".to_string());
    }
    let path = req.path.trim();
    if path.is_empty() {
        return Err("path is required".to_string());
    }

    let worktree_root = resolve_worktree_root(&state, &scope, worktree_id).map_err(to_err)?;
    let resolved = resolve_worktree_path(&worktree_root, path).map_err(to_err)?;
    let line = req.line.filter(|v| *v > 0);
    let col = req.col.filter(|v| *v > 0);
    let editor_settings = load_desktop_settings(&app).editor;
    open_in_editor(
        &editor_settings,
        &resolved,
        line,
        col,
        state.is_remote_for_scope(&scope),
    )
    .map_err(to_err)?;
    Ok(())
}

#[tauri::command]
pub(super) fn desktop_open_path(
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
pub(super) fn desktop_read_file(req: DesktopOpenPathReq) -> Result<DesktopReadFileResp, String> {
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
pub(super) fn desktop_read_binary_file(
    req: DesktopOpenPathReq,
) -> Result<DesktopReadBinaryFileResp, String> {
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
    let bytes = std::fs::read(&resolved).map_err(|e| format!("failed to read file: {e}"))?;
    Ok(DesktopReadBinaryFileResp {
        path: resolved.to_string_lossy().to_string(),
        bytes,
    })
}

#[tauri::command]
pub(super) async fn desktop_git_clone(req: DesktopGitCloneReq) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let repo_url = req.repo_url.trim().to_string();
        if repo_url.is_empty() {
            return Err("repo_url is required".to_string());
        }
        let dest_parent = PathBuf::from(req.dest_parent);
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

fn desktop_settings_path(_app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = desktop_local_data_root()?;
    Ok(ctx_fs::paths::ui_root(root).join("desktop-settings.json"))
}

pub(super) fn load_desktop_settings(app: &tauri::AppHandle) -> DesktopSettings {
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

fn resolve_worktree_root(
    state: &ConnectionManager,
    scope: &str,
    worktree_id: &str,
) -> Result<PathBuf> {
    let resp = state.daemon_request_for_scope(
        scope,
        DesktopDaemonRequest {
            method: "GET".to_string(),
            path: format!("/api/worktrees/{worktree_id}"),
            body: None,
            headers: vec![],
        },
    )?;
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

pub(super) fn resolve_worktree_path(worktree_root: &Path, raw_path: &str) -> Result<PathBuf> {
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

pub(super) fn open_in_editor(
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

pub(super) fn open_with_system(target: &str) -> Result<()> {
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
