use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use regex::Regex;
use ctx_core::ids::{MessageId, TurnId};
use ctx_core::models::{
    Artifact, Message, MessageDelivery, MessageRole, Session, SessionCatchupSummary,
    SessionEvent, SessionEventType, SessionHead, SessionHistoryPage, SessionStatus, SessionTurn,
    SessionTurnStatus, SessionTurnTool, SessionTurnToolSummary,
};
use serde::Serialize;
use serde_json::Value;

pub(crate) use ctx_core::models::MessageAttachment;

#[derive(Clone)]
pub(crate) struct MessageItem {
    pub(crate) id: Option<MessageId>,
    pub(crate) turn_id: Option<TurnId>,
    pub(crate) turn_sequence: Option<i64>,
    pub(crate) role: MessageRole,
    pub(crate) content: String,
    pub(crate) attachments: Vec<MessageAttachment>,
    pub(crate) delivery: MessageDelivery,
    pub(crate) created_at: DateTime<Utc>,
}

impl MessageItem {
    pub(crate) fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            id: None,
            turn_id: None,
            turn_sequence: None,
            role,
            content: content.into(),
            attachments: Vec::new(),
            delivery: MessageDelivery::Immediate,
            created_at: Utc::now(),
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

#[allow(dead_code)]
fn message_role_label(role: &MessageRole) -> &'static str {
    match role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    }
}

pub(crate) fn message_item_from_model(message: &Message) -> MessageItem {
    MessageItem {
        id: Some(message.id),
        turn_id: message.turn_id,
        turn_sequence: message.turn_sequence,
        role: message.role.clone(),
        content: message.content.clone(),
        attachments: message.attachments.clone(),
        delivery: message.delivery.clone(),
        created_at: message.created_at,
    }
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

#[allow(dead_code)]
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
    let path = if !artifact.absolute_path.is_empty() {
        Some(artifact.absolute_path.as_str())
    } else {
        artifact.name.as_deref()
    }?;
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
}

pub(crate) fn is_image_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if matches!(
        mime.as_str(),
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
    ) {
        return true;
    }
    matches!(
        artifact_extension(artifact).as_deref(),
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp")
    )
}

pub(crate) fn is_video_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if mime.starts_with("video/") {
        return true;
    }
    if matches!(mime.as_str(), "application/mp4" | "application/quicktime") {
        return true;
    }
    matches!(
        artifact_extension(artifact).as_deref(),
        Some("mp4") | Some("webm") | Some("mov") | Some("m4v")
    )
}

pub(crate) fn is_pdf_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if matches!(mime.as_str(), "application/pdf" | "application/x-pdf") {
        return true;
    }
    matches!(artifact_extension(artifact).as_deref(), Some("pdf"))
}

pub(crate) fn is_diff_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if matches!(
        mime.as_str(),
        "text/diff"
            | "text/x-diff"
            | "text/x-patch"
            | "application/diff"
            | "application/x-diff"
            | "application/x-patch"
    ) {
        return true;
    }
    matches!(
        artifact_extension(artifact).as_deref(),
        Some("diff") | Some("patch")
    )
}

pub(crate) fn is_text_artifact(artifact: &Artifact) -> bool {
    let mime = artifact.mime_type.trim().to_ascii_lowercase();
    if mime.starts_with("text/") {
        return true;
    }
    if mime.ends_with("+json") || mime.ends_with("+xml") || mime.ends_with("+yaml") {
        return true;
    }
    if matches!(
        mime.as_str(),
        "application/json"
            | "application/ndjson"
            | "application/x-ndjson"
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
            | "text/x-diff"
            | "text/x-patch"
            | "text/diff"
            | "application/diff"
            | "application/x-diff"
            | "application/x-patch"
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
            | Some("diff")
            | Some("patch")
    )
}

pub(crate) fn is_absolute_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.starts_with('/') {
        return true;
    }
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return true;
    }
    false
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

#[derive(Clone)]
pub(crate) struct WorkbenchTurnHeader {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) plain_text: String,
    pub(crate) attachments: Vec<MessageAttachment>,
    pub(crate) created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub(crate) struct ThreadGroup {
    pub(crate) key: String,
    pub(crate) header: Option<WorkbenchTurnHeader>,
    pub(crate) items: Vec<ThreadItem>,
}

#[derive(Clone)]
pub(crate) struct ThreadViewModel {
    pub(crate) groups: Vec<ThreadGroup>,
}

#[derive(Clone)]
pub(crate) enum ThreadListItem {
    TurnHeader { id: String, header: WorkbenchTurnHeader },
    Item(ThreadItem),
}

impl ThreadListItem {
    pub(crate) fn id(&self) -> &str {
        match self {
            ThreadListItem::TurnHeader { id, .. } => id,
            ThreadListItem::Item(item) => thread_item_id(item),
        }
    }
}


#[derive(Clone, Serialize)]
pub(crate) struct TurnToolSnapshot {
    pub(crate) tool_call_id: String,
    pub(crate) turn_id: TurnId,
    pub(crate) tool_kind: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) input_json: Option<Value>,
    pub(crate) output_text: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) summary_only: bool,
}

impl TurnToolSnapshot {
    #[allow(dead_code)]
    pub(crate) fn from_full(tool: &SessionTurnTool) -> Self {
        Self {
            tool_call_id: tool.tool_call_id.clone(),
            turn_id: tool.turn_id,
            tool_kind: tool.tool_kind.clone(),
            title: tool.title.clone(),
            status: tool.status.clone(),
            input_json: tool.input_json.clone(),
            output_text: tool.output_text.clone(),
            created_at: tool.created_at,
            updated_at: tool.updated_at,
            summary_only: false,
        }
    }

    pub(crate) fn from_summary(summary: &SessionTurnToolSummary) -> Self {
        Self {
            tool_call_id: summary.tool_call_id.clone(),
            turn_id: summary.turn_id,
            tool_kind: summary.tool_kind.clone(),
            title: summary.title.clone(),
            status: summary.status.clone(),
            input_json: summary.input_preview.clone(),
            output_text: None,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
            summary_only: true,
        }
    }
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct ToolLocation {
    pub(crate) path: Option<String>,
    pub(crate) range: Option<Value>,
}

#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct ThreadToolItem {
    pub(crate) id: String,
    pub(crate) turn_id: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) tool_call_id: String,
    pub(crate) tool_kind: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) locations: Vec<ToolLocation>,
    pub(crate) input: Option<Value>,
    pub(crate) output_text: String,
    pub(crate) raw: Option<Value>,
    pub(crate) updates_seen: i64,
    pub(crate) has_details: bool,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) enum ThreadItem {
    Message {
        id: String,
        role: MessageRole,
        content: String,
        attachments: Vec<MessageAttachment>,
        created_at: DateTime<Utc>,
        delivery: Option<MessageDelivery>,
    },
    Spacer {
        id: String,
        created_at: DateTime<Utc>,
    },
    Assistant {
        id: String,
        turn_id: String,
        created_at: DateTime<Utc>,
        content: String,
        thought: String,
        is_complete: bool,
        thought_seconds: Option<i64>,
    },
    Thought {
        id: String,
        turn_id: String,
        created_at: DateTime<Utc>,
        content: String,
    },
    TurnStatus {
        id: String,
        turn_id: String,
        created_at: DateTime<Utc>,
        status: SessionTurnStatus,
        started_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        custom_status: Option<String>,
        status_text: Option<String>,
        assistant_messages_content: Option<String>,
    },
    Tool(ThreadToolItem),
    ToolGroup {
        id: String,
        turn_id: String,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        tool_total: i64,
        tool_pending: i64,
        tool_running: i64,
        tool_completed: i64,
        tool_failed: i64,
        tools: Vec<ThreadToolItem>,
        thought: String,
    },
}

