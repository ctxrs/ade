use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{
    CheckMenuItemBuilder, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, MenuItemKind,
    PredefinedMenuItem, Submenu, SubmenuBuilder,
};
use tauri::{Emitter, Manager};

const MENU_EVENT_NAME: &str = "desktop_menu_action";

pub(super) const CMD_FILE_NEW_WORKSPACE: &str = "file.new-workspace";
pub(super) const CMD_FILE_OPEN_WORKSPACES: &str = "file.open-workspaces";
pub(super) const CMD_FILE_OPEN_RECENT: &str = "file.open-recent";
pub(super) const CMD_FILE_OPEN_WORKSPACE_NEW_WINDOW: &str = "file.open-workspace-new-window";
pub(super) const CMD_FILE_EXPORT_TRANSCRIPT: &str = "file.export-transcript";
pub(super) const CMD_FILE_EXPORT_SESSION_LOG: &str = "file.export-session-log";
pub(super) const CMD_VIEW_FIND_TASKS: &str = "view.find-tasks";
pub(super) const CMD_VIEW_TOGGLE_SIDEBAR: &str = "view.toggle-sidebar";
pub(super) const CMD_VIEW_TOGGLE_DIFF: &str = "view.toggle-diff";
pub(super) const CMD_VIEW_TOGGLE_ARTIFACTS: &str = "view.toggle-artifacts";
pub(super) const CMD_VIEW_TOGGLE_SESSIONS: &str = "view.toggle-sessions";
pub(super) const CMD_VIEW_TOGGLE_TERMINAL: &str = "view.toggle-terminal";
pub(super) const CMD_TASK_NEW: &str = "task.new";
pub(super) const CMD_TASK_RENAME: &str = "task.rename";
pub(super) const CMD_TASK_ARCHIVE_TOGGLE: &str = "task.archive-toggle";
pub(super) const CMD_TASK_MARK_READ_TOGGLE: &str = "task.mark-read-toggle";
pub(super) const CMD_TASK_DELETE: &str = "task.delete";
pub(super) const CMD_SESSION_COPY_TRANSCRIPT: &str = "session.copy-transcript";
pub(super) const CMD_SESSION_COPY_SESSION_LOG: &str = "session.copy-session-log";
pub(super) const CMD_SESSION_COPY_WORKTREE_LOCATION: &str = "session.copy-worktree-location";
pub(super) const CMD_SESSION_OPEN_WORKTREE_TERMINAL: &str = "session.open-worktree-terminal";
pub(super) const CMD_SESSION_INTERRUPT: &str = "session.interrupt";
pub(super) const CMD_GO_LAUNCHER: &str = "go.launcher";
pub(super) const CMD_GO_WORKSPACE_SETUP: &str = "go.workspace-setup";
pub(super) const CMD_GO_WORKSPACES: &str = "go.workspaces";
pub(super) const CMD_GO_SETTINGS: &str = "go.settings";
pub(super) const CMD_GO_DIAGNOSTICS: &str = "go.diagnostics";
pub(super) const CMD_GO_AGENT_HARNESSES: &str = "go.agent-harnesses";
pub(super) const CMD_HELP_CRASH_COURSE: &str = "help.crash-course";
pub(super) const CMD_HELP_KEYBOARD_SHORTCUTS: &str = "help.keyboard-shortcuts";
pub(super) const CMD_HELP_OPEN_LOGS_FOLDER: &str = "help.open-logs-folder";
pub(super) const CMD_HELP_REPORT_ISSUE: &str = "help.report-issue";
pub(super) const CMD_HELP_DIAGNOSTICS: &str = "help.diagnostics";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopMenuActionEvent {
    command_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopMenuItemStateUpdate {
    pub id: String,
    pub enabled: Option<bool>,
    pub checked: Option<bool>,
}

#[derive(Default)]
pub(super) struct DesktopMenuStateCache {
    by_window: Mutex<HashMap<String, Vec<DesktopMenuItemStateUpdate>>>,
}

impl DesktopMenuStateCache {
    fn set_state(&self, window_label: &str, items: Vec<DesktopMenuItemStateUpdate>) {
        let mut guard = self
            .by_window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.insert(window_label.to_string(), items);
    }

    fn get_state(&self, window_label: &str) -> Option<Vec<DesktopMenuItemStateUpdate>> {
        let guard = self
            .by_window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.get(window_label).cloned()
    }

    fn remove_state(&self, window_label: &str) {
        let mut guard = self
            .by_window
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(window_label);
    }
}

fn menu_item(
    app: &tauri::AppHandle,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
    enabled: bool,
) -> tauri::Result<tauri::menu::MenuItem<tauri::Wry>> {
    let mut builder = MenuItemBuilder::with_id(id, text).enabled(enabled);
    if let Some(accel) = accelerator {
        builder = builder.accelerator(accel);
    }
    builder.build(app)
}

fn check_item(
    app: &tauri::AppHandle,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
    enabled: bool,
    checked: bool,
) -> tauri::Result<tauri::menu::CheckMenuItem<tauri::Wry>> {
    let mut builder = CheckMenuItemBuilder::with_id(id, text)
        .enabled(enabled)
        .checked(checked);
    if let Some(accel) = accelerator {
        builder = builder.accelerator(accel);
    }
    builder.build(app)
}

fn build_file_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let new_workspace = menu_item(
        app,
        CMD_FILE_NEW_WORKSPACE,
        "New Workspace...",
        Some("CmdOrCtrl+Shift+N"),
        true,
    )?;
    let open_workspaces = menu_item(app, CMD_FILE_OPEN_WORKSPACES, "Open Workspaces", None, true)?;
    let open_recent = menu_item(app, CMD_FILE_OPEN_RECENT, "Open Recent", None, false)?;
    let open_new_window = menu_item(
        app,
        CMD_FILE_OPEN_WORKSPACE_NEW_WINDOW,
        "Open Workspace in New Window",
        Some("CmdOrCtrl+Shift+O"),
        false,
    )?;
    let export_transcript = menu_item(
        app,
        CMD_FILE_EXPORT_TRANSCRIPT,
        "Export Transcript",
        Some("CmdOrCtrl+Shift+E"),
        false,
    )?;
    let export_session_log = menu_item(
        app,
        CMD_FILE_EXPORT_SESSION_LOG,
        "Export Session Log",
        Some("CmdOrCtrl+Alt+E"),
        false,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let close_window = PredefinedMenuItem::close_window(app, None)?;

    SubmenuBuilder::new(app, "File")
        .item(&new_workspace)
        .item(&open_workspaces)
        .item(&open_recent)
        .item(&open_new_window)
        .item(&sep1)
        .item(&export_transcript)
        .item(&export_session_log)
        .item(&sep2)
        .item(&close_window)
        .build()
}

fn build_edit_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let undo = PredefinedMenuItem::undo(app, None)?;
    let redo = PredefinedMenuItem::redo(app, None)?;
    let cut = PredefinedMenuItem::cut(app, None)?;
    let copy = PredefinedMenuItem::copy(app, None)?;
    let paste = PredefinedMenuItem::paste(app, None)?;
    let select_all = PredefinedMenuItem::select_all(app, None)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let find_tasks = menu_item(app, CMD_VIEW_FIND_TASKS, "Find Tasks", Some("CmdOrCtrl+F"), false)?;

    SubmenuBuilder::new(app, "Edit")
        .item(&undo)
        .item(&redo)
        .item(&sep1)
        .item(&cut)
        .item(&copy)
        .item(&paste)
        .item(&select_all)
        .item(&sep2)
        .item(&find_tasks)
        .build()
}

