use ctx_core::models::{
    Artifact, Message, MessageRole, Session, SessionCatchupSummary, SessionHead,
    SessionHistoryPage, SessionStatus,
};

#[derive(Clone)]
pub(crate) struct MessageItem {
    pub(crate) role: String,
    pub(crate) content: String,
}

impl MessageItem {
    pub(crate) fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

pub(crate) struct SessionInfo {
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) detail: String,
}

impl SessionInfo {
    pub(crate) fn placeholder() -> Self {
        Self {
            title: "Session".to_string(),
            status: "Idle".to_string(),
            detail: "Select a task to begin.".to_string(),
        }
    }
}

fn message_role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    }
}

fn message_item_from_model(message: &Message) -> MessageItem {
    MessageItem::new(message_role_label(&message.role), message.content.clone())
}

fn session_status_label(status: &SessionStatus) -> &'static str {
    match status {
        SessionStatus::Active => "Active",
        SessionStatus::Completed => "Completed",
        SessionStatus::Failed => "Failed",
        SessionStatus::Cancelled => "Cancelled",
    }
}

fn session_status_text(status: &SessionStatus, is_working: bool) -> String {
    if is_working {
        "Working".to_string()
    } else {
        session_status_label(status).to_string()
    }
}

fn session_detail_text(session: &Session) -> String {
    format!(
        "Provider: {} / Model: {} / Role: {}",
        session.provider_id, session.model_id, session.agent_role
    )
}

pub(crate) fn artifact_label(artifact: &Artifact) -> String {
    if let Some(name) = artifact.name.as_ref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if !artifact.absolute_path.is_empty() {
        if let Some(last) = artifact
            .absolute_path
            .rsplit(|ch| ch == '/' || ch == '\\')
            .next()
        {
            if !last.is_empty() {
                return last.to_string();
            }
        }
    }
    format!("artifact-{}", artifact.id.0)
}

pub(crate) fn session_info_from_head(head: &SessionHead) -> SessionInfo {
    let title = if head.session.title.is_empty() {
        "Session".to_string()
    } else {
        head.session.title.clone()
    };
    let status = session_status_text(&head.session.status, head.activity.is_working);
    let detail = session_detail_text(&head.session);
    SessionInfo {
        title,
        status,
        detail,
    }
}

pub(crate) fn session_info_from_summary(summary: &SessionCatchupSummary) -> SessionInfo {
    let title = if summary.session.title.is_empty() {
        "Session".to_string()
    } else {
        summary.session.title.clone()
    };
    let status = session_status_text(&summary.session.status, summary.activity.is_working);
    let detail = session_detail_text(&summary.session);
    SessionInfo {
        title,
        status,
        detail,
    }
}

pub(crate) fn build_message_items(
    session_head: Option<&SessionHead>,
    session_history: Option<&SessionHistoryPage>,
) -> Vec<MessageItem> {
    if let Some(history) = session_history {
        if !history.messages.is_empty() {
            return history
                .messages
                .iter()
                .map(message_item_from_model)
                .collect();
        }
    }

    if let Some(head) = session_head {
        if !head.messages.is_empty() {
            return head
                .messages
                .iter()
                .map(message_item_from_model)
                .collect();
        }
    }

    Vec::new()
}