pub(crate) fn build_thread_list_items(
    turns: &[SessionTurn],
    messages: &[MessageItem],
    tools_by_turn_id: &HashMap<TurnId, Vec<TurnToolSnapshot>>,
    events: &[SessionEvent],
) -> Vec<ThreadListItem> {
    let view = build_thread_view_model(turns, messages, tools_by_turn_id, events);
    let mut out = Vec::new();
    for group in view.groups {
        if let Some(header) = group.header {
            out.push(ThreadListItem::TurnHeader {
                id: format!("turn-header-{}", header.id),
                header,
            });
        }
        out.extend(group.items.into_iter().map(ThreadListItem::Item));
    }
    out
}

pub(crate) fn build_thread_view_model(
    turns: &[SessionTurn],
    messages: &[MessageItem],
    tools_by_turn_id: &HashMap<TurnId, Vec<TurnToolSnapshot>>,
    events: &[SessionEvent],
) -> ThreadViewModel {
    if !turns.is_empty() {
        return build_thread_view_model_from_turns(turns, messages, tools_by_turn_id, events);
    }
    build_thread_view_model_from_events(events, messages)
}

#[derive(Clone)]
struct SortableThreadGroup {
    sort_at: DateTime<Utc>,
    group: ThreadGroup,
}

#[derive(Clone, Copy)]
enum ActivityKind {
    Tool,
    Thought,
}

#[derive(Clone)]
struct ActivityEntry {
    item: ThreadItem,
    created_at: DateTime<Utc>,
    kind: ActivityKind,
    order_seq: Option<i64>,
}

fn build_thread_view_model_from_turns(
    turns: &[SessionTurn],
    messages: &[MessageItem],
    tools_by_turn_id: &HashMap<TurnId, Vec<TurnToolSnapshot>>,
    events: &[SessionEvent],
) -> ThreadViewModel {
    let custom_status_by_turn_id = build_custom_status_by_turn_id(events);

    let mut message_by_id = HashMap::new();
    let mut messages_by_turn_id: HashMap<TurnId, Vec<MessageItem>> = HashMap::new();
    for message in messages {
        if let Some(id) = message.id {
            message_by_id.insert(id, message.clone());
        }
        if let Some(turn_id) = message.turn_id {
            messages_by_turn_id
                .entry(turn_id)
                .or_default()
                .push(message.clone());
        }
    }

    let mut events_by_turn_id: HashMap<TurnId, Vec<SessionEvent>> = HashMap::new();
    for event in events {
        if let Some(turn_id) = event.turn_id {
            events_by_turn_id
                .entry(turn_id)
                .or_default()
                .push(event.clone());
        }
    }

    let mut groups: Vec<SortableThreadGroup> = Vec::new();

    for turn in turns {
        let turn_id = turn.turn_id;
        let turn_id_string = turn_id.0.to_string();
        let user_message_id = turn.user_message_id;
        let user_message = user_message_id.and_then(|id| message_by_id.get(&id).cloned());

        let header = user_message.map(|message| WorkbenchTurnHeader {
            id: message_id_string(message.id).unwrap_or_else(|| turn_id_string.clone()),
            content: message.content.clone(),
            plain_text: markdown_to_plain_text(&message.content),
            attachments: message.attachments.clone(),
            created_at: message.created_at,
        });

        let tools_snapshot = tools_by_turn_id
            .get(&turn_id)
            .cloned()
            .unwrap_or_default();
        let tools = tools_snapshot
            .into_iter()
            .map(|tool| {
                let tool_kind = tool.tool_kind.clone().unwrap_or_else(|| "tool".to_string());
                let title = tool
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| human_tool_kind(&tool_kind));
                let has_details = !tool.summary_only
                    && (tool.input_json.is_some()
                        || tool.output_text.as_deref().unwrap_or("").trim().len() > 0);
                ThreadToolItem {
                    id: format!("tool-{}-{}", turn_id_string, tool.tool_call_id),
                    turn_id: turn_id_string.clone(),
                    tool_call_id: tool.tool_call_id.clone(),
                    created_at: tool.created_at,
                    updated_at: tool.updated_at,
                    tool_kind,
                    title,
                    status: tool.status.clone().unwrap_or_else(|| "pending".to_string()),
                    locations: Vec::new(),
                    input: tool.input_json.clone(),
                    output_text: tool.output_text.clone().unwrap_or_default(),
                    raw: Some(serde_json::to_value(tool).unwrap_or(Value::Null)),
                    updates_seen: 1,
                    has_details,
                }
            })
            .collect::<Vec<_>>();

        let (activity, _tools) = build_turn_activity_timeline(
            turn_id,
            turn,
            tools,
            events_by_turn_id.get(&turn_id).cloned().unwrap_or_default(),
        );

        let mut assistant_messages = messages_by_turn_id
            .get(&turn_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|message| matches!(message.role, MessageRole::Assistant))
            .collect::<Vec<_>>();

        assistant_messages.sort_by(|a, b| {
            let sa = a.turn_sequence;
            let sb = b.turn_sequence;
            match (sa, sb) {
                (Some(sa), Some(sb)) if sa != sb => sa.cmp(&sb),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                _ => a
                    .created_at
                    .cmp(&b.created_at)
                    .then_with(|| message_id_string(a.id).cmp(&message_id_string(b.id))),
            }
        });

        #[derive(Clone, Copy)]
        enum TimelineKind {
            Assistant,
            Tool,
            Thought,
        }

        #[derive(Clone)]
        struct TimelineEntry {
            item: ThreadItem,
            created_at: DateTime<Utc>,
            kind: TimelineKind,
            order_seq: Option<i64>,
            turn_sequence: Option<i64>,
        }

        let mut timeline: Vec<TimelineEntry> = Vec::new();
        for message in &assistant_messages {
            timeline.push(TimelineEntry {
                item: ThreadItem::Assistant {
                    id: format!(
                        "assistant-{}-{}",
                        turn_id_string,
                        message.turn_sequence.unwrap_or_default()
                    ),
                    turn_id: turn_id_string.clone(),
                    created_at: message.created_at,
                    content: message.content.clone(),
                    thought: String::new(),
                    is_complete: true,
                    thought_seconds: None,
                },
                created_at: message.created_at,
                kind: TimelineKind::Assistant,
                order_seq: None,
                turn_sequence: message.turn_sequence,
            });
        }

        let pending_content = turn.assistant_partial.clone().unwrap_or_default();
        if !pending_content.trim().is_empty() {
            timeline.push(TimelineEntry {
                item: ThreadItem::Assistant {
                    id: format!("assistant-{}-pending", turn_id_string),
                    turn_id: turn_id_string.clone(),
                    created_at: turn.updated_at,
                    content: pending_content,
                    thought: String::new(),
                    is_complete: false,
                    thought_seconds: None,
                },
                created_at: turn.updated_at,
                kind: TimelineKind::Assistant,
                order_seq: None,
                turn_sequence: Some(i64::MAX),
            });
        }

        for entry in activity {
            let kind = match entry.kind {
                ActivityKind::Tool => TimelineKind::Tool,
                ActivityKind::Thought => TimelineKind::Thought,
            };
            timeline.push(TimelineEntry {
                item: entry.item,
                created_at: entry.created_at,
                kind,
                order_seq: entry.order_seq,
                turn_sequence: None,
            });
        }

        timeline.sort_by(|a, b| {
            if let (Some(a_seq), Some(b_seq)) = (a.order_seq, b.order_seq) {
                if a_seq != b_seq {
                    return a_seq.cmp(&b_seq);
                }
            }

            let time_cmp = a.created_at.cmp(&b.created_at);
            if time_cmp != std::cmp::Ordering::Equal {
                return time_cmp;
            }

            if matches!(a.kind, TimelineKind::Assistant) && matches!(b.kind, TimelineKind::Assistant)
            {
                if let (Some(sa), Some(sb)) = (a.turn_sequence, b.turn_sequence) {
                    if sa != sb {
                        return sa.cmp(&sb);
                    }
                }
            }

            let a_rank = if matches!(a.kind, TimelineKind::Assistant) { 2 } else { 1 };
            let b_rank = if matches!(b.kind, TimelineKind::Assistant) { 2 } else { 1 };
            if a_rank != b_rank {
                return a_rank.cmp(&b_rank);
            }

            thread_item_id(&a.item).cmp(thread_item_id(&b.item))
        });

        let mut items: Vec<ThreadItem> = timeline.into_iter().map(|entry| entry.item).collect();
        if items.is_empty() {
            items.push(ThreadItem::Spacer {
                id: format!("spacer-{}", turn_id_string),
                created_at: turn.started_at,
            });
        }

        let status_text = custom_status_by_turn_id.get(&turn_id).cloned();
        let assistant_messages_content = assistant_messages
            .iter()
            .map(|message| message.content.trim())
            .filter(|content| !content.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        items.push(ThreadItem::TurnStatus {
            id: format!("turn-status-{}", turn_id_string),
            turn_id: turn_id_string.clone(),
            created_at: turn.updated_at,
            status: turn.status.clone(),
            started_at: turn.started_at,
            updated_at: turn.updated_at,
            custom_status: status_text.clone(),
            status_text: status_text,
            assistant_messages_content: if assistant_messages_content.trim().is_empty() {
                None
            } else {
                Some(assistant_messages_content)
            },
        });

        groups.push(SortableThreadGroup {
            sort_at: header
                .as_ref()
                .map(|h| h.created_at)
                .unwrap_or(turn.started_at),
            group: ThreadGroup {
                key: format!("turn-{}", turn_id_string),
                header,
                items,
            },
        });
    }

    ThreadViewModel {
        groups: merge_groups_with_system_messages(groups, messages),
    }
}

