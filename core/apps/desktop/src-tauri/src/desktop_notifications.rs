use anyhow::Result;
use ctx_desktop_ipc::{
    DesktopNotificationKind, DesktopNotificationPermission, DesktopShowSystemNotificationReq,
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::Manager;
use url::Url;

#[cfg(target_os = "macos")]
use std::ptr::NonNull;
#[cfg(target_os = "macos")]
use std::sync::mpsc;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use mac_notification_sys::{Notification, NotificationResponse};
#[cfg(target_os = "macos")]
use objc2::runtime::Bool;
#[cfg(target_os = "macos")]
use objc2_foundation::NSError;
#[cfg(target_os = "macos")]
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopNotificationAutomationSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deep_link: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<DesktopNotificationKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    shown_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct DesktopNotificationAutomationState {
    snapshot: Mutex<DesktopNotificationAutomationSnapshot>,
}

impl DesktopNotificationAutomationState {
    fn clear(&self) {
        if let Ok(mut guard) = self.snapshot.lock() {
            *guard = DesktopNotificationAutomationSnapshot::default();
        }
    }

    fn record(&self, req: &DesktopShowSystemNotificationReq, deep_link: &str) {
        if let Ok(mut guard) = self.snapshot.lock() {
            let next_count = guard.shown_count.saturating_add(1);
            *guard = DesktopNotificationAutomationSnapshot {
                body: req.body.clone(),
                deep_link: Some(deep_link.to_string()),
                kind: Some(req.kind),
                session_id: req.session_id.clone(),
                shown_count: next_count,
                task_id: Some(req.task_id.clone()),
                title: Some(req.title.clone()),
                workspace_id: Some(req.workspace_id.clone()),
            };
        }
    }

    fn snapshot(&self) -> DesktopNotificationAutomationSnapshot {
        match self.snapshot.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => DesktopNotificationAutomationSnapshot::default(),
        }
    }

    #[cfg(feature = "automation")]
    fn last_deep_link(&self) -> Option<String> {
        match self.snapshot.lock() {
            Ok(guard) => guard.deep_link.clone(),
            Err(_) => None,
        }
    }
}

fn build_notification_deep_link(req: &DesktopShowSystemNotificationReq) -> Result<String> {
    let workspace_id = req.workspace_id.trim();
    if workspace_id.is_empty() {
        anyhow::bail!("workspace_id is required");
    }
    let task_id = req.task_id.trim();
    if task_id.is_empty() {
        anyhow::bail!("task_id is required");
    }
    let mut url = Url::parse("ctx://task")?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("v", "1");
        pairs.append_pair("workspaceId", workspace_id);
        pairs.append_pair("taskId", task_id);
        if let Some(session_id) = req.session_id.as_deref().map(str::trim) {
            if !session_id.is_empty() {
                pairs.append_pair("sessionId", session_id);
            }
        }
    }
    Ok(url.to_string())
}

fn open_notification_target(app: tauri::AppHandle, deep_link: &str) {
    let Ok(url) = Url::parse(deep_link) else {
        eprintln!("invalid notification deep link: {deep_link}");
        return;
    };
    super::handle_deep_link(app, url);
}

#[cfg(target_os = "macos")]
fn map_macos_permission(status: UNAuthorizationStatus) -> DesktopNotificationPermission {
    if status == UNAuthorizationStatus::Authorized
        || status == UNAuthorizationStatus::Provisional
        || status == UNAuthorizationStatus::Ephemeral
    {
        DesktopNotificationPermission::Granted
    } else if status == UNAuthorizationStatus::Denied {
        DesktopNotificationPermission::Denied
    } else {
        DesktopNotificationPermission::Default
    }
}

#[cfg(target_os = "macos")]
fn macos_notification_permission() -> DesktopNotificationPermission {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
        let permission = unsafe { map_macos_permission(settings.as_ref().authorizationStatus()) };
        let _ = tx.send(permission);
    });
    center.getNotificationSettingsWithCompletionHandler(&completion);
    rx.recv_timeout(Duration::from_secs(2))
        .unwrap_or(DesktopNotificationPermission::Default)
}

