use std::collections::{HashMap, HashSet};
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "macos")]
use std::sync::{Once, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use ctx_desktop_ipc::{
    DesktopDeepLinkToken, DesktopDockRecentLocalWorkspace as DockRecentLocalWorkspaceEntry,
    DesktopOpenWorkspaceInNewWindowReq, DesktopRecordWorkspaceVisitReq,
    DesktopSetDockRecentLocalWorkspacesReq, DesktopSetOpenWorkspacesReq, DesktopSetWindowTitleReq,
    DesktopTitlebarColor,
};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, NSObject, Sel};
#[cfg(target_os = "macos")]
use objc2::{msg_send, sel, AnyThread, ClassType, MainThreadMarker, MainThreadOnly};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSButton, NSColor, NSImage, NSImageNamePreferencesGeneral, NSLayoutAttribute,
    NSTitlebarAccessoryViewController, NSTitlebarSeparatorStyle, NSView, NSWindow,
    NSWindowTitleVisibility,
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
use tauri_plugin_automation::init as automation_init;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::OnceCell;
use url::Url;

mod desktop_attention;
mod desktop_connection;
mod desktop_daemon;
mod desktop_deeplink;
mod desktop_dock_menu;
mod desktop_editor;
mod desktop_env;
mod desktop_local_daemon;
mod desktop_logs;
mod desktop_menu;
mod desktop_notifications;
mod desktop_paths;
mod desktop_runtime;
mod desktop_shared;
mod desktop_ssh;
mod desktop_storage;
mod desktop_updater;
mod desktop_webview_recovery;
mod desktop_windows;
mod linux_sandbox;
use desktop_attention::*;
use desktop_connection::*;
use desktop_daemon::*;
use desktop_deeplink::*;
use desktop_dock_menu::*;
use desktop_editor::*;
use desktop_env::*;
use desktop_local_daemon::*;
use desktop_logs::*;
use desktop_menu::*;
use desktop_notifications::*;
use desktop_paths::*;
use desktop_runtime::*;
use desktop_shared::*;
use desktop_ssh::*;
use desktop_storage::*;
use desktop_updater::*;
use desktop_webview_recovery::*;
use desktop_windows::*;
use linux_sandbox::*;