fn build_turn_activity_timeline(
    turn_id: TurnId,
    turn: &SessionTurn,
    tools: Vec<ThreadToolItem>,
    events: Vec<SessionEvent>,
) -> (Vec<ActivityEntry>, Vec<ThreadToolItem>) {
    let turn_id_string = turn_id.0.to_string();
    let mut tool_by_id: HashMap<String, ThreadToolItem> = HashMap::new();
    for tool in tools {
        tool_by_id.insert(tool.tool_call_id.clone(), tool);
    }

    let mut activity: Vec<ActivityEntry> = Vec::new();
    let mut tool_inserted: std::collections::HashSet<String> = std::collections::HashSet::new();

    for event in &events {
        match event.event_type {
            SessionEventType::ThoughtChunk => {
                if !should_render_thought_chunk(event) {
                    continue;
                }
                let fragment = string_value(event.payload_json.get("content_fragment")).unwrap_or_default();
                if fragment.is_empty() {
                    continue;
                }
                if let Some(last) = activity.last_mut() {
                    if matches!(last.kind, ActivityKind::Thought) {
                        if let ThreadItem::Thought { content, .. } = &mut last.item {
                            *content = merge_streaming_text(content, &fragment);
                        }
                        continue;
                    }
                }
                activity.push(ActivityEntry {
                    item: ThreadItem::Thought {
                        id: format!("thought-{}-{}", turn_id_string, event.seq),
                        turn_id: turn_id_string.clone(),
                        created_at: event.created_at,
                        content: fragment,
                    },
                    created_at: event.created_at,
                    kind: ActivityKind::Thought,
                    order_seq: Some(event.seq),
                });
            }
            SessionEventType::ToolCall
            | SessionEventType::ToolCallUpdate
            | SessionEventType::ToolResult => {
                let update = event_update(event);
                let Some(tool_call_id) = extract_tool_call_id(event, update) else {
                    continue;
                };
                let tool = tool_by_id
                    .entry(tool_call_id.clone())
                    .or_insert_with(|| ThreadToolItem {
                        id: format!("tool-{}-{}", turn_id_string, tool_call_id),
                        turn_id: turn_id_string.clone(),
                        tool_call_id: tool_call_id.clone(),
                        created_at: event.created_at,
                        updated_at: event.created_at,
                        tool_kind: "tool".to_string(),
                        title: "Tool".to_string(),
                        status: "pending".to_string(),
                        locations: Vec::new(),
                        input: None,
                        output_text: String::new(),
                        raw: None,
                        updates_seen: 0,
                        has_details: true,
                    });

                apply_tool_update_from_event(tool, event, update);
                if !tool_inserted.contains(&tool_call_id) {
                    activity.push(ActivityEntry {
                        item: ThreadItem::Tool(tool.clone()),
                        created_at: tool.created_at,
                        kind: ActivityKind::Tool,
                        order_seq: Some(event.seq),
                    });
                    tool_inserted.insert(tool_call_id);
                }
            }
            _ => {}
        }
    }

    let fallback_thought = turn.thought_partial.clone().unwrap_or_default();
    if !fallback_thought.trim().is_empty()
        && !activity.iter().any(|entry| matches!(entry.item, ThreadItem::Thought { .. }))
    {
        activity.push(ActivityEntry {
            item: ThreadItem::Thought {
                id: format!("thought-{}-fallback", turn_id_string),
                turn_id: turn_id_string.clone(),
                created_at: turn.updated_at,
                content: fallback_thought,
            },
            created_at: turn.updated_at,
            kind: ActivityKind::Thought,
            order_seq: None,
        });
    }

    let mut remaining_tools: Vec<ThreadToolItem> = tool_by_id
        .values()
        .filter(|tool| !tool_inserted.contains(&tool.tool_call_id))
        .cloned()
        .collect();
    remaining_tools.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    for tool in remaining_tools {
        let created_at = tool.created_at;
        activity.push(ActivityEntry {
            item: ThreadItem::Tool(tool),
            created_at,
            kind: ActivityKind::Tool,
            order_seq: None,
        });
    }

    let tools = tool_by_id.values().cloned().collect::<Vec<_>>();
    (activity, tools)
}