fn build_view_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let toggle_sidebar = check_item(
        app,
        CMD_VIEW_TOGGLE_SIDEBAR,
        "Toggle Sidebar",
        Some("CmdOrCtrl+B"),
        false,
        false,
    )?;
    let toggle_diff = check_item(app, CMD_VIEW_TOGGLE_DIFF, "Toggle Diff Pane", None, false, false)?;
    let toggle_artifacts = check_item(
        app,
        CMD_VIEW_TOGGLE_ARTIFACTS,
        "Toggle Artifacts Pane",
        None,
        false,
        false,
    )?;
    let toggle_sessions = check_item(
        app,
        CMD_VIEW_TOGGLE_SESSIONS,
        "Toggle Sessions Pane",
        None,
        false,
        false,
    )?;
    let toggle_terminal = check_item(
        app,
        CMD_VIEW_TOGGLE_TERMINAL,
        "Toggle Terminal",
        Some("Ctrl+`"),
        false,
        false,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let fullscreen = PredefinedMenuItem::fullscreen(app, None)?;

    SubmenuBuilder::new(app, "View")
        .item(&toggle_sidebar)
        .item(&toggle_diff)
        .item(&toggle_artifacts)
        .item(&toggle_sessions)
        .item(&toggle_terminal)
        .item(&sep)
        .item(&fullscreen)
        .build()
}