#[cfg(target_os = "macos")]
fn macos_request_notification_permission() -> DesktopNotificationPermission {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
        let permission = if granted.as_bool() {
            DesktopNotificationPermission::Granted
        } else {
            DesktopNotificationPermission::Denied
        };
        let _ = tx.send(permission);
    });
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Sound
            | UNAuthorizationOptions::Badge,
        &completion,
    );
    rx.recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|_| macos_notification_permission())
}

#[cfg(target_os = "macos")]
fn show_macos_notification(
    app: &tauri::AppHandle,
    req: DesktopShowSystemNotificationReq,
    deep_link: String,
) -> Result<()> {
    let title = req.title;
    let body = req.body.unwrap_or_default();
    let app = app.clone();
    std::thread::spawn(move || {
        let mut notification = Notification::new();
        notification
            .title(&title)
            .message(&body)
            .asynchronous(false)
            .wait_for_click(true);
        match notification.send() {
            Ok(NotificationResponse::Click) => open_notification_target(app, &deep_link),
            Ok(_) => {}
            Err(err) => eprintln!("macOS notification failed: {err}"),
        }
    });
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn show_linux_notification(
    app: &tauri::AppHandle,
    req: DesktopShowSystemNotificationReq,
    deep_link: String,
) -> Result<()> {
    let mut notification = notify_rust::Notification::new();
    notification.summary(&req.title);
    if let Some(body) = req.body.as_deref() {
        notification.body(body);
    }
    notification.action("default", "Open");
    let handle = notification.show()?;
    let app = app.clone();
    std::thread::spawn(move || {
        handle.wait_for_action(|action| {
            if action == "default" {
                open_notification_target(app, &deep_link);
            }
        });
    });
    Ok(())
}

#[cfg(target_os = "windows")]
fn show_windows_notification(
    req: DesktopShowSystemNotificationReq,
    deep_link: String,
) -> Result<()> {
    let _ = deep_link;
    let mut notification = notify_rust::Notification::new();
    notification.summary(&req.title);
    if let Some(body) = req.body.as_deref() {
        notification.body(body);
    }
    notification.show()?;
    Ok(())
}

pub(super) fn notification_permission() -> DesktopNotificationPermission {
    #[cfg(target_os = "macos")]
    {
        return macos_notification_permission();
    }

    #[cfg(not(target_os = "macos"))]
    {
        DesktopNotificationPermission::Granted
    }
}

pub(super) fn request_notification_permission() -> DesktopNotificationPermission {
    #[cfg(target_os = "macos")]
    {
        return macos_request_notification_permission();
    }

    #[cfg(not(target_os = "macos"))]
    {
        DesktopNotificationPermission::Granted
    }
}

fn should_simulate_system_notifications() -> bool {
    #[cfg(feature = "automation")]
    {
        return matches!(
            std::env::var("CTX_AUTOMATION_SIMULATE_SYSTEM_NOTIFICATIONS"),
            Ok(value) if value.trim() == "1"
        );
    }

    #[cfg(not(feature = "automation"))]
    {
        false
    }
}

pub(super) fn show_system_notification(
    app: &tauri::AppHandle,
    req: DesktopShowSystemNotificationReq,
) -> Result<()> {
    let deep_link = build_notification_deep_link(&req)?;
    let automation = app.state::<DesktopNotificationAutomationState>();
    automation.record(&req, &deep_link);
    if should_simulate_system_notifications() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        return show_macos_notification(app, req, deep_link);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return show_linux_notification(app, req, deep_link);
    }

    #[cfg(target_os = "windows")]
    {
        return show_windows_notification(req, deep_link);
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[tauri::command]
pub(super) fn desktop_get_notification_permission() -> Result<DesktopNotificationPermission, String>
{
    Ok(notification_permission())
}

#[tauri::command]
pub(super) fn desktop_request_notification_permission(
) -> Result<DesktopNotificationPermission, String> {
    Ok(request_notification_permission())
}

#[tauri::command]
pub(super) fn desktop_show_system_notification(
    app: tauri::AppHandle,
    req: DesktopShowSystemNotificationReq,
) -> Result<(), String> {
    show_system_notification(&app, req).map_err(super::to_err)
}

#[tauri::command]
pub(super) fn desktop_get_notification_automation_snapshot(
    state: tauri::State<DesktopNotificationAutomationState>,
) -> Result<DesktopNotificationAutomationSnapshot, String> {
    #[cfg(feature = "automation")]
    {
        return Ok(state.snapshot());
    }

    #[cfg(not(feature = "automation"))]
    {
        let _ = state;
        Err("desktop_get_notification_automation_snapshot is automation-only".to_string())
    }
}

#[tauri::command]
pub(super) fn desktop_clear_notification_automation_snapshot(
    state: tauri::State<DesktopNotificationAutomationState>,
) -> Result<(), String> {
    #[cfg(feature = "automation")]
    {
        state.clear();
        return Ok(());
    }

    #[cfg(not(feature = "automation"))]
    {
        let _ = state;
        Err("desktop_clear_notification_automation_snapshot is automation-only".to_string())
    }
}

#[tauri::command]
pub(super) fn desktop_simulate_last_notification_click(
    app: tauri::AppHandle,
    state: tauri::State<DesktopNotificationAutomationState>,
) -> Result<(), String> {
    #[cfg(feature = "automation")]
    {
        let deep_link = state
            .last_deep_link()
            .ok_or_else(|| "no recorded system notification".to_string())?;
        open_notification_target(app, &deep_link);
        return Ok(());
    }

    #[cfg(not(feature = "automation"))]
    {
        let _ = app;
        let _ = state;
        Err("desktop_simulate_last_notification_click is automation-only".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_task_deep_link_with_optional_session() {
        let url = build_notification_deep_link(&DesktopShowSystemNotificationReq {
            kind: ctx_desktop_ipc::DesktopNotificationKind::TurnCompleted,
            body: Some("Done".to_string()),
            session_id: Some("session-1".to_string()),
            task_id: "task-1".to_string(),
            title: "Turn completed".to_string(),
            workspace_id: "workspace-1".to_string(),
        })
        .expect("deep link");

        assert_eq!(
            url,
            "ctx://task?v=1&workspaceId=workspace-1&taskId=task-1&sessionId=session-1"
        );
    }

    #[test]
    fn automation_state_records_latest_notification() {
        let state = DesktopNotificationAutomationState::default();
        state.record(
            &DesktopShowSystemNotificationReq {
                kind: DesktopNotificationKind::TurnCompleted,
                body: Some("Done".to_string()),
                session_id: Some("session-1".to_string()),
                task_id: "task-1".to_string(),
                title: "Turn completed".to_string(),
                workspace_id: "workspace-1".to_string(),
            },
            "ctx://task?v=1&workspaceId=workspace-1&taskId=task-1&sessionId=session-1",
        );
        state.record(
            &DesktopShowSystemNotificationReq {
                kind: DesktopNotificationKind::TurnFailed,
                body: Some("Failed".to_string()),
                session_id: None,
                task_id: "task-2".to_string(),
                title: "Turn failed".to_string(),
                workspace_id: "workspace-2".to_string(),
            },
            "ctx://task?v=1&workspaceId=workspace-2&taskId=task-2",
        );

        assert_eq!(
            state.snapshot(),
            DesktopNotificationAutomationSnapshot {
                body: Some("Failed".to_string()),
                deep_link: Some("ctx://task?v=1&workspaceId=workspace-2&taskId=task-2".to_string()),
                kind: Some(DesktopNotificationKind::TurnFailed),
                session_id: None,
                shown_count: 2,
                task_id: Some("task-2".to_string()),
                title: Some("Turn failed".to_string()),
                workspace_id: Some("workspace-2".to_string()),
            }
        );
    }

    #[test]
    fn automation_state_clear_resets_snapshot() {
        let state = DesktopNotificationAutomationState::default();
        state.record(
            &DesktopShowSystemNotificationReq {
                kind: DesktopNotificationKind::TurnCompleted,
                body: None,
                session_id: None,
                task_id: "task-1".to_string(),
                title: "Turn completed".to_string(),
                workspace_id: "workspace-1".to_string(),
            },
            "ctx://task?v=1&workspaceId=workspace-1&taskId=task-1",
        );

        state.clear();

        assert_eq!(
            state.snapshot(),
            DesktopNotificationAutomationSnapshot::default()
        );
    }
}