fn build_thread_view_model_from_events(
    events: &[SessionEvent],
    messages: &[MessageItem],
) -> ThreadViewModel {
    #[derive(Clone)]
    struct TurnGroup {
        key: String,
        header: Option<WorkbenchTurnHeader>,
        first_at: DateTime<Utc>,
        tool_items: Vec<ThreadToolItem>,
        tool_by_id: HashMap<String, ThreadToolItem>,
        assistant: Option<ThreadItem>,
        thought_first_at: Option<DateTime<Utc>>,
        thought_last_at: Option<DateTime<Utc>>,
        assistant_first_at: Option<DateTime<Utc>>,
        assistant_complete_at: Option<DateTime<Utc>>,
    }

    let mut user_messages = messages
        .iter()
        .filter(|message| matches!(message.role, MessageRole::User))
        .cloned()
        .collect::<Vec<_>>();
    user_messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let mut assistant_messages = messages
        .iter()
        .filter(|message| matches!(message.role, MessageRole::Assistant))
        .cloned()
        .collect::<Vec<_>>();
    assistant_messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let ensure_tool = |group: &mut TurnGroup, tool_call_id: &str, created_at: DateTime<Utc>| {
        if let Some(existing) = group.tool_by_id.get(tool_call_id).cloned() {
            return existing;
        }
        let tool = ThreadToolItem {
            id: format!("tool-{}", tool_call_id),
            turn_id: group.key.clone(),
            tool_call_id: tool_call_id.to_string(),
            created_at,
            updated_at: created_at,
            tool_kind: "tool".to_string(),
            title: "Tool".to_string(),
            status: "pending".to_string(),
            locations: Vec::new(),
            input: None,
            output_text: String::new(),
            raw: None,
            updates_seen: 0,
            has_details: true,
        };
        group.tool_by_id.insert(tool_call_id.to_string(), tool.clone());
        group.tool_items.push(tool.clone());
        tool
    };

    let mut groups: Vec<SortableThreadGroup> = Vec::new();

    if user_messages.is_empty() {
        let mut user_events = events
            .iter()
            .filter(|event| matches!(event.event_type, SessionEventType::UserMessage))
            .cloned()
            .collect::<Vec<_>>();
        user_events.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        let events_in_range_exclusive = |start: DateTime<Utc>, end: Option<DateTime<Utc>>| {
            events
                .iter()
                .filter(|event| {
                    let t = event.created_at;
                    let end = end.unwrap_or_else(|| DateTime::<Utc>::MAX_UTC);
                    t >= start && t < end
                })
                .cloned()
                .collect::<Vec<_>>()
        };

        if user_events.is_empty() {
            let mut group = TurnGroup {
                key: "no-user-messages".to_string(),
                header: None,
                first_at: events.first().map(|e| e.created_at).unwrap_or_else(Utc::now),
                tool_items: Vec::new(),
                tool_by_id: HashMap::new(),
                assistant: None,
                thought_first_at: None,
                thought_last_at: None,
                assistant_first_at: None,
                assistant_complete_at: None,
            };
            let mut thought = String::new();
            let mut thought_at: Option<DateTime<Utc>> = None;

            for event in events {
                if matches!(event.event_type, SessionEventType::ThoughtChunk) {
                    let fragment =
                        string_value(event.payload_json.get("content_fragment")).unwrap_or_default();
                    if !fragment.is_empty() {
                        thought.push_str(&fragment);
                        if thought_at.is_none() {
                            thought_at = Some(event.created_at);
                        }
                    }
                }

                if let Some(tool_call_id) = tool_call_id_from_event(event) {
                    ensure_tool(&mut group, &tool_call_id, event.created_at);
                }
            }

            let mut items: Vec<ThreadItem> = Vec::new();
            if !thought.trim().is_empty() {
                items.push(ThreadItem::Thought {
                    id: format!("thought-{}", group.key),
                    turn_id: group.key.clone(),
                    created_at: thought_at.unwrap_or(group.first_at),
                    content: thought,
                });
            }
            items.extend(group.tool_items.iter().cloned().map(ThreadItem::Tool));
            if items.is_empty() {
                items.push(ThreadItem::Spacer {
                    id: format!("spacer-{}", group.key),
                    created_at: group.first_at,
                });
            }
            groups.push(SortableThreadGroup {
                sort_at: group.first_at,
                group: ThreadGroup {
                    key: group.key,
                    header: group.header,
                    items,
                },
            });
            return ThreadViewModel {
                groups: merge_groups_with_system_messages(groups, messages),
            };
        }

        for (index, user_event) in user_events.iter().enumerate() {
            let next_user = user_events.get(index + 1);
            let message_id = string_value(user_event.payload_json.get("message_id"))
                .or_else(|| Some(user_event.id.0.to_string()))
                .unwrap_or_else(|| format!("msg-{}", user_event.created_at));

            let attachments = attachments_from_value(user_event.payload_json.get("attachments"));
            let header = WorkbenchTurnHeader {
                id: message_id.clone(),
                content: string_value(user_event.payload_json.get("content")).unwrap_or_default(),
                plain_text: markdown_to_plain_text(
                    &string_value(user_event.payload_json.get("content")).unwrap_or_default(),
                ),
                attachments,
                created_at: user_event.created_at,
            };

            let mut group = TurnGroup {
                key: format!("m-{}", message_id),
                header: Some(header),
                first_at: user_event.created_at,
                tool_items: Vec::new(),
                tool_by_id: HashMap::new(),
                assistant: None,
                thought_first_at: None,
                thought_last_at: None,
                assistant_first_at: None,
                assistant_complete_at: None,
            };

            let events_range = events_in_range_exclusive(
                user_event.created_at,
                next_user.map(|event| event.created_at),
            );

            for event in events_range {
                if event.created_at < group.first_at {
                    group.first_at = event.created_at;
                }

                match event.event_type {
                    SessionEventType::Error => {
                        let output = extract_error_message(&event).unwrap_or_else(|| "Error".to_string());
                        let tool_call_id = format!("error-{}", event.id.0);
                        let mut tool = ensure_tool(&mut group, &tool_call_id, event.created_at);
                        tool.tool_kind = "error".to_string();
                        tool.title = "Error".to_string();
                        tool.status = "failed".to_string();
                        tool.updated_at = event.created_at;
                        tool.updates_seen += 1;
                        tool.input = Some(event.payload_json.clone());
                        tool.output_text = output;
                        tool.raw = Some(serde_json::to_value(event).unwrap_or(Value::Null));
                        group.tool_by_id.insert(tool_call_id, tool);
                    }
                    SessionEventType::AssistantChunk => {
                        let fragment =
                            string_value(event.payload_json.get("content_fragment")).unwrap_or_default();
                        if fragment.is_empty() {
                            continue;
                        }
                        if group.assistant.is_none() {
                            group.assistant = Some(ThreadItem::Assistant {
                                id: format!("assistant-{}", group.key),
                                turn_id: group.key.clone(),
                                created_at: event.created_at,
                                content: String::new(),
                                thought: String::new(),
                                is_complete: false,
                                thought_seconds: None,
                            });
                        }
                        group.assistant_first_at = group.assistant_first_at.or(Some(event.created_at));
                        if let Some(ThreadItem::Assistant { content, .. }) = group.assistant.as_mut()
                        {
                            content.push_str(&fragment);
                        }
                    }
                    SessionEventType::AssistantComplete => {
                        let full = string_value(event.payload_json.get("full_content"))
                            .or_else(|| string_value(event.payload_json.get("content")))
                            .unwrap_or_default();
                        if group.assistant.is_none() {
                            group.assistant = Some(ThreadItem::Assistant {
                                id: format!("assistant-{}", group.key),
                                turn_id: group.key.clone(),
                                created_at: event.created_at,
                                content: String::new(),
                                thought: String::new(),
                                is_complete: false,
                                thought_seconds: None,
                            });
                        }
                        group.assistant_complete_at = Some(event.created_at);
                        if let Some(ThreadItem::Assistant { content, is_complete, .. }) =
                            group.assistant.as_mut()
                        {
                            if !full.is_empty() {
                                *content = full;
                            }
                            *is_complete = true;
                        }
                    }
                    SessionEventType::ThoughtChunk => {
                        if !should_render_thought_chunk(&event) {
                            continue;
                        }
                        let fragment =
                            string_value(event.payload_json.get("content_fragment")).unwrap_or_default();
                        if fragment.is_empty() {
                            continue;
                        }
                        if group.assistant.is_none() {
                            group.assistant = Some(ThreadItem::Assistant {
                                id: format!("assistant-{}", group.key),
                                turn_id: group.key.clone(),
                                created_at: event.created_at,
                                content: String::new(),
                                thought: String::new(),
                                is_complete: false,
                                thought_seconds: None,
                            });
                        }
                        group.thought_first_at = group.thought_first_at.or(Some(event.created_at));
                        group.thought_last_at = Some(event.created_at);
                        if let Some(ThreadItem::Assistant { thought, .. }) = group.assistant.as_mut()
                        {
                            thought.push_str(&fragment);
                        }
                    }
                    SessionEventType::ToolCall
                    | SessionEventType::ToolCallUpdate
                    | SessionEventType::ToolResult => {
                        let update = event_update(&event);
                        let Some(tool_call_id) = tool_call_id_from_event(&event) else {
                            continue;
                        };
                        let mut tool = ensure_tool(&mut group, &tool_call_id, event.created_at);
                        apply_tool_update_from_event(&mut tool, &event, update);
                        group.tool_by_id.insert(tool_call_id, tool);
                    }
                    _ => {}
                }
            }

            let mut items: Vec<ThreadItem> = Vec::new();
            items.extend(group.tool_items.iter().cloned().map(ThreadItem::Tool));
            if let Some(ThreadItem::Assistant { thought, .. }) = group.assistant.as_ref() {
                if !thought.trim().is_empty() {
                    items.push(ThreadItem::Thought {
                        id: format!("thought-{}", group.key),
                        turn_id: group.key.clone(),
                        created_at: group
                            .thought_first_at
                            .or(group.assistant_first_at)
                            .unwrap_or(group.first_at),
                        content: thought.clone(),
                    });
                }
            }
            if let Some(assistant) = group.assistant.clone() {
                let thought_seconds = if let (Some(start), Some(end)) = (
                    group.thought_first_at,
                    group
                        .assistant_first_at
                        .or(group.assistant_complete_at)
                        .or(group.thought_last_at),
                ) {
                    let delta = end.signed_duration_since(start).num_seconds();
                    Some(std::cmp::max(1, delta))
                } else {
                    None
                };
                items.push(match assistant {
                    ThreadItem::Assistant {
                        id,
                        turn_id,
                        created_at,
                        content,
                        thought,
                        is_complete,
                        ..
                    } => ThreadItem::Assistant {
                        id,
                        turn_id,
                        created_at,
                        content,
                        thought,
                        is_complete,
                        thought_seconds,
                    },
                    other => other,
                });
            }
            if items.is_empty() {
                items.push(ThreadItem::Spacer {
                    id: format!("spacer-{}", group.key),
                    created_at: group.first_at,
                });
            }

            groups.push(SortableThreadGroup {
                sort_at: group.first_at,
                group: ThreadGroup {
                    key: group.key,
                    header: group.header,
                    items,
                },
            });
        }

        return ThreadViewModel {
            groups: merge_groups_with_system_messages(groups, messages),
        };
    }

    let events_in_range = |start: DateTime<Utc>, end: Option<DateTime<Utc>>| {
        events
            .iter()
            .filter(|event| {
                let end = end.unwrap_or_else(|| DateTime::<Utc>::MAX_UTC);
                event.created_at >= start && event.created_at <= end
            })
            .cloned()
            .collect::<Vec<_>>()
    };

    for (index, user_message) in user_messages.iter().enumerate() {
        let next_user = user_messages.get(index + 1);
        let message_id = message_id_string(user_message.id)
            .unwrap_or_else(|| format!("msg-{}", user_message.created_at));

        let mut group = TurnGroup {
            key: format!("m-{}", message_id),
            header: Some(WorkbenchTurnHeader {
                id: message_id.clone(),
                content: user_message.content.clone(),
                plain_text: user_message.content.clone(),
                attachments: user_message.attachments.clone(),
                created_at: user_message.created_at,
            }),
            first_at: user_message.created_at,
            tool_items: Vec::new(),
            tool_by_id: HashMap::new(),
            assistant: None,
            thought_first_at: None,
            thought_last_at: None,
            assistant_first_at: None,
            assistant_complete_at: None,
        };

        let assistant_message = assistant_messages.iter().find(|assistant| {
            let ta = assistant.created_at;
            let tu = user_message.created_at;
            if ta <= tu {
                return false;
            }
            if let Some(next_user) = next_user {
                return ta < next_user.created_at;
            }
            true
        });

        let end_at = assistant_message
            .map(|assistant| assistant.created_at)
            .or_else(|| next_user.map(|u| u.created_at));
        let events_range = events_in_range(user_message.created_at, end_at);

        for event in events_range {
            if event.created_at < group.first_at {
                group.first_at = event.created_at;
            }

            match event.event_type {
                SessionEventType::Error => {
                    let output = extract_error_message(&event).unwrap_or_else(|| "Error".to_string());
                    let tool_call_id = format!("error-{}", event.id.0);
                    let mut tool = ensure_tool(&mut group, &tool_call_id, event.created_at);
                    tool.tool_kind = "error".to_string();
                    tool.title = "Error".to_string();
                    tool.status = "failed".to_string();
                    tool.updated_at = event.created_at;
                    tool.updates_seen += 1;
                    tool.input = Some(event.payload_json.clone());
                    tool.output_text = output;
                    tool.raw = Some(serde_json::to_value(event).unwrap_or(Value::Null));
                    group.tool_by_id.insert(tool_call_id, tool);
                }
                SessionEventType::AssistantChunk => {
                    let fragment =
                        string_value(event.payload_json.get("content_fragment")).unwrap_or_default();
                    if fragment.is_empty() {
                        continue;
                    }
                    if group.assistant.is_none() {
                        group.assistant = Some(ThreadItem::Assistant {
                            id: format!("assistant-{}", group.key),
                            turn_id: group.key.clone(),
                            created_at: event.created_at,
                            content: String::new(),
                            thought: String::new(),
                            is_complete: false,
                            thought_seconds: None,
                        });
                    }
                    group.assistant_first_at = group.assistant_first_at.or(Some(event.created_at));
                    if let Some(ThreadItem::Assistant { content, .. }) = group.assistant.as_mut() {
                        content.push_str(&fragment);
                    }
                }
                SessionEventType::AssistantComplete => {
                    let full = string_value(event.payload_json.get("full_content"))
                        .or_else(|| string_value(event.payload_json.get("content")))
                        .unwrap_or_default();
                    if group.assistant.is_none() {
                        group.assistant = Some(ThreadItem::Assistant {
                            id: format!("assistant-{}", group.key),
                            turn_id: group.key.clone(),
                            created_at: event.created_at,
                            content: String::new(),
                            thought: String::new(),
                            is_complete: false,
                            thought_seconds: None,
                        });
                    }
                    group.assistant_complete_at = Some(event.created_at);
                    if let Some(ThreadItem::Assistant { content, is_complete, .. }) =
                        group.assistant.as_mut()
                    {
                        if !full.is_empty() {
                            *content = full;
                        }
                        *is_complete = true;
                    }
                }
                SessionEventType::ThoughtChunk => {
                    if !should_render_thought_chunk(&event) {
                        continue;
                    }
                    let fragment =
                        string_value(event.payload_json.get("content_fragment")).unwrap_or_default();
                    if fragment.is_empty() {
                        continue;
                    }
                    if group.assistant.is_none() {
                        group.assistant = Some(ThreadItem::Assistant {
                            id: format!("assistant-{}", group.key),
                            turn_id: group.key.clone(),
                            created_at: event.created_at,
                            content: String::new(),
                            thought: String::new(),
                            is_complete: false,
                            thought_seconds: None,
                        });
                    }
                    group.thought_first_at = group.thought_first_at.or(Some(event.created_at));
                    group.thought_last_at = Some(event.created_at);
                    if let Some(ThreadItem::Assistant { thought, .. }) = group.assistant.as_mut() {
                        thought.push_str(&fragment);
                    }
                }
                SessionEventType::ToolCall
                | SessionEventType::ToolCallUpdate
                | SessionEventType::ToolResult => {
                    let update = event_update(&event);
                    let Some(tool_call_id) = tool_call_id_from_event(&event) else {
                        continue;
                    };
                    let mut tool = ensure_tool(&mut group, &tool_call_id, event.created_at);
                    apply_tool_update_from_event(&mut tool, &event, update);
                    group.tool_by_id.insert(tool_call_id, tool);
                }
                _ => {}
            }
        }

        if group.assistant.is_none() {
            if let Some(assistant) = assistant_message {
                group.assistant = Some(ThreadItem::Assistant {
                    id: format!("assistant-{}", group.key),
                    turn_id: group.key.clone(),
                    created_at: assistant.created_at,
                    content: assistant.content.clone(),
                    thought: String::new(),
                    is_complete: true,
                    thought_seconds: None,
                });
                group.assistant_complete_at = Some(assistant.created_at);
            }
        } else if let Some(assistant) = assistant_message {
            if let Some(ThreadItem::Assistant { content, is_complete, .. }) = group.assistant.as_mut()
            {
                if content.trim().is_empty() {
                    *content = assistant.content.clone();
                    *is_complete = true;
                    group.assistant_complete_at = Some(assistant.created_at);
                }
            }
        }

        let mut items: Vec<ThreadItem> = Vec::new();
        items.extend(group.tool_items.iter().cloned().map(ThreadItem::Tool));
        if let Some(ThreadItem::Assistant { thought, .. }) = group.assistant.as_ref() {
            if !thought.trim().is_empty() {
                items.push(ThreadItem::Thought {
                    id: format!("thought-{}", group.key),
                    turn_id: group.key.clone(),
                    created_at: group
                        .thought_first_at
                        .or(group.assistant_first_at)
                        .unwrap_or(group.first_at),
                    content: thought.clone(),
                });
            }
        }
        if let Some(assistant) = group.assistant.clone() {
            let thought_seconds = if let (Some(start), Some(end)) = (
                group.thought_first_at,
                group
                    .assistant_first_at
                    .or(group.assistant_complete_at)
                    .or(group.thought_last_at),
            ) {
                let delta = end.signed_duration_since(start).num_seconds();
                Some(std::cmp::max(1, delta))
            } else {
                None
            };
            items.push(match assistant {
                ThreadItem::Assistant {
                    id,
                    turn_id,
                    created_at,
                    content,
                    thought,
                    is_complete,
                    ..
                } => ThreadItem::Assistant {
                    id,
                    turn_id,
                    created_at,
                    content,
                    thought,
                    is_complete,
                    thought_seconds,
                },
                other => other,
            });
        }
        if items.is_empty() {
            items.push(ThreadItem::Spacer {
                id: format!("spacer-{}", group.key),
                created_at: group.first_at,
            });
        }

        groups.push(SortableThreadGroup {
            sort_at: group.first_at,
            group: ThreadGroup {
                key: group.key,
                header: group.header,
                items,
            },
        });
    }

    if groups.is_empty() {
        let mut group = TurnGroup {
            key: "no-user-messages".to_string(),
            header: None,
            first_at: events.first().map(|event| event.created_at).unwrap_or_else(Utc::now),
            tool_items: Vec::new(),
            tool_by_id: HashMap::new(),
            assistant: None,
            thought_first_at: None,
            thought_last_at: None,
            assistant_first_at: None,
            assistant_complete_at: None,
        };
        for event in events {
            if let Some(tool_call_id) = tool_call_id_from_event(event) {
                ensure_tool(&mut group, &tool_call_id, event.created_at);
            }
        }
        let mut items: Vec<ThreadItem> = group.tool_items.iter().cloned().map(ThreadItem::Tool).collect();
        if items.is_empty() {
            items.push(ThreadItem::Spacer {
                id: format!("spacer-{}", group.key),
                created_at: group.first_at,
            });
        }
        groups.push(SortableThreadGroup {
            sort_at: group.first_at,
            group: ThreadGroup {
                key: group.key,
                header: group.header,
                items,
            },
        });
    }

    ThreadViewModel {
        groups: merge_groups_with_system_messages(groups, messages),
    }
}