fn build_task_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let new_task = menu_item(app, CMD_TASK_NEW, "New Task", Some("CmdOrCtrl+N"), false)?;
    let rename_task = menu_item(app, CMD_TASK_RENAME, "Rename Task", Some("F2"), false)?;
    let archive_toggle = menu_item(app, CMD_TASK_ARCHIVE_TOGGLE, "Archive/Unarchive", None, false)?;
    let mark_read_toggle = menu_item(app, CMD_TASK_MARK_READ_TOGGLE, "Mark Read/Unread", None, false)?;
    let delete_task = menu_item(app, CMD_TASK_DELETE, "Delete Task", Some("CmdOrCtrl+Backspace"), false)?;

    SubmenuBuilder::new(app, "Task")
        .item(&new_task)
        .item(&rename_task)
        .item(&archive_toggle)
        .item(&mark_read_toggle)
        .item(&delete_task)
        .build()
}

fn build_session_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let copy_transcript = menu_item(
        app,
        CMD_SESSION_COPY_TRANSCRIPT,
        "Copy Transcript",
        Some("CmdOrCtrl+Shift+C"),
        false,
    )?;
    let copy_session_log = menu_item(
        app,
        CMD_SESSION_COPY_SESSION_LOG,
        "Copy Session Log",
        Some("CmdOrCtrl+Alt+C"),
        false,
    )?;
    let copy_worktree = menu_item(
        app,
        CMD_SESSION_COPY_WORKTREE_LOCATION,
        "Copy Worktree Location",
        None,
        false,
    )?;
    let open_terminal = menu_item(
        app,
        CMD_SESSION_OPEN_WORKTREE_TERMINAL,
        "Open Worktree Terminal",
        None,
        false,
    )?;
    let interrupt = menu_item(
        app,
        CMD_SESSION_INTERRUPT,
        "Interrupt Run",
        Some("CmdOrCtrl+."),
        false,
    )?;

    SubmenuBuilder::new(app, "Session")
        .item(&copy_transcript)
        .item(&copy_session_log)
        .item(&copy_worktree)
        .item(&open_terminal)
        .item(&interrupt)
        .build()
}

fn build_go_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let launcher = menu_item(app, CMD_GO_LAUNCHER, "Launcher", Some("CmdOrCtrl+1"), true)?;
    let workspace_setup = menu_item(
        app,
        CMD_GO_WORKSPACE_SETUP,
        "Workspace Setup",
        Some("CmdOrCtrl+Shift+S"),
        true,
    )?;
    let workspaces = menu_item(app, CMD_GO_WORKSPACES, "Workspaces", Some("CmdOrCtrl+2"), true)?;
    let settings = menu_item(app, CMD_GO_SETTINGS, "Settings", Some("CmdOrCtrl+,"), true)?;
    let diagnostics = menu_item(app, CMD_GO_DIAGNOSTICS, "Diagnostics", Some("CmdOrCtrl+3"), true)?;
    let harnesses = menu_item(
        app,
        CMD_GO_AGENT_HARNESSES,
        "Agent Harnesses",
        Some("CmdOrCtrl+Shift+A"),
        true,
    )?;

    SubmenuBuilder::new(app, "Go")
        .item(&launcher)
        .item(&workspace_setup)
        .item(&workspaces)
        .item(&settings)
        .item(&diagnostics)
        .item(&harnesses)
        .build()
}

fn build_window_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let minimize = PredefinedMenuItem::minimize(app, None)?;
    let maximize = PredefinedMenuItem::maximize(app, None)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let close_window = PredefinedMenuItem::close_window(app, None)?;

    #[cfg(target_os = "macos")]
    {
        let show_all = PredefinedMenuItem::show_all(app, None)?;
        return SubmenuBuilder::new(app, "Window")
            .item(&minimize)
            .item(&maximize)
            .item(&sep)
            .item(&close_window)
            .item(&show_all)
            .build();
    }

    #[cfg(not(target_os = "macos"))]
    {
        return SubmenuBuilder::new(app, "Window")
            .item(&minimize)
            .item(&maximize)
            .item(&sep)
            .item(&close_window)
            .build();
    }
}

fn build_help_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let crash_course = menu_item(app, CMD_HELP_CRASH_COURSE, "Crash Course", None, true)?;
    let keyboard_shortcuts = menu_item(
        app,
        CMD_HELP_KEYBOARD_SHORTCUTS,
        "Keyboard Shortcuts",
        Some("CmdOrCtrl+/"),
        true,
    )?;
    let open_logs_folder = menu_item(app, CMD_HELP_OPEN_LOGS_FOLDER, "Open Logs Folder", None, true)?;
    let diagnostics = menu_item(app, CMD_HELP_DIAGNOSTICS, "Diagnostics", None, true)?;
    let report_issue = menu_item(app, CMD_HELP_REPORT_ISSUE, "Report Issue", None, true)?;

    SubmenuBuilder::new(app, "Help")
        .item(&crash_course)
        .item(&keyboard_shortcuts)
        .item(&open_logs_folder)
        .item(&diagnostics)
        .item(&report_issue)
        .build()
}

