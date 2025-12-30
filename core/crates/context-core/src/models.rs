use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root_path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub exec_plan_id: Option<String>,
    #[serde(default)]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_seen_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_assistant_message_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_active_session: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub label: String,
    pub status: TrackStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    pub id: WorktreeId,
    pub workspace_id: WorkspaceId,
    pub root_path: String,
    pub base_commit_sha: String,
    pub git_branch: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAttachmentKind {
    ReferenceRepo,
    DocMirror,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentMode {
    Ro,
    Rw,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentUpdatePolicy {
    Manual,
    OnOpen,
    Scheduled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAttachment {
    pub id: WorkspaceAttachmentId,
    pub workspace_id: WorkspaceId,
    pub kind: WorkspaceAttachmentKind,
    pub name: String,
    pub source: String,
    pub revision: Option<String>,
    pub subpath: Option<String>,
    pub mount_relpath: String,
    pub mode: AttachmentMode,
    pub update_policy: AttachmentUpdatePolicy,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackAttachmentStatus {
    Ready,
    Stale,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackAttachmentMount {
    pub track_id: TrackId,
    pub attachment_id: WorkspaceAttachmentId,
    pub mount_abs_path: String,
    pub materialized_id: String,
    pub status: TrackAttachmentStatus,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub track_id: TrackId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub provider_id: String,
    pub model_id: String,
    pub title: String,
    pub agent_role: String,
    pub status: SessionStatus,
    pub provider_session_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDelivery {
    Immediate,
    Queued,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageAttachment {
    Image {
        mime_type: String,
        data_base64: String,
        #[serde(default)]
        name: Option<String>,
    },
    ImageRef {
        blob_id: String,
        mime_type: String,
        #[serde(default)]
        name: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub track_id: TrackId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    #[serde(default)]
    pub turn_sequence: Option<i64>,
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
    pub delivery: MessageDelivery,
    pub delivered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionTurnStatus {
    Queued,
    Running,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionActivityState {
    pub is_working: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_status: Option<SessionTurnStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTurn {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub user_message_id: Option<MessageId>,
    pub status: SessionTurnStatus,
    pub start_seq: Option<i64>,
    pub end_seq: Option<i64>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub assistant_partial: Option<String>,
    pub thought_partial: Option<String>,
    pub metrics_json: Option<serde_json::Value>,
    pub tool_total: i64,
    pub tool_pending: i64,
    pub tool_running: i64,
    pub tool_completed: i64,
    pub tool_failed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTurnTool {
    pub session_id: SessionId,
    pub tool_call_id: String,
    pub turn_id: TurnId,
    pub tool_kind: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub input_json: Option<serde_json::Value>,
    pub output_text: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionTurnToolSummary {
    pub session_id: SessionId,
    pub tool_call_id: String,
    pub turn_id: TurnId,
    pub tool_kind: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub track_id: TrackId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub provider_id: String,
    pub model_id: String,
    pub title: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSummary {
    pub track: Track,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTaskSummary {
    pub task: Task,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_ids: Vec<String>,
    #[serde(default)]
    pub tracks: Vec<TrackSummary>,
    pub sort_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceIndexCursor {
    pub sort_at: DateTime<Utc>,
    pub task_id: TaskId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceIndexPage {
    pub workspace_id: WorkspaceId,
    pub snapshot_rev: i64,
    pub tasks: Vec<WorkspaceTaskSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<WorkspaceIndexCursor>,
    pub total_active: i64,
    pub total_archived: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceIndexEvent {
    Ready {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
    },
    TaskUpsert {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task: Box<WorkspaceTaskSummary>,
    },
    TaskDelete {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task_id: TaskId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCatchupCursor {
    pub sort_at: DateTime<Utc>,
    pub task_id: TaskId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCatchupSummary {
    pub session: Session,
    pub last_message_at: Option<DateTime<Utc>>,
    pub last_message_preview: Option<String>,
    pub last_event_seq: Option<i64>,
    #[serde(default)]
    pub activity: SessionActivityState,
    pub unread: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackDiffSummary {
    pub file_count: i64,
    pub line_additions: i64,
    pub line_deletions: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackDiffSummaryResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<TrackDiffSummary>,
    pub too_large: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCatchupTrackSummary {
    pub track: Track,
    pub primary_session_id: Option<SessionId>,
    #[serde(default)]
    pub sessions: Vec<SessionCatchupSummary>,
    pub diff_summary: Option<TrackDiffSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCatchupTaskSummary {
    pub task: Task,
    #[serde(default)]
    pub tracks: Vec<WorkspaceCatchupTrackSummary>,
    pub sort_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCatchupPage {
    pub tasks: Vec<WorkspaceCatchupTaskSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<WorkspaceCatchupCursor>,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCatchupSnapshot {
    pub workspace_id: WorkspaceId,
    pub snapshot_rev: i64,
    pub active: WorkspaceCatchupPage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<WorkspaceCatchupPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHead {
    pub session: Session,
    #[serde(default)]
    pub turns: Vec<SessionTurn>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_summaries: Vec<SessionTurnToolSummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<SessionEvent>,
    #[serde(default)]
    pub messages: Vec<Message>,
    pub last_event_seq: i64,
    #[serde(default)]
    pub activity: SessionActivityState,
    pub has_more_turns: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeadDelta {
    pub session_id: SessionId,
    pub last_event_seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<SessionEvent>,
    pub turn: Option<SessionTurn>,
    pub message: Option<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHistoryPage {
    pub session_id: SessionId,
    #[serde(default)]
    pub turns: Vec<SessionTurn>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEventsPage {
    pub session_id: SessionId,
    #[serde(default)]
    pub events: Vec<SessionEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<i64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceCatchupEvent {
    Ready {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
    },
    TaskUpsert {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task: WorkspaceCatchupTaskSummary,
    },
    TaskDelete {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task_id: TaskId,
    },
    TrackUpsert {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        track: WorkspaceCatchupTrackSummary,
    },
    SessionSummary {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        summary: SessionCatchupSummary,
    },
    SessionHeadDelta {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        delta: Box<SessionHeadDelta>,
    },
    SessionGap {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        session_id: SessionId,
        after_seq: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCatchupSessionSubscription {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceCatchupClientMessage {
    Subscribe {
        #[serde(default)]
        session_ids: Vec<SessionId>,
        #[serde(default)]
        sessions: Vec<WorkspaceCatchupSessionSubscription>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    Init,
    UserMessage,
    InputQueued,
    AuthRequired,
    Notice,
    AssistantChunk,
    ThoughtChunk,
    AssistantComplete,
    AssistantMessageInserted,
    ToolCall,
    ToolCallUpdate,
    ToolResult,
    Plan,
    Done,
    InterruptRequested,
    TurnInterrupted,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub seq: i64,
    pub id: SessionEventId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub event_type: SessionEventType,
    pub payload_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileConnectionProfile {
    pub id: ConnectionProfileId,
    pub label: String,
    pub base_url: String,
    pub token_prefix: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileDeviceRegistration {
    pub id: MobileDeviceId,
    pub profile_id: ConnectionProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}