fn build_system_message_groups(messages: &[MessageItem]) -> Vec<SortableThreadGroup> {
    let mut system_messages = messages
        .iter()
        .filter(|message| matches!(message.role, MessageRole::System))
        .cloned()
        .collect::<Vec<_>>();
    system_messages.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    system_messages
        .into_iter()
        .enumerate()
        .map(|(idx, message)| {
            let id = message_id_string(message.id).unwrap_or_else(|| format!("system-{}", idx));
            SortableThreadGroup {
                sort_at: message.created_at,
                group: ThreadGroup {
                    key: format!("system-{}", id),
                    header: None,
                    items: vec![ThreadItem::Message {
                        id,
                        role: MessageRole::System,
                        content: message.content,
                        attachments: message.attachments,
                        created_at: message.created_at,
                        delivery: Some(message.delivery),
                    }],
                },
            }
        })
        .collect()
}

fn merge_groups_with_system_messages(
    groups: Vec<SortableThreadGroup>,
    messages: &[MessageItem],
) -> Vec<ThreadGroup> {
    let mut combined = groups;
    let system_groups = build_system_message_groups(messages);
    if !system_groups.is_empty() {
        combined.extend(system_groups);
    }
    combined.sort_by(|a, b| {
        let cmp = a.sort_at.cmp(&b.sort_at);
        if cmp != std::cmp::Ordering::Equal {
            return cmp;
        }
        a.group.key.cmp(&b.group.key)
    });
    combined.into_iter().map(|item| item.group).collect()
}