#[cfg(target_os = "macos")]
fn build_app_submenu(app: &tauri::AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let about = PredefinedMenuItem::about(app, None, None)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let sep3 = PredefinedMenuItem::separator(app)?;
    let settings = menu_item(app, CMD_GO_SETTINGS, "Settings...", Some("CmdOrCtrl+,"), true)?;
    let hide = PredefinedMenuItem::hide(app, None)?;
    let hide_others = PredefinedMenuItem::hide_others(app, None)?;
    let show_all = PredefinedMenuItem::show_all(app, None)?;
    let quit = PredefinedMenuItem::quit(app, None)?;

    SubmenuBuilder::new(app, "ctx")
        .item(&about)
        .item(&sep1)
        .item(&settings)
        .item(&sep2)
        .item(&hide)
        .item(&hide_others)
        .item(&show_all)
        .item(&sep3)
        .item(&quit)
        .build()
}

pub(super) fn build_app_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let file = build_file_submenu(app)?;
    let edit = build_edit_submenu(app)?;
    let view = build_view_submenu(app)?;
    let task = build_task_submenu(app)?;
    let session = build_session_submenu(app)?;
    let go = build_go_submenu(app)?;
    let window = build_window_submenu(app)?;
    let help = build_help_submenu(app)?;

    let mut builder = MenuBuilder::new(app);

    #[cfg(target_os = "macos")]
    {
        let app_submenu = build_app_submenu(app)?;
        builder = builder.item(&app_submenu);
    }

    builder
        .item(&file)
        .item(&edit)
        .item(&view)
        .item(&task)
        .item(&session)
        .item(&go)
        .item(&window)
        .item(&help)
        .build()
}

fn is_menu_command_id(id: &str) -> bool {
    matches!(
        id,
        CMD_FILE_NEW_WORKSPACE
            | CMD_FILE_OPEN_WORKSPACES
            | CMD_FILE_OPEN_RECENT
            | CMD_FILE_OPEN_WORKSPACE_NEW_WINDOW
            | CMD_FILE_EXPORT_TRANSCRIPT
            | CMD_FILE_EXPORT_SESSION_LOG
            | CMD_VIEW_FIND_TASKS
            | CMD_VIEW_TOGGLE_SIDEBAR
            | CMD_VIEW_TOGGLE_DIFF
            | CMD_VIEW_TOGGLE_ARTIFACTS
            | CMD_VIEW_TOGGLE_SESSIONS
            | CMD_VIEW_TOGGLE_TERMINAL
            | CMD_TASK_NEW
            | CMD_TASK_RENAME
            | CMD_TASK_ARCHIVE_TOGGLE
            | CMD_TASK_MARK_READ_TOGGLE
            | CMD_TASK_DELETE
            | CMD_SESSION_COPY_TRANSCRIPT
            | CMD_SESSION_COPY_SESSION_LOG
            | CMD_SESSION_COPY_WORKTREE_LOCATION
            | CMD_SESSION_OPEN_WORKTREE_TERMINAL
            | CMD_SESSION_INTERRUPT
            | CMD_GO_LAUNCHER
            | CMD_GO_WORKSPACE_SETUP
            | CMD_GO_WORKSPACES
            | CMD_GO_SETTINGS
            | CMD_GO_DIAGNOSTICS
            | CMD_GO_AGENT_HARNESSES
            | CMD_HELP_CRASH_COURSE
            | CMD_HELP_KEYBOARD_SHORTCUTS
            | CMD_HELP_OPEN_LOGS_FOLDER
            | CMD_HELP_REPORT_ISSUE
            | CMD_HELP_DIAGNOSTICS
    )
}

fn focused_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    for window in app.webview_windows().values() {
        if window.is_focused().unwrap_or(false) {
            return Some(window.clone());
        }
    }

    app.get_webview_window("main")
        .or_else(|| app.webview_windows().values().next().cloned())
}

fn emit_menu_action(app: &tauri::AppHandle, command_id: &str) {
    let payload = DesktopMenuActionEvent {
        command_id: command_id.to_string(),
    };
    if let Some(window) = focused_window(app) {
        let _ = window.emit(MENU_EVENT_NAME, payload);
        return;
    }
    let _ = app.emit(MENU_EVENT_NAME, payload);
}

