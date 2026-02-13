use super::*;

#[derive(Debug)]
pub(super) enum DeepLinkAction {
    Open(DeepLinkOpen),
    Reveal(DeepLinkReveal),
    Workspace(DeepLinkWorkspace),
    Focus,
}

#[derive(Debug)]
pub(super) struct DeepLinkOpen {
    target: DeepLinkTarget,
    line: Option<u32>,
    col: Option<u32>,
    open_with: DeepLinkOpenWith,
    editor_override: Option<DesktopEditorTarget>,
    token: Option<String>,
}

#[derive(Debug)]
pub(super) struct DeepLinkReveal {
    target: DeepLinkTarget,
    token: Option<String>,
}

#[derive(Debug)]
pub(super) struct DeepLinkWorkspace {
    workspace_id: Option<String>,
    path: Option<String>,
}

#[derive(Debug)]
pub(super) enum DeepLinkTarget {
    WorktreeFile { worktree_id: String, file: String },
    Path { path: String },
}

#[derive(Debug)]
pub(super) struct WorktreeInfo {
    root: PathBuf,
    workspace_id: String,
}

pub(super) fn setup_deep_link_listener(app: &tauri::AppHandle) {
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

pub(super) fn handle_deep_link(app: tauri::AppHandle, url: Url) {
    std::thread::spawn(move || {
        if let Err(err) = handle_deep_link_inner(&app, &url) {
            show_error_dialog(&app, &format!("Deep link failed: {err:#}"));
        }
    });
}

pub(super) fn handle_deep_link_inner(app: &tauri::AppHandle, url: &Url) -> Result<()> {
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

pub(super) fn parse_deep_link(url: &Url) -> Result<DeepLinkAction> {
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

pub(super) fn parse_open(params: &HashMap<String, String>) -> Result<DeepLinkOpen> {
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

pub(super) fn parse_reveal(params: &HashMap<String, String>) -> Result<DeepLinkReveal> {
    let target = parse_target(params)?;
    let token = params.get("token").cloned();
    Ok(DeepLinkReveal { target, token })
}

pub(super) fn parse_workspace(params: &HashMap<String, String>) -> Result<DeepLinkWorkspace> {
    let workspace_id = params.get("workspaceId").cloned();
    let path = params.get("path").cloned();
    if workspace_id.is_none() && path.is_none() {
        anyhow::bail!("workspaceId or path is required");
    }
    Ok(DeepLinkWorkspace { workspace_id, path })
}

pub(super) fn parse_target(params: &HashMap<String, String>) -> Result<DeepLinkTarget> {
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

pub(super) fn parse_open_with(value: Option<&String>) -> Result<DeepLinkOpenWith> {
    match value.map(|v| v.trim().to_lowercase()) {
        None => Ok(DeepLinkOpenWith::Ctx),
        Some(v) if v == "ctx" => Ok(DeepLinkOpenWith::Ctx),
        Some(v) if v == "editor" => Ok(DeepLinkOpenWith::Editor),
        Some(v) if v == "system" => Ok(DeepLinkOpenWith::System),
        Some(v) => anyhow::bail!("unsupported openWith: {v}"),
    }
}

pub(super) fn parse_editor_target(value: &str) -> Result<DesktopEditorTarget> {
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

pub(super) fn parse_optional_positive(value: Option<&String>) -> Option<u32> {
    value.and_then(|v| v.parse::<u32>().ok()).filter(|v| *v > 0)
}

pub(super) fn validate_relative_file(path: &str) -> Result<String> {
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

pub(super) fn validate_absolute_path(path: &str) -> Result<String> {
    if path.trim().is_empty() {
        anyhow::bail!("path is empty");
    }
    let candidate = Path::new(path);
    if !candidate.is_absolute() {
        anyhow::bail!("path must be absolute");
    }
    Ok(path.to_string())
}

pub(super) fn handle_open(
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

pub(super) fn handle_reveal(
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

pub(super) fn handle_workspace(
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

pub(super) fn open_in_ctx(
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

pub(super) fn build_file_preview_url(target: &DeepLinkTarget, line: Option<u32>, col: Option<u32>) -> String {
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

pub(super) fn resolve_editor_target(
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

pub(super) fn offer_editor_settings(app: &tauri::AppHandle) {
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

pub(super) fn open_settings_window(app: &tauri::AppHandle) -> Result<()> {
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

pub(super) fn open_in_editor_with_target(
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

pub(super) fn resolve_target_path(state: &ConnectionManager, target: &DeepLinkTarget) -> Result<PathBuf> {
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

pub(super) fn is_target_in_open_workspace(
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

pub(super) fn resolve_or_create_workspace_id(state: &ConnectionManager, root_path: &str) -> Result<String> {
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
    value["id"]
        .as_str()
        .map(|id| id.to_string())
        .ok_or_else(|| anyhow!("workspace id missing"))
}

pub(super) fn resolve_workspace_id_by_path(
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
            if let Some(id) = entry.get("id").and_then(|v| v.as_str()) {
                return Ok(Some(id.to_string()));
            }
        }
    }
    Ok(None)
}

pub(super) fn resolve_workspace_root(state: &ConnectionManager, workspace_id: &str) -> Result<PathBuf> {
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

pub(super) fn resolve_worktree_info(state: &ConnectionManager, worktree_id: &str) -> Result<WorktreeInfo> {
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
        .and_then(|v| v.as_str())
        .map(|id| id.to_string())
        .ok_or_else(|| anyhow!("worktree workspace_id missing"))?;
    Ok(WorktreeInfo {
        root: PathBuf::from(root),
        workspace_id,
    })
}

