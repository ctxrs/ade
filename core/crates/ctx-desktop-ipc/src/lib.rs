use ctx_client::BlobUploadResp;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopConnectionKind {
    None,
    Local,
    Ssh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopConnectionIntent {
    AutoLocalBootstrap,
    ExplicitLocal,
    ExplicitRemote,
    ExplicitDisconnected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DesktopRemoteDaemonUpdateState {
    Pending,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopConnectionInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub base_url: Option<String>,
    #[ts(optional, as = "Option<_>")]
    pub intent: DesktopConnectionIntent,
    pub kind: DesktopConnectionKind,
    #[ts(optional, as = "Option<_>")]
    pub local_auto_bootstrap_allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub remote_data_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub remote_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub remote_update_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub remote_update_state: Option<DesktopRemoteDaemonUpdateState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct SshConnectReq {
    pub host: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub password_once: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub remote_data_dir: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub remote_port: Option<u16>,
    #[serde(default = "default_true")]
    pub start_remote: bool,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub user: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSshConnectPollReq {
    #[serde(default)]
    pub consume: bool,
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSshConnectJobStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional = nullable)]
    pub created_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub info: Option<DesktopConnectionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub phase: Option<String>,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional = nullable)]
    pub updated_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopDaemonRequest {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub body: Option<String>,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopHttpResponse {
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub content_type: Option<String>,
    pub status: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopRemoteDaemonUpdateReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub channel: Option<String>,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopRemoteDaemonUpdateResp {
    pub message: String,
    pub updated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopLinuxSandboxEnsureResp {
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopLocalLinuxSandboxEnsureReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub admin_password_once: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopRemoteLinuxSandboxEnsureReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub admin_password_once: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateAttemptStageResp {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional = nullable)]
    pub finished_at_ms: Option<u64>,
    pub result: String,
    pub stage: String,
    #[ts(type = "number")]
    pub started_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateAttemptResp {
    pub attempt_id: String,
    pub channel: String,
    pub current_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(type = "number", optional = nullable)]
    pub finished_at_ms: Option<u64>,
    pub result: String,
    pub stages: Vec<DesktopAppUpdateAttemptStageResp>,
    #[ts(type = "number")]
    pub started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub target_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateCheckReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub channel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateCheckResp {
    pub available: bool,
    pub configured: bool,
    pub current_version: String,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub last_attempt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub message: Option<String>,
    pub phase: String,
    pub restart_required: bool,
    pub staged: bool,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateStateResp {
    pub available: bool,
    pub configured: bool,
    pub current_version: String,
    pub endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub last_attempt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub message: Option<String>,
    pub phase: String,
    pub restart_required: bool,
    pub staged: bool,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateApplyReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub channel: Option<String>,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub download_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppUpdateApplyResp {
    pub applied: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub latest_version: Option<String>,
    pub message: String,
    pub needs_restart: bool,
    pub up_to_date: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopAppRestartResp {
    pub message: String,
    pub requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopStorageBatchOp {
    Delete {
        key: String,
    },
    Set {
        key: String,
        #[ts(type = "unknown")]
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopUiStateResetReason {
    SchemaMismatch,
    InvalidUiStateDb,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopStorageNotice {
    UiStateReset { reason: DesktopUiStateResetReason },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopStorageGetReq {
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopStorageBatchReq {
    pub ops: Vec<DesktopStorageBatchOp>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopWebviewSurface {
    Main,
    Workbench,
    Launcher,
    Settings,
    FilePreview,
    WorkspaceSetup,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopWebviewRecoveryTriggerKind {
    NativeProcessTermination,
    HeartbeatTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopWebviewRecoveryAction {
    Noop,
    Reload,
    Recreate,
    PromptRestart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopWebviewRecoveryDaemonHealth {
    Unknown,
    Ok,
    Down,
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopWebviewRecoverySuppressionReason {
    RecoveryInProgress,
    WindowNotVisible,
    WindowNotFocused,
    StartupGrace,
    NoHeartbeatYet,
    DaemonDown,
    DaemonMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
pub struct DesktopWebviewRecoveryIncident {
    pub incident_id: String,
    pub window_label: String,
    pub window_surface: DesktopWebviewSurface,
    pub route: String,
    pub trigger_kind: DesktopWebviewRecoveryTriggerKind,
    pub action: DesktopWebviewRecoveryAction,
    pub daemon_health: DesktopWebviewRecoveryDaemonHealth,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub suppression_reason: Option<DesktopWebviewRecoverySuppressionReason>,
    #[ts(type = "number")]
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopWebviewRecoveryHeartbeatReq {
    pub route: String,
    #[serde(default)]
    pub document_visible: bool,
    #[serde(default)]
    pub window_focused: bool,
    #[serde(default)]
    pub startup_ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopWebviewRecoveryFaultKind {
    NativeProcessTermination,
    HeartbeatTimeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopWebviewRecoveryFaultReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub window_label: Option<String>,
    pub kind: DesktopWebviewRecoveryFaultKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopWebviewRecoveryWindowAutomationSnapshot {
    pub window_label: String,
    pub window_surface: DesktopWebviewSurface,
    pub route: String,
    #[serde(default)]
    #[ts(type = "number", optional = nullable)]
    pub last_heartbeat_at_ms: Option<u64>,
    #[serde(default)]
    #[ts(type = "number", optional = nullable)]
    pub startup_completed_at_ms: Option<u64>,
    pub recovery_in_progress: bool,
    pub consecutive_recovery_count: u32,
    pub pending_heartbeat_timeout: bool,
    pub daemon_health: DesktopWebviewRecoveryDaemonHealth,
    pub recent_incident_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopWebviewRecoveryAutomationSnapshot {
    pub windows: Vec<DesktopWebviewRecoveryWindowAutomationSnapshot>,
    pub pending_incident_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopDeepLinkToken {
    #[ts(type = "number")]
    pub expires_at_ms: u64,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSshHost {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSshTestReq {
    pub host: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub password_once: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopRemotePrewarmReq {
    pub host: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub remote_data_dir: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub remote_port: Option<u16>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSshPathReq {
    pub host: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub path: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSshPathEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopGitBranchReq {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopGitCloneReq {
    pub dest_parent: String,
    pub repo_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopEditorTarget {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "vscode")]
    VsCode,
    #[serde(rename = "vscode_insiders")]
    VsCodeInsiders,
    #[serde(rename = "cursor")]
    Cursor,
    #[serde(rename = "windsurf")]
    Windsurf,
    #[serde(rename = "antigravity")]
    Antigravity,
    #[serde(rename = "idea")]
    Idea,
    #[serde(rename = "pycharm")]
    Pycharm,
    #[serde(rename = "xcode")]
    Xcode,
    #[serde(rename = "android_studio")]
    AndroidStudio,
    #[serde(rename = "custom")]
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DesktopEditorSettings {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub custom_command: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub remote_authority: Option<String>,
    pub target: DesktopEditorTarget,
}

impl Default for DesktopEditorSettings {
    fn default() -> Self {
        Self {
            custom_command: None,
            remote_authority: None,
            target: DesktopEditorTarget::System,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopOpenFileReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub col: Option<u32>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub line: Option<u32>,
    pub path: String,
    pub worktree_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopOpenPathReq {
    #[serde(default)]
    #[ts(optional = nullable)]
    pub col: Option<u32>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub line: Option<u32>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopReadFileResp {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopReadBinaryFileResp {
    pub bytes: Vec<u8>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSaveTextFileReq {
    pub contents: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub suggested_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopRestartLocalDaemonReq {
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopUploadBlobReq {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopCodexLoginRelayReq {
    pub callback_url: String,
    pub completion_token: String,
    pub login_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesktopMenuItemStateUpdate {
    #[ts(optional = nullable)]
    pub checked: Option<bool>,
    #[ts(optional = nullable)]
    pub enabled: Option<bool>,
    pub id: String,
    #[ts(optional = nullable)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSetMenuStateReq {
    pub items: Vec<DesktopMenuItemStateUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSetOpenWorkspacesReq {
    pub workspace_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopOpenWorkspaceInNewWindowReq {
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopRecordWorkspaceVisitReq {
    pub workspace_id: String,
    pub workspace_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopDockRecentLocalWorkspace {
    pub label: String,
    pub root_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSetDockRecentLocalWorkspacesReq {
    pub entries: Vec<DesktopDockRecentLocalWorkspace>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopTitlebarColor {
    #[ts(optional = nullable)]
    pub a: Option<f64>,
    pub b: f64,
    pub g: f64,
    pub r: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSetWindowTitleReq {
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNotificationPermission {
    Default,
    Granted,
    Denied,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DesktopNotificationKind {
    TurnCompleted,
    TurnFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopShowSystemNotificationReq {
    pub kind: DesktopNotificationKind,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub body: Option<String>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub session_id: Option<String>,
    pub task_id: String,
    pub title: String,
    pub workspace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct DesktopSyncWorkspaceAttentionReq {
    pub has_unread_error: bool,
    pub unread_primary_task_count: u32,
    pub workspace_id: String,
}

pub fn typescript_declarations() -> String {
    let mut out = String::from("// Generated by ctx-desktop-ipc. Do not edit by hand.\n\n");
    push_decl::<BlobUploadResp>(&mut out);
    push_decl::<DesktopConnectionKind>(&mut out);
    push_decl::<DesktopConnectionIntent>(&mut out);
    push_decl::<DesktopRemoteDaemonUpdateState>(&mut out);
    push_decl::<DesktopConnectionInfo>(&mut out);
    push_decl::<SshConnectReq>(&mut out);
    push_decl::<DesktopSshConnectPollReq>(&mut out);
    push_decl::<DesktopSshConnectJobStatus>(&mut out);
    push_decl::<DesktopDaemonRequest>(&mut out);
    push_decl::<DesktopHttpResponse>(&mut out);
    push_decl::<DesktopRemoteDaemonUpdateReq>(&mut out);
    push_decl::<DesktopRemoteDaemonUpdateResp>(&mut out);
    push_decl::<DesktopLinuxSandboxEnsureResp>(&mut out);
    push_decl::<DesktopLocalLinuxSandboxEnsureReq>(&mut out);
    push_decl::<DesktopRemoteLinuxSandboxEnsureReq>(&mut out);
    push_decl::<DesktopAppUpdateAttemptStageResp>(&mut out);
    push_decl::<DesktopAppUpdateAttemptResp>(&mut out);
    push_decl::<DesktopAppUpdateCheckReq>(&mut out);
    push_decl::<DesktopAppUpdateCheckResp>(&mut out);
    push_decl::<DesktopAppUpdateStateResp>(&mut out);
    push_decl::<DesktopAppUpdateApplyReq>(&mut out);
    push_decl::<DesktopAppUpdateApplyResp>(&mut out);
    push_decl::<DesktopAppRestartResp>(&mut out);
    push_decl::<DesktopStorageBatchOp>(&mut out);
    push_decl::<DesktopUiStateResetReason>(&mut out);
    push_decl::<DesktopStorageNotice>(&mut out);
    push_decl::<DesktopStorageGetReq>(&mut out);
    push_decl::<DesktopStorageBatchReq>(&mut out);
    push_decl::<DesktopWebviewSurface>(&mut out);
    push_decl::<DesktopWebviewRecoveryTriggerKind>(&mut out);
    push_decl::<DesktopWebviewRecoveryAction>(&mut out);
    push_decl::<DesktopWebviewRecoveryDaemonHealth>(&mut out);
    push_decl::<DesktopWebviewRecoverySuppressionReason>(&mut out);
    push_decl::<DesktopWebviewRecoveryIncident>(&mut out);
    push_decl::<DesktopWebviewRecoveryHeartbeatReq>(&mut out);
    push_decl::<DesktopWebviewRecoveryFaultKind>(&mut out);
    push_decl::<DesktopWebviewRecoveryFaultReq>(&mut out);
    push_decl::<DesktopWebviewRecoveryWindowAutomationSnapshot>(&mut out);
    push_decl::<DesktopWebviewRecoveryAutomationSnapshot>(&mut out);
    push_decl::<DesktopDeepLinkToken>(&mut out);
    push_decl::<DesktopSshHost>(&mut out);
    push_decl::<DesktopSshTestReq>(&mut out);
    push_decl::<DesktopRemotePrewarmReq>(&mut out);
    push_decl::<DesktopSshPathReq>(&mut out);
    push_decl::<DesktopSshPathEntry>(&mut out);
    push_decl::<DesktopGitBranchReq>(&mut out);
    push_decl::<DesktopGitCloneReq>(&mut out);
    push_decl::<DesktopEditorTarget>(&mut out);
    push_decl::<DesktopEditorSettings>(&mut out);
    push_decl::<DesktopOpenFileReq>(&mut out);
    push_decl::<DesktopOpenPathReq>(&mut out);
    push_decl::<DesktopReadFileResp>(&mut out);
    push_decl::<DesktopReadBinaryFileResp>(&mut out);
    push_decl::<DesktopSaveTextFileReq>(&mut out);
    push_decl::<DesktopRestartLocalDaemonReq>(&mut out);
    push_decl::<DesktopUploadBlobReq>(&mut out);
    push_decl::<DesktopCodexLoginRelayReq>(&mut out);
    push_decl::<DesktopMenuItemStateUpdate>(&mut out);
    push_decl::<DesktopSetMenuStateReq>(&mut out);
    push_decl::<DesktopSetOpenWorkspacesReq>(&mut out);
    push_decl::<DesktopOpenWorkspaceInNewWindowReq>(&mut out);
    push_decl::<DesktopRecordWorkspaceVisitReq>(&mut out);
    push_decl::<DesktopDockRecentLocalWorkspace>(&mut out);
    push_decl::<DesktopSetDockRecentLocalWorkspacesReq>(&mut out);
    push_decl::<DesktopTitlebarColor>(&mut out);
    push_decl::<DesktopSetWindowTitleReq>(&mut out);
    push_decl::<DesktopNotificationPermission>(&mut out);
    push_decl::<DesktopNotificationKind>(&mut out);
    push_decl::<DesktopShowSystemNotificationReq>(&mut out);
    push_decl::<DesktopSyncWorkspaceAttentionReq>(&mut out);
    out
}

fn push_decl<T: TS>(out: &mut String) {
    out.push_str("export ");
    out.push_str(&T::decl());
    out.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::typescript_declarations;

    #[test]
    fn generated_typescript_is_current() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let output_path = manifest_dir.join("../../apps/web/src/generated/desktop-ipc.ts");
        let existing = fs::read_to_string(&output_path)
            .unwrap_or_else(|err| panic!("reading {}: {err}", output_path.display()));
        assert_eq!(
            existing,
            typescript_declarations(),
            "desktop IPC TypeScript bindings are stale: regenerate {}",
            output_path.display()
        );
    }
}