pub(super) fn handle_app_menu_event(app: &tauri::AppHandle, event: MenuEvent) {
    let id = event.id().as_ref();
    if !is_menu_command_id(id) {
        return;
    }
    emit_menu_action(app, id);
}

fn set_enabled_for_kind(kind: &MenuItemKind<tauri::Wry>, enabled: bool) -> Result<(), String> {
    match kind {
        MenuItemKind::MenuItem(item) => item.set_enabled(enabled).map_err(|e| e.to_string()),
        MenuItemKind::Submenu(item) => item.set_enabled(enabled).map_err(|e| e.to_string()),
        MenuItemKind::Check(item) => item.set_enabled(enabled).map_err(|e| e.to_string()),
        MenuItemKind::Icon(item) => item.set_enabled(enabled).map_err(|e| e.to_string()),
        MenuItemKind::Predefined(_) => Ok(()),
    }
}

fn apply_state_to_kind(kind: &MenuItemKind<tauri::Wry>, update: &DesktopMenuItemStateUpdate) -> Result<(), String> {
    if let Some(enabled) = update.enabled {
        set_enabled_for_kind(kind, enabled)?;
    }
    if let Some(checked) = update.checked {
        if let Some(item) = kind.as_check_menuitem() {
            item.set_checked(checked).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn apply_state_to_submenu(
    submenu: &Submenu<tauri::Wry>,
    update: &DesktopMenuItemStateUpdate,
) -> Result<bool, String> {
    for item in submenu.items().map_err(|e| e.to_string())? {
        if item.id() == &update.id.as_str() {
            apply_state_to_kind(&item, update)?;
            return Ok(true);
        }
        if let Some(child_submenu) = item.as_submenu() {
            if apply_state_to_submenu(child_submenu, update)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn apply_state_to_menu(
    menu: &Menu<tauri::Wry>,
    update: &DesktopMenuItemStateUpdate,
) -> Result<bool, String> {
    for item in menu.items().map_err(|e| e.to_string())? {
        if item.id() == &update.id.as_str() {
            apply_state_to_kind(&item, update)?;
            return Ok(true);
        }
        if let Some(submenu) = item.as_submenu() {
            if apply_state_to_submenu(submenu, update)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn apply_menu_state_updates(
    app: &tauri::AppHandle,
    items: &[DesktopMenuItemStateUpdate],
) -> Result<(), String> {
    let Some(menu) = app.menu() else {
        return Ok(());
    };
    for update in items {
        let found = apply_state_to_menu(&menu, update)?;
        if !found {
            eprintln!(
                "desktop menu state update ignored: unknown menu id '{}'",
                update.id
            );
        }
    }
    Ok(())
}

pub(super) fn apply_cached_menu_state_for_window(
    app: &tauri::AppHandle,
    window_label: &str,
) -> Result<(), String> {
    let cache = app.state::<DesktopMenuStateCache>();
    let Some(items) = cache.get_state(window_label) else {
        return Ok(());
    };
    apply_menu_state_updates(app, &items)
}

pub(super) fn clear_cached_menu_state_for_window(app: &tauri::AppHandle, window_label: &str) {
    let cache = app.state::<DesktopMenuStateCache>();
    cache.remove_state(window_label);
}

#[tauri::command]
pub(super) fn desktop_set_menu_state(
    app: tauri::AppHandle,
    webview_window: tauri::WebviewWindow,
    cache: tauri::State<'_, DesktopMenuStateCache>,
    items: Vec<DesktopMenuItemStateUpdate>,
) -> Result<(), String> {
    let window_label = webview_window.label().to_string();
    cache.set_state(&window_label, items.clone());

    if webview_window
        .is_focused()
        .map_err(|err| err.to_string())?
    {
        apply_menu_state_updates(&app, &items)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_menu_state_cache_is_window_scoped() {
        let cache = DesktopMenuStateCache::default();
        cache.set_state(
            "window-a",
            vec![DesktopMenuItemStateUpdate {
                id: "task.new".to_string(),
                enabled: Some(true),
                checked: Some(false),
            }],
        );
        cache.set_state(
            "window-b",
            vec![DesktopMenuItemStateUpdate {
                id: "task.new".to_string(),
                enabled: Some(false),
                checked: Some(false),
            }],
        );

        let a = cache
            .get_state("window-a")
            .expect("expected state for window-a");
        let b = cache
            .get_state("window-b")
            .expect("expected state for window-b");

        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].enabled, Some(true));
        assert_eq!(b[0].enabled, Some(false));

        cache.remove_state("window-a");
        assert!(cache.get_state("window-a").is_none());
        assert!(cache.get_state("window-b").is_some());
    }
}
