use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;

mod mobile;
mod session_events;
mod workspace_activity;

pub use mobile::*;
pub use session_events::*;
pub use workspace_activity::*;

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

fn is_true(v: &bool) -> bool {
    *v
}

fn default_true() -> bool {
    true
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
    pub bootstrap_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_script_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SandboxSubstrate {
    NativeContainer,
    SharedVmContainer,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxGuestPlatform {
    #[default]
    Linux,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxIsolationKind {
    #[default]
    Container,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxGuestRuntime {
    #[default]
    Ubuntu,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SandboxGuestIdentity {
    #[serde(default)]
    pub platform: SandboxGuestPlatform,
    #[serde(default)]
    pub isolation_kind: SandboxIsolationKind,
    #[serde(default)]
    pub runtime: SandboxGuestRuntime,
}

impl SandboxGuestIdentity {
    pub const fn linux_container_ubuntu() -> Self {
        Self {
            platform: SandboxGuestPlatform::Linux,
            isolation_kind: SandboxIsolationKind::Container,
            runtime: SandboxGuestRuntime::Ubuntu,
        }
    }
}

pub fn sandbox_instance_id_for_workspace(workspace_id: WorkspaceId) -> SandboxInstanceId {
    SandboxInstanceId(workspace_id.0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SandboxProfile {
    #[default]
    Standard,
    Strict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxBinding {
    pub worktree_id: WorktreeId,
    pub workspace_id: WorkspaceId,
    pub sandbox_instance_id: SandboxInstanceId,
    #[serde(alias = "runtime_family")]
    pub substrate: SandboxSubstrate,
    #[serde(default)]
    pub guest_identity: SandboxGuestIdentity,
    #[serde(default)]
    pub profile: SandboxProfile,
    pub live_workspace_root: String,
    pub live_worktree_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_settings_json: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_name: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "host_projection_root"
    )]
    pub host_materialization_root: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl SandboxBinding {
    pub fn expected_sandbox_instance_id(&self) -> SandboxInstanceId {
        sandbox_instance_id_for_workspace(self.workspace_id)
    }

    pub fn uses_workspace_mapped_sandbox_instance(&self) -> bool {
        self.sandbox_instance_id == self.expected_sandbox_instance_id()
    }
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
pub enum WorktreeVcsFreshness {
    #[default]
    Unknown,
    Refreshing,
    Fresh,
    Stale,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffUnavailableReason {
    NoRepo,
    NoTargetBranch,
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
    PrimaryBranchConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeVcsTouchedFilesState {
    #[default]
    NotLoaded,
    Loading,
    Ready,
    Stale,
    Error,
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
    pub touched_files_state: WorktreeVcsTouchedFilesState,
    #[serde(default)]
    pub freshness: WorktreeVcsFreshness,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<DiffUnavailableReason>,
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    #[default]
    Host,
    Sandbox,
}

impl<'de> Deserialize<'de> for ExecutionEnvironment {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let trimmed = raw.trim();
        if trimmed.eq_ignore_ascii_case("host") {
            return Ok(Self::Host);
        }
        if trimmed.eq_ignore_ascii_case("sandbox") || trimmed.starts_with("container_") {
            return Ok(Self::Sandbox);
        }
        Err(serde::de::Error::custom(format!(
            "unknown execution environment: {trimmed}"
        )))
    }
}

impl ExecutionEnvironment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Sandbox => "sandbox",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub task_id: TaskId,
    pub workspace_id: WorkspaceId,
    pub worktree_id: WorktreeId,
    pub execution_environment: ExecutionEnvironment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
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
    pub execution_environment: ExecutionEnvironment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionTurnStatus {
    Queued,
    Running,
    Completed,
    Interrupted,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_tool_name: Option<String>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Option<String>,
    pub input_json: Option<serde_json::Value>,
    pub output_text: Option<String>,
    pub order_seq: i64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_tool_name: Option<String>,
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_preview: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
    pub order_seq: i64,
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
    pub execution_environment: ExecutionEnvironment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
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
