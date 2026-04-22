#[cfg(target_os = "macos")]
use anyhow::Context;
use anyhow::Result;
use ctx_desktop_ipc::{
    DesktopNotificationKind, DesktopNotificationPermission, DesktopShowSystemNotificationReq,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::Manager;
use url::Url;

#[cfg(target_os = "macos")]
const NOTIFICATION_DEEP_LINK_USER_INFO_KEY: &str = "deep_link";
#[cfg(any(target_os = "macos", test))]
const NOTIFICATION_IDENTIFIER_PREFIX: &str = "ctx-task-notification-";

#[cfg(target_os = "macos")]
use std::ptr::NonNull;
#[cfg(target_os = "macos")]
use std::sync::{mpsc, OnceLock};
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
use block2::{DynBlock, RcBlock};
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
#[cfg(target_os = "macos")]
use objc2::{define_class, msg_send, AnyThread};
#[cfg(target_os = "macos")]
#[cfg(all(target_os = "macos", feature = "automation"))]
use objc2_foundation::NSArray;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSDictionary, NSError, NSObject, NSObjectProtocol, NSString};
#[cfg(target_os = "macos")]
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSettings, UNNotificationSound,
    UNUserNotificationCenter, UNUserNotificationCenterDelegate,
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopDeliveredNotificationSnapshot {
    delivered: Vec<DesktopDeliveredNotificationEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopDeliveredNotificationEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deep_link: Option<String>,
    identifier: String,
    title: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DesktopClearDeliveredNotificationsReq {
    identifiers: Vec<String>,
}

#[derive(Debug, Default)]
pub(super) struct DesktopNotificationAutomationState {
    snapshot: Mutex<DesktopNotificationAutomationSnapshot>,
}

impl DesktopNotificationAutomationState {
    #[cfg(any(feature = "automation", test))]
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

    #[cfg(any(feature = "automation", test))]
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

#[cfg(any(target_os = "macos", test))]
fn notification_deep_link_from_payload_value(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if url.scheme() != "ctx" || url.host_str() != Some("task") {
        return None;
    }
    Some(url.to_string())
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
static MACOS_NOTIFICATION_APP: OnceLock<tauri::AppHandle> = OnceLock::new();

#[cfg(target_os = "macos")]
static MACOS_NOTIFICATION_DELEGATE: OnceLock<Retained<MacosNotificationDelegate>> = OnceLock::new();

#[cfg(target_os = "macos")]
define_class!(
    // SAFETY:
    // - The superclass NSObject does not have subclassing requirements.
    // - `MacosNotificationDelegate` does not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    struct MacosNotificationDelegate;

    // SAFETY: `NSObjectProtocol` has no safety requirements.
    unsafe impl NSObjectProtocol for MacosNotificationDelegate {}

    // SAFETY: `UNUserNotificationCenterDelegate` callbacks are invoked by
    // UserNotifications with the generated Objective-C signatures below.
    unsafe impl UNUserNotificationCenterDelegate for MacosNotificationDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present_notification(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion_handler: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion_handler.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive_notification_response(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion_handler: &DynBlock<dyn Fn()>,
        ) {
            if macos_response_is_default_action(response) {
                let notification = response.notification();
                let request = notification.request();
                let content = request.content();
                let user_info = content.userInfo();
                if let Some(deep_link) = macos_notification_deep_link(&user_info) {
                    if let Some(app) = MACOS_NOTIFICATION_APP.get().cloned() {
                        open_notification_target(app, &deep_link);
                    }
                }
            }
            completion_handler.call(());
        }
    }
);

#[cfg(target_os = "macos")]
impl MacosNotificationDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn install_macos_notification_delegate(app: tauri::AppHandle) {
    let _ = MACOS_NOTIFICATION_APP.set(app);
    let delegate = MACOS_NOTIFICATION_DELEGATE.get_or_init(MacosNotificationDelegate::new);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.setDelegate(Some(ProtocolObject::from_ref(&**delegate)));
}

#[cfg(not(target_os = "macos"))]
pub(super) fn install_macos_notification_delegate(_app: tauri::AppHandle) {}

#[cfg(target_os = "macos")]
fn macos_response_is_default_action(response: &UNNotificationResponse) -> bool {
    let action_identifier = response.actionIdentifier();
    unsafe { &*action_identifier == UNNotificationDefaultActionIdentifier }
}

#[cfg(target_os = "macos")]
fn macos_notification_deep_link(user_info: &NSDictionary) -> Option<String> {
    let key = NSString::from_str(NOTIFICATION_DEEP_LINK_USER_INFO_KEY);
    let value = user_info.objectForKey(&**key)?;
    let value = value.downcast::<NSString>().ok()?;
    notification_deep_link_from_payload_value(Some(&value.to_string()))
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
fn schedule_macos_notification(request: &UNNotificationRequest) -> Result<()> {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |err: *mut NSError| {
        let result = match NonNull::new(err) {
            Some(err) => {
                let err = unsafe { err.as_ref() };
                Err(format!("failed to schedule macOS notification: {err}"))
            }
            None => Ok(()),
        };
        let _ = tx.send(result);
    });
    center.addNotificationRequest_withCompletionHandler(request, Some(&completion));
    rx.recv_timeout(Duration::from_secs(2))
        .context("timed out scheduling macOS notification")?
        .map_err(anyhow::Error::msg)
}

#[cfg(any(all(target_os = "macos", feature = "automation"), test))]
fn normalize_delivered_notification_identifiers(identifiers: &[String]) -> Result<Vec<String>> {
    let mut normalized = Vec::new();
    for identifier in identifiers {
        let trimmed = identifier.trim();
        if trimmed.is_empty() {
            anyhow::bail!("delivered notification identifier must not be empty");
        }
        if !trimmed.starts_with(NOTIFICATION_IDENTIFIER_PREFIX) {
            anyhow::bail!("delivered notification identifier is not owned by ctx: {trimmed}");
        }
        normalized.push(trimmed.to_string());
    }
    if normalized.is_empty() {
        anyhow::bail!("at least one delivered notification identifier is required");
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

#[cfg(all(target_os = "macos", feature = "automation"))]
fn macos_delivered_notification_entry(
    notification: &UNNotification,
) -> DesktopDeliveredNotificationEntry {
    let request = notification.request();
    let identifier = request.identifier().to_string();
    let content = request.content();
    let title = content.title().to_string();
    let body = content.body().to_string();
    let user_info = content.userInfo();
    DesktopDeliveredNotificationEntry {
        body: if body.trim().is_empty() {
            None
        } else {
            Some(body)
        },
        deep_link: macos_notification_deep_link(&user_info),
        identifier,
        title,
    }
}

#[cfg(all(target_os = "macos", feature = "automation"))]
fn macos_delivered_notification_entries() -> Result<Vec<DesktopDeliveredNotificationEntry>> {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let (tx, rx) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |notifications: NonNull<NSArray<UNNotification>>| {
        let notifications = unsafe { notifications.as_ref() };
        let entries = notifications
            .to_vec()
            .iter()
            .map(|notification| macos_delivered_notification_entry(notification))
            .collect::<Vec<_>>();
        let _ = tx.send(entries);
    });
    center.getDeliveredNotificationsWithCompletionHandler(&completion);
    rx.recv_timeout(Duration::from_secs(2))
        .context("timed out reading delivered macOS notifications")
}

#[cfg(all(target_os = "macos", feature = "automation"))]
fn macos_clear_delivered_notifications(req: DesktopClearDeliveredNotificationsReq) -> Result<()> {
    let identifiers = normalize_delivered_notification_identifiers(&req.identifiers)?;
    let ns_identifiers = identifiers
        .iter()
        .map(|identifier| NSString::from_str(identifier))
        .collect::<Vec<_>>();
    let identifier_array = NSArray::from_retained_slice(&ns_identifiers);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removeDeliveredNotificationsWithIdentifiers(&identifier_array);
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_macos_notification(
    app: &tauri::AppHandle,
    req: DesktopShowSystemNotificationReq,
    deep_link: String,
) -> Result<()> {
    install_macos_notification_delegate(app.clone());

    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(&req.title));
    if let Some(body) = req.body.as_deref() {
        content.setBody(&NSString::from_str(body));
    }
    let sound = UNNotificationSound::defaultSound();
    content.setSound(Some(&sound));

    let deep_link_key = NSString::from_str(NOTIFICATION_DEEP_LINK_USER_INFO_KEY);
    let deep_link_value = NSString::from_str(&deep_link);
    let user_info: Retained<NSDictionary<NSString, NSString>> =
        NSDictionary::from_slices(&[&*deep_link_key], &[&*deep_link_value]);
    unsafe { content.setUserInfo(user_info.cast_unchecked::<AnyObject, AnyObject>()) };

    let identifier = NSString::from_str(&format!(
        "{}{}",
        NOTIFICATION_IDENTIFIER_PREFIX,
        uuid::Uuid::new_v4()
    ));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&identifier, &content, None);
    schedule_macos_notification(&request)
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
pub(super) fn desktop_get_delivered_notification_automation_snapshot(
) -> Result<DesktopDeliveredNotificationSnapshot, String> {
    #[cfg(all(feature = "automation", target_os = "macos"))]
    {
        return macos_delivered_notification_entries()
            .map(|delivered| DesktopDeliveredNotificationSnapshot { delivered })
            .map_err(super::to_err);
    }

    #[cfg(all(feature = "automation", not(target_os = "macos")))]
    {
        Err("desktop_get_delivered_notification_automation_snapshot is macOS-only".to_string())
    }

    #[cfg(not(feature = "automation"))]
    {
        Err("desktop_get_delivered_notification_automation_snapshot is automation-only".to_string())
    }
}

#[tauri::command]
pub(super) fn desktop_clear_delivered_notification_automation_snapshot(
    req: DesktopClearDeliveredNotificationsReq,
) -> Result<(), String> {
    #[cfg(all(feature = "automation", target_os = "macos"))]
    {
        return macos_clear_delivered_notifications(req).map_err(super::to_err);
    }

    #[cfg(all(feature = "automation", not(target_os = "macos")))]
    {
        let DesktopClearDeliveredNotificationsReq { identifiers } = req;
        let _ = identifiers;
        Err("desktop_clear_delivered_notification_automation_snapshot is macOS-only".to_string())
    }

    #[cfg(not(feature = "automation"))]
    {
        let DesktopClearDeliveredNotificationsReq { identifiers } = req;
        let _ = identifiers;
        Err(
            "desktop_clear_delivered_notification_automation_snapshot is automation-only"
                .to_string(),
        )
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
    fn builds_task_deep_link_trims_ids_and_omits_blank_session() {
        let url = build_notification_deep_link(&DesktopShowSystemNotificationReq {
            kind: ctx_desktop_ipc::DesktopNotificationKind::TurnCompleted,
            body: Some("Done".to_string()),
            session_id: Some("  ".to_string()),
            task_id: " task-1 ".to_string(),
            title: "Turn completed".to_string(),
            workspace_id: " workspace-1 ".to_string(),
        })
        .expect("deep link");

        assert_eq!(url, "ctx://task?v=1&workspaceId=workspace-1&taskId=task-1");
    }

    #[test]
    fn rejects_missing_task_deep_link_inputs() {
        let mut req = DesktopShowSystemNotificationReq {
            kind: ctx_desktop_ipc::DesktopNotificationKind::TurnCompleted,
            body: Some("Done".to_string()),
            session_id: None,
            task_id: "task-1".to_string(),
            title: "Turn completed".to_string(),
            workspace_id: "workspace-1".to_string(),
        };

        req.workspace_id = " ".to_string();
        assert!(build_notification_deep_link(&req).is_err());

        req.workspace_id = "workspace-1".to_string();
        req.task_id = " ".to_string();
        assert!(build_notification_deep_link(&req).is_err());
    }

    #[test]
    fn accepts_only_valid_task_notification_payload_links() {
        assert_eq!(
            notification_deep_link_from_payload_value(Some(
                " ctx://task?v=1&workspaceId=workspace-1&taskId=task-1 "
            )),
            Some("ctx://task?v=1&workspaceId=workspace-1&taskId=task-1".to_string())
        );
        assert_eq!(notification_deep_link_from_payload_value(None), None);
        assert_eq!(notification_deep_link_from_payload_value(Some("")), None);
        assert_eq!(
            notification_deep_link_from_payload_value(Some("https://example.com")),
            None
        );
        assert_eq!(
            notification_deep_link_from_payload_value(Some("ctx://settings")),
            None
        );
        assert_eq!(
            notification_deep_link_from_payload_value(Some("not a url")),
            None
        );
    }

    #[test]
    fn delivered_notification_clear_requires_ctx_owned_identifiers() {
        let req = DesktopClearDeliveredNotificationsReq {
            identifiers: vec!["ctx-task-notification-from-req".to_string()],
        };
        assert_eq!(
            normalize_delivered_notification_identifiers(&req.identifiers)
                .expect("req identifiers"),
            vec!["ctx-task-notification-from-req".to_string()]
        );

        assert_eq!(
            normalize_delivered_notification_identifiers(&[
                " ctx-task-notification-b ".to_string(),
                "ctx-task-notification-a".to_string(),
                "ctx-task-notification-a".to_string(),
            ])
            .expect("identifiers"),
            vec![
                "ctx-task-notification-a".to_string(),
                "ctx-task-notification-b".to_string(),
            ]
        );

        assert!(normalize_delivered_notification_identifiers(&[]).is_err());
        assert!(normalize_delivered_notification_identifiers(&[" ".to_string()]).is_err());
        assert!(
            normalize_delivered_notification_identifiers(&["other-notification".to_string(),])
                .is_err()
        );
    }

    #[test]
    fn delivered_notification_snapshot_serializes_camel_case() {
        let snapshot = DesktopDeliveredNotificationSnapshot {
            delivered: vec![DesktopDeliveredNotificationEntry {
                body: Some("Body".to_string()),
                deep_link: Some("ctx://task?v=1&workspaceId=workspace-1&taskId=task-1".to_string()),
                identifier: "ctx-task-notification-1".to_string(),
                title: "Title".to_string(),
            }],
        };

        let value = serde_json::to_value(snapshot).expect("snapshot json");
        assert_eq!(
            value,
            serde_json::json!({
                "delivered": [{
                    "body": "Body",
                    "deepLink": "ctx://task?v=1&workspaceId=workspace-1&taskId=task-1",
                    "identifier": "ctx-task-notification-1",
                    "title": "Title",
                }],
            })
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
