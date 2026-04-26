use super::*;

pub(crate) fn handle_open(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    tokens: &DeepLinkTokenStore,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkOpen,
) -> Result<()> {
    if matches!(req.target, DeepLinkTarget::WorktreeFile { .. })
        && matches!(state.info().kind, DesktopConnectionKind::None)
    {
        ensure_local_connection_for_user_action(app, state)?;
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

pub(crate) fn handle_reveal(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    tokens: &DeepLinkTokenStore,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkReveal,
) -> Result<()> {
    if matches!(req.target, DeepLinkTarget::WorktreeFile { .. })
        && matches!(state.info().kind, DesktopConnectionKind::None)
    {
        ensure_local_connection_for_user_action(app, state)?;
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

pub(crate) fn handle_workspace(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkWorkspace,
) -> Result<()> {
    if matches!(state.info().kind, DesktopConnectionKind::None) {
        ensure_local_connection_for_user_action(app, state)?;
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

pub(crate) fn handle_task(
    app: &tauri::AppHandle,
    state: &ConnectionManager,
    registry: &WorkspaceWindowRegistry,
    req: DeepLinkTask,
) -> Result<()> {
    if matches!(state.info().kind, DesktopConnectionKind::None) {
        ensure_local_connection(app, state)?;
    }

    focus_or_open_workspace_target(
        app,
        registry,
        &req.workspace_id,
        Some(&req.task_id),
        req.session_id.as_deref(),
    )
}

pub(crate) fn open_in_ctx(
    app: &tauri::AppHandle,
    target: &DeepLinkTarget,
    line: Option<u32>,
    col: Option<u32>,
) -> Result<()> {
    let url = build_file_preview_url(target, line, col);
    let label = format!("file:{}", uuid::Uuid::new_v4());
    open_file_preview_window_with_label(app, &label, &url)
}

pub(crate) fn open_file_preview_window_with_label(
    app: &tauri::AppHandle,
    label: &str,
    route: &str,
) -> Result<()> {
    let init_script = desktop_startup_initialization_script(label, route);
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(route.into()))
        .title("ctx")
        .inner_size(1000.0, 780.0)
        .initialization_script(&init_script)
        .build()
        .context("creating file preview window")?;
    log_window_created(label, route);
    log_navigation_start(label, route, "file_preview_window");
    register_window_for_recovery(app, label, route);
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

pub(crate) fn resolve_editor_target(
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

pub(crate) fn offer_editor_settings(app: &tauri::AppHandle) {
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

pub(crate) fn open_settings_window(app: &tauri::AppHandle) -> Result<()> {
    open_settings_window_at_route(app, "settings", "/settings")
}

pub(crate) fn open_settings_window_at_route(
    app: &tauri::AppHandle,
    label: &str,
    route: &str,
) -> Result<()> {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
        record_window_route(app, label, route);
        return Ok(());
    }
    let init_script = desktop_startup_initialization_script(label, route);
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(route.into()))
        .title("Settings")
        .inner_size(1000.0, 780.0)
        .initialization_script(&init_script)
        .build()
        .context("creating settings window")?;
    log_window_created(label, route);
    log_navigation_start(label, route, "settings_window");
    register_window_for_recovery(app, label, route);
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
