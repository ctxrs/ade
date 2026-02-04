use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub root_path: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_kind: Option<VcsKind>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_worktree_id: Option<WorktreeId>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeBootstrapStatus {
    Success,
    Failed,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VcsKind {
    Git,
    Jj,
    Hg,
    Svn,
    P4,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    pub id: WorktreeId,
    pub workspace_id: WorkspaceId,
    pub root_path: String,
    pub base_commit_sha: String,
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_kind: Option<VcsKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcs_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_status: Option<WorktreeBootstrapStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_timeout_sec: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_log_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_log_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_config_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_config_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_script_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeVcsComputeState {
    Computing,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeVcsBaseResolutionKind {
    ExplicitBase,
    MergeBase,
    #[default]
    WorktreeBase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeVcsTargetSource {
    Explicit,
    MergeQueueOverride,
    MergeQueueConfig,
    DefaultBranch,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeVcsBaseResolution {
    pub kind: WorktreeVcsBaseResolutionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_source: Option<WorktreeVcsTargetSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeVcsSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_additions: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_deletions: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeVcsTouchedFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeVcsTouchedFiles {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<WorktreeVcsTouchedFile>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorktreeVcsGitStatusSummary {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub raw: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<WorktreeVcsTouchedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Invariant: summary counts align with default session diff summary semantics.
pub struct WorktreeVcsSnapshot {
    pub worktree_id: WorktreeId,
    pub rev: i64,
    pub emitted_at_ms: i64,
    pub base_commit_sha: String,
    pub head_commit_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_branch_commit_sha: Option<String>,
    pub base_resolution: WorktreeVcsBaseResolution,
    pub compute_state: WorktreeVcsComputeState,
    #[serde(default)]
    pub summary: WorktreeVcsSummary,
    #[serde(default)]
    pub git_status: WorktreeVcsGitStatusSummary,
    #[serde(default)]
    pub touched_files: WorktreeVcsTouchedFiles,
    #[serde(default)]
    pub schema_version: i64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAttachmentStatus {
    Pending,
    Syncing,
    Ready,
    Error,
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
    pub status: WorkspaceAttachmentStatus,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeAttachmentStatus {
    Ready,
    Stale,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeAttachmentMount {
    pub worktree_id: WorktreeId,
    pub attachment_id: WorkspaceAttachmentId,
    pub mount_abs_path: String,
    pub materialized_id: String,
    pub status: WorktreeAttachmentStatus,
    pub last_sync_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueueEntryStatus {
    Queued,
    Running,
    Passed,
    Failed,
    Conflict,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueuePatchSource {
    Generated,
    Provided,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeQueueEntry {
    pub id: MergeQueueEntryId,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    pub target_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub patch_source: MergeQueuePatchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_commit_sha: Option<String>,
    pub patch_path: String,
    pub patch_size: i64,
    pub status: MergeQueueEntryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_commit_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeQueueRunStatus {
    Running,
    Passed,
    Failed,
    Conflict,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeQueueRun {
    pub id: MergeQueueRunId,
    pub entry_id: MergeQueueEntryId,
    pub status: MergeQueueRunStatus,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_commit_sha: Option<String>,
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
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
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
pub struct SessionMetadata {
    pub id: SessionId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub title: String,
    pub agent_role: String,
    pub status: SessionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_ref: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInvocation {
    pub id: String,
    pub tool_call_id: String,
    pub parent_session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<TurnId>,
    pub requested_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_json: Option<serde_json::Value>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub children: Vec<SubagentInvocationChild>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInvocationChild {
    pub invocation_id: String,
    pub child_session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub position: i64,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    pub prompt_length: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    Running,
    Exited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSession {
    pub id: TerminalId,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    pub cwd: String,
    pub shell: String,
    pub title: String,
    pub status: TerminalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
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
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    #[serde(default)]
    pub turn_sequence: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_seq: Option<i64>,
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub attachments: Vec<MessageAttachment>,
    pub delivery: MessageDelivery,
    pub delivered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub session_id: SessionId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub absolute_path: String,
    pub mime_type: String,
    pub bytes: i64,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_event_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_original_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_original_bytes: Option<i64>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_event_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_original_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_original_bytes: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    pub title: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTaskSummary {
    pub task: Task,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<SessionSummary>,
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
pub struct WorkspaceArchivedPage {
    pub workspace_id: WorkspaceId,
    #[serde(default)]
    pub archived_rev: i64,
    pub tasks: Vec<WorkspaceTaskSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<WorkspaceIndexCursor>,
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
pub struct WorkspaceActiveTaskSummary {
    pub task: Task,
    pub primary_session: SessionSnapshotSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_session_head: Option<SessionHeadSnapshot>,
    #[serde(default)]
    pub sessions: Vec<SessionSnapshotSummary>,
    pub sort_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceActivePage {
    pub tasks: Vec<WorkspaceActiveTaskSummary>,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceActiveSnapshot {
    pub workspace_id: WorkspaceId,
    pub snapshot_rev: i64,
    #[serde(default)]
    pub archived_rev: i64,
    pub active: WorkspaceActivePage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worktree_vcs_snapshots: Vec<WorktreeVcsSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceActiveHeadBatch {
    pub workspace_id: WorkspaceId,
    pub snapshot_rev: i64,
    #[serde(default)]
    pub heads: Vec<SessionHeadSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshotSummary {
    pub session: SessionMetadata,
    pub last_message_at: Option<DateTime<Utc>>,
    pub last_message_preview: Option<String>,
    pub last_event_seq: Option<i64>,
    #[serde(default)]
    pub state_rev: i64,
    #[serde(default)]
    pub activity: SessionActivityState,
    pub unread: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummaryCheckpoint {
    pub session_id: SessionId,
    pub checkpoint_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_seq: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionHeadWindow {
    pub turn_limit: i64,
    pub message_limit: i64,
    pub event_limit: i64,
    pub byte_limit: i64,
    pub turn_count: i64,
    pub message_count: i64,
    pub event_count: i64,
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeadSnapshot {
    pub session: SessionMetadata,
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
    pub state_rev: i64,
    #[serde(default)]
    pub activity: SessionActivityState,
    pub has_more_turns: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_cursor: Option<i64>,
    #[serde(default)]
    pub has_more_history: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_checkpoint: Option<SessionSummaryCheckpoint>,
    #[serde(default)]
    pub head_window: SessionHeadWindow,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_checkpoint: Option<SessionSummaryCheckpoint>,
    #[serde(default)]
    pub head_window: SessionHeadWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionGitStatusSummary {
    pub summary_line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_status: Option<SessionGitStatusSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub summary: SessionSnapshotSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<SessionHeadSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<SessionState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeadDelta {
    pub session_id: SessionId,
    pub last_event_seq: i64,
    #[serde(default)]
    pub state_rev: i64,
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
pub struct WorktreeBootstrapNotice {
    pub worktree_id: WorktreeId,
    pub worktree_root: String,
    pub status: WorktreeBootstrapStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceActiveSnapshotEvent {
    Ready {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        #[serde(default)]
        archived_rev: i64,
    },
    ActiveTaskUpsert {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task: Box<WorkspaceActiveTaskSummary>,
    },
    ActiveTaskDelete {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        task_id: TaskId,
    },
    SessionSummary {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        summary: Box<SessionSnapshotSummary>,
    },
    SessionHeadDelta {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        delta: Box<SessionHeadDelta>,
    },
    SessionHeadReset {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        head: Box<SessionHeadSnapshot>,
    },
    SessionGap {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        session_id: SessionId,
        after_seq: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    WorktreeBootstrap {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        notice: WorktreeBootstrapNotice,
    },
    WorktreeVcsSnapshot {
        workspace_id: WorkspaceId,
        snapshot_rev: i64,
        snapshot: Box<WorktreeVcsSnapshot>,
    },
    ArchivedTaskUpsert {
        workspace_id: WorkspaceId,
        archived_rev: i64,
        task: Box<WorkspaceTaskSummary>,
    },
    ArchivedTaskDelete {
        workspace_id: WorkspaceId,
        archived_rev: i64,
        task_id: TaskId,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceActiveSnapshotStreamMessage {
    Snapshot {
        rev: i64,
        active_snapshot: WorkspaceActiveSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        active_heads: Option<WorkspaceActiveHeadBatch>,
    },
    Event {
        rev: i64,
        event: Box<WorkspaceActiveSnapshotEvent>,
    },
    HeadsBatch {
        rev: i64,
        snapshot_rev: i64,
        #[serde(default)]
        deltas: Vec<SessionHeadDelta>,
    },
    ResetRequired {
        latest_rev: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceActiveSnapshotSessionSubscription {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_seq: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceActiveSnapshotSubscribeScope {
    Active,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceActiveSnapshotClientMessage {
    Subscribe {
        #[serde(default)]
        session_ids: Vec<SessionId>,
        #[serde(default)]
        sessions: Vec<WorkspaceActiveSnapshotSessionSubscription>,
        #[serde(default)]
        task_ids: Vec<TaskId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        foreground_task_id: Option<TaskId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<WorkspaceActiveSnapshotSubscribeScope>,
        #[serde(default, skip_serializing_if = "is_false")]
        include_active_heads: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventType {
    Init,
    UserMessage,
    InputQueued,
    TurnQueued,
    TurnStarted,
    TurnFinished,
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
    ArtifactsSet,
    Done,
    InterruptRequested,
    TurnInterrupted,
    MessageQueueAdded,
    MessageQueueUpdated,
    MessageQueueRemoved,
    MessageQueuePromoted,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionEvent {
    pub seq: i64,
    pub id: SessionEventId,
    pub session_id: SessionId,
    pub run_id: Option<RunId>,
    pub turn_id: Option<TurnId>,
    pub event_type: SessionEventType,
    pub payload_json: serde_json::Value,
    #[serde(default)]
    pub transient: bool,
    pub created_at: DateTime<Utc>,
}

impl Serialize for SessionEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("SessionEvent", 9)?;
        if self.transient {
            state.serialize_field("seq", &Option::<i64>::None)?;
        } else {
            state.serialize_field("seq", &self.seq)?;
        }
        state.serialize_field("id", &self.id)?;
        state.serialize_field("session_id", &self.session_id)?;
        state.serialize_field("run_id", &self.run_id)?;
        state.serialize_field("turn_id", &self.turn_id)?;
        state.serialize_field("event_type", &self.event_type)?;
        state.serialize_field("payload_json", &self.payload_json)?;
        if self.transient {
            state.serialize_field("transient", &true)?;
        }
        state.serialize_field("created_at", &self.created_at)?;
        state.end()
    }
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