fn build_custom_status_by_turn_id(events: &[SessionEvent]) -> HashMap<TurnId, String> {
    let mut sorted = events.to_vec();
    sorted.sort_by(|a, b| {
        if a.seq != b.seq {
            return a.seq.cmp(&b.seq);
        }
        a.created_at.cmp(&b.created_at)
    });

    let mut notice_by_turn: HashMap<TurnId, (i64, String)> = HashMap::new();
    let mut tools_by_turn: HashMap<TurnId, HashMap<String, (i64, String, Option<String>)>> =
        HashMap::new();

    let mut order: i64 = 0;
    for event in sorted {
        order += 1;
        let Some(turn_id) = event.turn_id else {
            continue;
        };

        if let Some(text) = extract_notice_status_text(&event) {
            notice_by_turn.insert(turn_id, (order, text));
            continue;
        }

        if !matches!(
            event.event_type,
            SessionEventType::ToolCall | SessionEventType::ToolCallUpdate | SessionEventType::ToolResult
        ) {
            continue;
        }

        let update = event_update(&event);
        let Some(tool_call_id) = extract_tool_call_id(&event, update) else {
            continue;
        };
        let status = string_value(update.get("status"))
            .or_else(|| string_value(update.get("tool_status")))
            .or_else(|| string_value(update.get("toolStatus")))
            .unwrap_or_default();
        let text = derive_tool_status_text(update);
        tools_by_turn
            .entry(turn_id)
            .or_default()
            .insert(tool_call_id, (order, status, text));
    }

    let mut out = HashMap::new();
    let all_turn_ids = notice_by_turn
        .keys()
        .cloned()
        .chain(tools_by_turn.keys().cloned())
        .collect::<std::collections::HashSet<_>>();
    for turn_id in all_turn_ids {
        let per_turn_tools = tools_by_turn.get(&turn_id);
        let mut best_tool: Option<(i64, String)> = None;
        if let Some(per_turn_tools) = per_turn_tools {
            for (_tool_id, (order, status, text)) in per_turn_tools.iter() {
                let Some(text) = text.clone() else {
                    continue;
                };
                if !is_active_tool_status(status) {
                    continue;
                }
                if best_tool.is_none() || *order > best_tool.as_ref().unwrap().0 {
                    best_tool = Some((*order, text));
                }
            }
        }
        if let Some((_order, text)) = best_tool {
            out.insert(turn_id, text);
            continue;
        }
        if let Some((_order, text)) = notice_by_turn.get(&turn_id) {
            out.insert(turn_id, text.clone());
        }
    }

    out
}

