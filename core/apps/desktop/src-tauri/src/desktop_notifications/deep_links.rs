use url::Url;

use super::*;

#[cfg(any(target_os = "macos", test))]
pub(super) fn notification_deep_link_from_payload_value(value: Option<&str>) -> Option<String> {
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

pub(super) fn build_notification_deep_link(
    req: &DesktopShowSystemNotificationReq,
) -> Result<String> {
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

pub(super) fn open_notification_target(app: tauri::AppHandle, deep_link: &str) {
    let Ok(url) = Url::parse(deep_link) else {
        eprintln!("invalid notification deep link: {deep_link}");
        return;
    };
    super::super::handle_deep_link(app, url);
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
}