fn main() {
    #[cfg(all(target_os = "windows", feature = "stt"))]
    {
        configure_windows_vosk_dll_search_path();
    }

    let mut builder = tauri::Builder::default()
        .manage(ConnectionManager::default())
        .manage(DeepLinkTokenStore::default())
        .manage(WorkspaceWindowRegistry::default())
        .manage(DesktopAttentionRegistry::default())
        .manage(DesktopNotificationAutomationState::default())
        .manage(DesktopMenuStateCache::default())
        .manage(DesktopStorage::default())
        .manage(DesktopWebviewRecoveryController::default());

    // EXCEPTION: ship the minimal automation runtime in the real desktop binary so
    // release CI can drive the exact app users install. Broader automation-only
    // behavior remains gated behind `feature = "automation"`.
    builder = builder.plugin(automation_init());

    // Keep single-instance behavior for normal desktop usage. Automation builds need
    // isolated instances so tests don't attach to a long-running interactive app.
    #[cfg(not(feature = "automation"))]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_app_window(app);
        }));
    }

    builder = builder
        .menu(|app| build_app_menu(app))
        .on_menu_event(handle_app_menu_event)
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            desktop_get_connection,
            desktop_daemon_request,
            desktop_disconnect,
            desktop_set_demo_connection,
            desktop_connect_local,
            desktop_restart_local_daemon,
            desktop_connect_ssh,
            desktop_connect_ssh_begin,
            desktop_connect_ssh_poll,
            desktop_update_remote_daemon,
            desktop_kickoff_remote_prewarm,
            desktop_ensure_local_linux_sandbox_ready,
            desktop_ensure_remote_linux_sandbox_ready,
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
            desktop_read_binary_file,
            desktop_get_deep_link_token,
            desktop_set_open_workspaces,
            desktop_open_launcher_in_new_window,
            desktop_open_workspace_in_new_window,
            desktop_open_workspace_setup_in_new_window,
            desktop_set_titlebar_color,
            desktop_set_menu_state,
            desktop_set_window_title,
            desktop_get_notification_permission,
            desktop_request_notification_permission,
            desktop_show_system_notification,
            desktop_get_notification_automation_snapshot,
            desktop_clear_notification_automation_snapshot,
            desktop_get_delivered_notification_automation_snapshot,
            desktop_clear_delivered_notification_automation_snapshot,
            desktop_simulate_last_notification_click,
            desktop_sync_workspace_attention,
            desktop_clear_window_attention,
            desktop_get_attention_automation_snapshot,
            desktop_trigger_menu_command,
            desktop_get_menu_item_state,
            desktop_record_workspace_visit,
            desktop_set_dock_recent_local_workspaces,
            desktop_register_workspace_window,
            desktop_unregister_workspace_window,
            desktop_upload_blob,
            desktop_storage_get,
            desktop_storage_batch,
            desktop_storage_consume_notice,
            desktop_webview_recovery_heartbeat,
            desktop_webview_recovery_consume_incidents,
            desktop_trigger_webview_recovery_fault,
            desktop_get_webview_recovery_automation_snapshot,
            desktop_start_codex_login_relay,
            desktop_get_app_update_state,
            desktop_check_app_update,
            desktop_get_last_app_update_attempt,
            desktop_apply_app_update,
            desktop_restart_app,
        ])
        .setup(|app| {
            enforce_desktop_parity_bundle_preflight(&app.handle())?;
            setup_webview_recovery(&app.handle());
            open_main_window(&app.handle())?;
            install_macos_dock_menu_bridge(app.handle().clone());
            install_macos_notification_delegate(app.handle().clone());
            schedule_local_daemon_prewarm(app.handle().clone());
            schedule_local_linux_sandbox_prefetch(app.handle().clone());
            schedule_force_launcher(app.handle().clone());
            schedule_startup_workspaces(app.handle().clone());
            setup_deep_link_listener(&app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Focused(is_focused) = event {
                if *is_focused {
                    let app_handle = window.app_handle();
                    let recovery = app_handle.state::<DesktopWebviewRecoveryController>();
                    recovery.rearm_heartbeat_detection(window.label());
                    mark_menu_state_window_focused(&app_handle, window.label());
                    if let Err(err) =
                        apply_cached_menu_state_for_window(&app_handle, window.label())
                    {
                        eprintln!(
                            "failed to apply cached desktop menu state for window '{}': {}",
                            window.label(),
                            err
                        );
                    }
                }
            }
            if matches!(event, tauri::WindowEvent::Destroyed) {
                let app_handle = window.app_handle();
                note_window_destroyed(&app_handle, window.label());
                clear_cached_menu_state_for_window(&app_handle, window.label());
                let registry = window.state::<WorkspaceWindowRegistry>();
                registry.unregister_window(window.label());
                let manager = window.state::<ConnectionManager>();
                manager.disconnect_for_scope(window.label());
                let attention = window.state::<DesktopAttentionRegistry>();
                attention.clear_window_attention(window.label());
                if let Err(err) = attention.apply_to_app(&app_handle) {
                    eprintln!(
                        "failed to clear desktop attention for window '{}': {}",
                        window.label(),
                        err
                    );
                }
            }
        });
    builder = builder.on_webview_event(|webview, event| {
        if matches!(event, tauri::WebviewEvent::WebContentProcessTerminated) {
            let app_handle = webview.app_handle().clone();
            let window_label = webview.label().to_string();
            tauri::async_runtime::spawn(async move {
                let _ = handle_native_process_termination(&app_handle, &window_label).await;
            });
        }
    });

    #[cfg(feature = "stt")]
    {
        builder = builder.plugin(tauri_plugin_stt::init());
    }

    let app = match builder.build(tauri::generate_context!()) {
        Ok(app) => app,
        Err(err) => {
            eprintln!("error while building tauri application: {err}");
            return;
        }
    };

    app.run(|app_handle, event| {
        if matches!(
            event,
            tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
        ) {
            let manager = app_handle.state::<ConnectionManager>();
            manager.disconnect_all();
        }
    });
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

        let mut workspace_ids: Vec<String> = Vec::new();
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
            .map(|id: String| urlencoding::encode(&id).into_owned())
            .collect::<Vec<_>>()
            .join(",");
        let url = format!("/workspaces/{}?ctxTabs={tabs}", urlencoding::encode(&first));
        if let Some(window) = app.get_webview_window("main") {
            eprintln!("CTX_DESKTOP_START_WORKSPACE_PATHS: navigating to {url}");
            let js = format!(
                "window.location.href = {};",
                serde_json::to_string(&url).unwrap_or_else(|_| "\"/\"".to_string())
            );
            let _ = window.eval(&js);
        }
    });
}

fn schedule_local_daemon_prewarm(app: tauri::AppHandle) {
    if !env_bool("CTX_DESKTOP_PREWARM_LOCAL_DAEMON_ON_STARTUP", true) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        let state = app.state::<ConnectionManager>();
        if let Err(err) = ensure_local_connection(&app, &state) {
            eprintln!("startup local daemon prewarm failed: {err:#}");
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