fn thread_item_id(item: &ThreadItem) -> &str {
    match item {
        ThreadItem::Message { id, .. } => id,
        ThreadItem::Spacer { id, .. } => id,
        ThreadItem::Assistant { id, .. } => id,
        ThreadItem::Thought { id, .. } => id,
        ThreadItem::TurnStatus { id, .. } => id,
        ThreadItem::Tool(item) => item.id.as_str(),
        ThreadItem::ToolGroup { id, .. } => id,
    }
}

fn message_id_string(id: Option<MessageId>) -> Option<String> {
    id.map(|id| id.0.to_string())
}

fn markdown_to_plain_text(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }

    static FENCE_RE: OnceLock<Regex> = OnceLock::new();
    static INLINE_CODE_RE: OnceLock<Regex> = OnceLock::new();
    static IMAGE_RE: OnceLock<Regex> = OnceLock::new();
    static LINK_RE: OnceLock<Regex> = OnceLock::new();
    static LIST_PREFIX_RE: OnceLock<Regex> = OnceLock::new();
    static MULTI_NEWLINE_RE: OnceLock<Regex> = OnceLock::new();

    let fence_re = FENCE_RE.get_or_init(|| Regex::new(r"```[a-zA-Z0-9_-]*\n").unwrap());
    let inline_code_re = INLINE_CODE_RE.get_or_init(|| Regex::new(r"`([^`]*)`").unwrap());
    let image_re = IMAGE_RE.get_or_init(|| Regex::new(r"!\[([^\]]*)\]\([^)]+\)").unwrap());
    let link_re = LINK_RE.get_or_init(|| Regex::new(r"\[([^\]]+)\]\([^)]+\)").unwrap());
    let list_prefix_re = LIST_PREFIX_RE.get_or_init(|| {
        Regex::new(r"^\s*(?:[#>*+-]|\d+\.)\s+").unwrap()
    });
    let multi_newline_re = MULTI_NEWLINE_RE.get_or_init(|| Regex::new(r"\n{3,}").unwrap());

    let mut text = input.replace('\r', "");
    text = fence_re.replace_all(&text, "").to_string();
    text = text.replace("```", "");
    text = inline_code_re.replace_all(&text, "$1").to_string();
    text = image_re.replace_all(&text, "$1").to_string();
    text = link_re.replace_all(&text, "$1").to_string();
    text = text
        .split('\n')
        .map(|line| list_prefix_re.replace(line, "").to_string())
        .collect::<Vec<_>>()
        .join("\n");
    text = multi_newline_re.replace_all(&text, "\n\n").to_string();
    text.trim().to_string()
}

fn event_update(event: &SessionEvent) -> &Value {
    event
        .payload_json
        .get("acp_update")
        .unwrap_or(&event.payload_json)
}

fn extract_notice_status_text(event: &SessionEvent) -> Option<String> {
    if !matches!(event.event_type, SessionEventType::Notice) {
        return None;
    }
    let payload = &event.payload_json;
    let meta = payload
        .get("acp_update")
        .and_then(|value| value.get("_meta").or_else(|| value.get("meta")))
        .or_else(|| payload.get("_meta").or_else(|| payload.get("meta")));
    if let Some(meta) = meta {
        if let Some(text) = pick_first_string(&[
            meta.get("statusText"),
            meta.get("status_text"),
        ]) {
            return Some(text);
        }
    }
    pick_first_string(&[
        payload.get("statusText"),
        payload.get("status_text"),
    ])
}

fn tool_status_verb(kind: &str) -> Option<&'static str> {
    let normalized = kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "search" => Some("Searching"),
        "read" | "read_file" => Some("Reading"),
        "execute" => Some("Running"),
        "write" | "edit" => Some("Writing"),
        _ => None,
    }
}

fn derive_tool_status_text(update: &Value) -> Option<String> {
    let kind = pick_first_string(&[
        update.get("kind"),
        update.get("tool_kind"),
        update.get("toolKind"),
    ])?;
    let verb = tool_status_verb(&kind)?;
    if verb == "Searching" {
        let query = pick_first_string(&[
            update.pointer("/input/query"),
            update.pointer("/input/q"),
        ]);
        if let Some(query) = query {
            return Some(format!("{} {}", verb, query));
        }
    }
    let title = pick_first_string(&[update.get("title")]);
    if let Some(title) = title {
        return Some(format!("{} {}", verb, title));
    }
    Some(verb.to_string())
}

fn is_active_tool_status(status: &str) -> bool {
    let normalized = status.trim().to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "pending" | "queued" | "running" | "in_progress" | "inprogress"
    )
}

fn should_render_thought_chunk(event: &SessionEvent) -> bool {
    let payload = &event.payload_json;
    let meta = payload
        .get("acp_update")
        .and_then(|value| value.get("_meta").or_else(|| value.get("meta")))
        .or_else(|| payload.get("_meta").or_else(|| payload.get("meta")));
    if let Some(meta) = meta {
        if meta.get("heartbeat").and_then(|value| value.as_bool()) == Some(true) {
            return false;
        }
        if is_status_update_meta(meta) {
            return false;
        }
        if let Some(codex) = meta.get("codex") {
            if let Some(reasoning) = pick_first_string(&[
                codex.get("reasoning_kind"),
                codex.get("reasoningKind"),
            ]) {
                if reasoning == "summary" {
                    return false;
                }
            }
        }
    }
    true
}

