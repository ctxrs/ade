use std::path::Path;

use ctx_core::models::{
    Artifact, Message, MessageRole, Session, SessionCatchupSummary, SessionEventType, SessionHead,
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

pub(crate) fn message_item_from_model(message: &Message) -> MessageItem {
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

pub(crate) fn session_event_type_label(event_type: &SessionEventType) -> &'static str {
    match event_type {
        SessionEventType::Init => "init",
        SessionEventType::UserMessage => "user_message",
        SessionEventType::InputQueued => "input_queued",
        SessionEventType::AuthRequired => "auth_required",
        SessionEventType::Notice => "notice",
        SessionEventType::AssistantChunk => "assistant_chunk",
        SessionEventType::ThoughtChunk => "thought_chunk",
        SessionEventType::AssistantComplete => "assistant_complete",
        SessionEventType::AssistantMessageInserted => "assistant_message_inserted",
        SessionEventType::ToolCall => "tool_call",
        SessionEventType::ToolCallUpdate => "tool_call_update",
        SessionEventType::ToolResult => "tool_result",
        SessionEventType::Plan => "plan",
        SessionEventType::ArtifactsSet => "artifacts_set",
        SessionEventType::Done => "done",
        SessionEventType::InterruptRequested => "interrupt_requested",
        SessionEventType::TurnInterrupted => "turn_interrupted",
        SessionEventType::Error => "error",
    }
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

fn artifact_extension(artifact: &Artifact) -> Option<String> {
    if artifact.absolute_path.is_empty() {
        return None;
    }
    Path::new(&artifact.absolute_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

pub(crate) fn is_image_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif"
    ) {
        return true;
    }
    matches!(
        artifact_extension(artifact).as_deref(),
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif")
    )
}

pub(crate) fn is_text_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if mime.starts_with("text/") {
        return true;
    }
    if matches!(
        mime.as_str(),
        "application/json"
            | "application/xml"
            | "application/yaml"
            | "application/x-yaml"
            | "application/toml"
            | "application/javascript"
            | "application/x-javascript"
            | "application/typescript"
            | "application/x-sh"
            | "application/x-shellscript"
            | "application/csv"
    ) {
        return true;
    }

    matches!(
        artifact_extension(artifact).as_deref(),
        Some("txt")
            | Some("md")
            | Some("json")
            | Some("yaml")
            | Some("yml")
            | Some("toml")
            | Some("rs")
            | Some("js")
            | Some("ts")
            | Some("tsx")
            | Some("jsx")
            | Some("html")
            | Some("css")
            | Some("csv")
            | Some("log")
            | Some("py")
            | Some("go")
            | Some("java")
            | Some("c")
            | Some("cpp")
            | Some("h")
            | Some("hpp")
            | Some("sh")
            | Some("bash")
            | Some("zsh")
            | Some("sql")
    )
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