fn extract_tool_call_id(event: &SessionEvent, update: &Value) -> Option<String> {
    pick_first_string(&[
        event.payload_json.get("tool_call_id"),
        update.get("toolCallId"),
        update.get("tool_call_id"),
        update.pointer("/toolCall/rawInput/call_id"),
        update.pointer("/rawInput/call_id"),
        update.pointer("/raw_input/call_id"),
    ])
}

fn tool_call_id_from_event(event: &SessionEvent) -> Option<String> {
    let update = event_update(event);
    extract_tool_call_id(event, update)
}

fn apply_tool_update_from_event(tool: &mut ThreadToolItem, event: &SessionEvent, update: &Value) {
    tool.updated_at = event.created_at;
    tool.updates_seen += 1;
    tool.raw = Some(event.payload_json.clone());

    if let Some(kind) = pick_first_string(&[
        update.get("kind"),
        update.pointer("/toolCall/kind"),
    ]) {
        tool.tool_kind = kind;
    }

    if let Some(title) = pick_first_string(&[
        update.get("title"),
        update.pointer("/toolCall/title"),
        update.pointer("/toolCall/name"),
    ]) {
        tool.title = title;
    } else if tool.title == "Tool" && !tool.tool_kind.is_empty() {
        tool.title = human_tool_kind(&tool.tool_kind);
    }

    if let Some(status) = pick_first_string(&[
        update.get("status"),
        update.pointer("/toolCall/status"),
    ]) {
        tool.status = normalize_tool_status(&status, &event.event_type);
    } else if matches!(event.event_type, SessionEventType::ToolResult) {
        tool.status = "completed".to_string();
    }

    if let Some(locations) = update.get("locations").and_then(|value| value.as_array()) {
        tool.locations = locations
            .iter()
            .map(|location| ToolLocation {
                path: string_value(location.get("path")),
                range: location.get("range").cloned(),
            })
            .collect();
    }

    if let Some(input) = update
        .get("rawInput")
        .or_else(|| update.pointer("/toolCall/rawInput"))
        .or_else(|| update.pointer("/toolCall/input"))
        .or_else(|| update.get("input"))
        .or_else(|| update.get("input_preview"))
    {
        if !input.is_null() {
            tool.input = Some(input.clone());
        }
    }

    if let Some(output) = extract_tool_output_text(update) {
        tool.output_text = merge_streaming_text(&tool.output_text, &output);
    }

    if tool.input.is_some() || !tool.output_text.trim().is_empty() {
        tool.has_details = true;
    }
}

fn extract_tool_output_text(update: &Value) -> Option<String> {
    if let Some(text) = pick_first_string(&[
        update.get("outputText"),
        update.get("output_text"),
        update.get("output_preview"),
        update.get("result"),
        update.pointer("/rawOutput/aggregated_output"),
        update.pointer("/rawOutput/output"),
    ]) {
        return Some(text);
    }

    let mut parts = Vec::new();
    if let Some(blocks) = update.get("content").and_then(|value| value.as_array()) {
        for block in blocks {
            let content = block.get("content").unwrap_or(block);
            if let Some(text) = content.get("text").and_then(|value| value.as_str()) {
                parts.push(text);
            }
        }
    }
    let joined = parts.join("");
    if joined.trim().is_empty() {
        None
    } else {
        Some(joined.trim().to_string())
    }
}

fn merge_streaming_text(prev: &str, next: &str) -> String {
    let p = prev.trim_end();
    let n = next.trim_end();
    if p.is_empty() {
        return n.to_string();
    }
    if n.is_empty() {
        return p.to_string();
    }
    if n.starts_with(p) {
        return n.to_string();
    }
    if p.starts_with(n) {
        return p.to_string();
    }
    if n.len() >= p.len() {
        n.to_string()
    } else {
        p.to_string()
    }
}

fn pick_first_string(values: &[Option<&Value>]) -> Option<String> {
    for value in values {
        if let Some(text) = value.and_then(|value| value.as_str()) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| value.as_str()).map(|text| text.trim().to_string())
}

fn is_non_tool_status(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    !matches!(
        normalized.as_str(),
        "pending"
            | "queued"
            | "running"
            | "in_progress"
            | "completed"
            | "failed"
            | "error"
            | "ok"
            | "success"
            | "succeeded"
    )
}

fn is_status_update_meta(meta: &Value) -> bool {
    if !meta.is_object() {
        return false;
    }
    let codex = meta.get("codex").unwrap_or(&Value::Null);
    if let Some(reasoning) = pick_first_string(&[
        codex.get("reasoning_kind"),
        codex.get("reasoningKind"),
    ]) {
        if reasoning == "status" {
            return true;
        }
    }

    let status_text = pick_first_string(&[
        meta.get("status_text"),
        meta.get("statusText"),
        meta.get("status_string"),
        meta.get("statusString"),
        codex.get("status_text"),
        codex.get("statusText"),
        codex.get("status_string"),
        codex.get("statusString"),
    ]);
    if status_text.is_some() {
        return true;
    }

    let status_value = pick_first_string(&[meta.get("status"), codex.get("status")]);
    if let Some(status) = status_value {
        if is_non_tool_status(&status) {
            return true;
        }
    }

    false
}

fn normalize_tool_status(status: &str, event_type: &SessionEventType) -> String {
    let normalized = status.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "inprogress" | "in_progress" | "running" => "in_progress".to_string(),
        "pending" | "queued" => "pending".to_string(),
        "completed" | "complete" | "ok" | "succeeded" => "completed".to_string(),
        "failed" | "error" => "failed".to_string(),
        _ => {
            if matches!(event_type, SessionEventType::ToolResult) {
                "completed".to_string()
            } else if normalized.is_empty() {
                "pending".to_string()
            } else {
                normalized
            }
        }
    }
}

fn human_tool_kind(kind: &str) -> String {
    let normalized = kind.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "execute" => "Run Command",
        "search" => "Search",
        "read" | "read_file" => "Read File",
        "edit" | "write" | "apply_patch" => "Edit File",
        "list" | "list_files" => "List Files",
        "fetch" | "http" | "curl" => "Fetch",
        "think" => "Think",
        "error" => "Error",
        "" => "Tool",
        _ => kind,
    }
    .to_string()
}

fn extract_error_message(event: &SessionEvent) -> Option<String> {
    let payload = &event.payload_json;
    let message = pick_first_string(&[
        payload.get("error"),
        payload.get("message"),
        payload.get("detail"),
    ])?;
    let provider = pick_first_string(&[payload.get("provider")]);
    Some(match provider {
        Some(provider) => format!("{}\nprovider: {}", message, provider),
        None => message,
    })
}

fn attachments_from_value(value: Option<&Value>) -> Vec<MessageAttachment> {
    let Some(value) = value else {
        return Vec::new();
    };
    match serde_json::from_value::<Vec<MessageAttachment>>(value.clone()) {
        Ok(items) => items,
        Err(_) => Vec::new(),
    }
}


pub(crate) fn inline_attachment_key(mime_type: &str, data_base64: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    mime_type.hash(&mut hasher);
    data_base64.hash(&mut hasher);
    format!("inline:{:016x}", hasher.finish())
}

pub(crate) fn attachment_cache_key(attachment: &MessageAttachment) -> String {
    match attachment {
        MessageAttachment::ImageRef { blob_id, .. } => blob_id.clone(),
        MessageAttachment::Image {
            mime_type,
            data_base64,
            ..
        } => inline_attachment_key(mime_type, data_base64),
    }
}
