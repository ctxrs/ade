use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use ctx_core::ids::{SessionId, TaskId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, MessageAttachment, MessageDelivery, Session,
    WorkspaceAttachmentKind, WorkspaceIndexCursor,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub version: String,
    pub daemon_version: String,
    pub pid: i64,
    pub data_root: String,
    pub daemon_url: String,
    pub auth_required: bool,
    pub compatibility: HealthCompatibility,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HealthCompatibility {
    pub desktop_exact_version: String,
    pub mobile_api_min: i64,
    pub mobile_api_max: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerfMetricKind {
    Histogram,
    Counter,
    Gauge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientTelemetryMetric {
    pub name: String,
    pub kind: PerfMetricKind,
    pub unit: String,
    pub value: f64,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientTelemetryBatch {
    pub events: Vec<ClientTelemetryMetric>,
}

#[derive(Debug, Clone, Default)]
pub struct TelemetrySummaryParams {
    pub metric: Option<String>,
    pub run_id: Option<String>,
    pub window_ms: Option<u64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnvTarget {
    Worktree,
    Local,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTaskRequest {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub create_default_session: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateTaskTitleRequest<'a> {
    pub title: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequest {
    pub provider_id: String,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relationship: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_target: Option<EnvTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTerminalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<WorktreeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PostMessageRequest {
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<MessageDelivery>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSessionModelRequest {
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionInfo {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_activity: String,
    pub url: String,
    pub viewport: WebSessionViewport,
    pub fps: u32,
    pub viewers: u32,
    pub stream_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobUploadResp {
    pub blob_id: String,
    pub sha256: String,
    pub bytes: i64,
    pub mime_type: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetSessionModeRequest {
    pub mode_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthenticateProviderRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AskUserQuestionRequest {
    pub tool_call_id: String,
    pub outcome: AskUserQuestionOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserQuestionOutcome {
    Submitted,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDiffApplyRequest {
    pub action: String,
    pub patch: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDiffResponse {
    pub diff: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDiffSummaryResponse {
    #[serde(default)]
    pub base_commit_sha: Option<String>,
    #[serde(default)]
    pub head_commit_sha: Option<String>,
    #[serde(default, alias = "files", alias = "fileCount")]
    pub file_count: i64,
    #[serde(default, alias = "additions", alias = "added")]
    pub line_additions: i64,
    #[serde(default, alias = "deletions", alias = "deleted")]
    pub line_deletions: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionGitStatusResponse {
    pub raw: String,
    pub summary_line: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub detached: bool,
    pub staged: i64,
    pub unstaged: i64,
    pub untracked: i64,
    #[serde(default)]
    pub entries: Vec<GitStatusEntry>,
    #[serde(default)]
    pub entries_truncated: bool,
    #[serde(default)]
    pub entries_total_count: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitStatusEntry {
    pub path: String,
    #[serde(default)]
    pub orig_path: Option<String>,
    #[serde(default)]
    pub index_status: String,
    #[serde(default)]
    pub worktree_status: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceActiveSnapshotParams {
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceArchivedPageParams {
    pub limit: Option<u32>,
    pub cursor: Option<WorkspaceIndexCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWithEnv {
    #[serde(flatten)]
    pub session: Session,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_target: Option<EnvTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicSettings {
    #[serde(default)]
    pub dictation: Option<PublicDictationSettings>,
    #[serde(default)]
    pub telemetry: Option<PublicTelemetrySettings>,
    #[serde(default)]
    pub title_generation: Option<PublicTitleGenerationSettings>,
    #[serde(default)]
    pub resource_governance: Option<PublicResourceGovernanceSettings>,
    #[serde(default)]
    pub tool_limits: Option<PublicToolLimitsSettings>,
    #[serde(default)]
    pub provider_restart: Option<PublicProviderRestartSettings>,
    #[serde(default)]
    pub subagents: Option<PublicSubagentSettings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicDictationSettings {
    pub enabled: bool,
    pub provider: DictationProvider,
    #[serde(default)]
    pub livekit: Option<PublicLiveKitDictationSettings>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicLiveKitDictationSettings {
    pub base_url: String,
    pub api_key: String,
    pub api_secret_set: bool,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicTelemetrySettings {
    pub enabled: bool,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TitleGenerationMode {
    Remote,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationRemoteSettings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub use_json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleGenerationLocalSettings {
    pub model_id: String,
    pub use_json: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicTitleGenerationSettings {
    pub mode: TitleGenerationMode,
    pub remote: TitleGenerationRemoteSettings,
    pub local: TitleGenerationLocalSettings,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicSubagentSettings {
    #[serde(default)]
    pub max_per_call: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationProvider {
    Disabled,
    #[serde(rename = "livekit_inference")]
    LiveKitInference,
    #[serde(rename = "tauri_stt")]
    TauriStt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceGovernanceMode {
    Auto,
    Custom,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceGovernanceStatusState {
    Disabled,
    Applied,
    Pending,
    Unsupported,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicResourceGovernanceStatus {
    pub state: ResourceGovernanceStatusState,
    pub can_apply_now: bool,
    pub requires_restart: bool,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicResourceGovernanceLimits {
    pub cpu_quota_pct: u32,
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicResourceGovernanceSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub cpu_quota_pct: Option<u32>,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
    #[serde(default)]
    pub effective: Option<PublicResourceGovernanceLimits>,
    #[serde(default)]
    pub status: Option<PublicResourceGovernanceStatus>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicToolLimitsLimits {
    pub memory_high_mb: u32,
    pub memory_max_mb: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicToolLimitsSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
    #[serde(default)]
    pub effective: Option<PublicToolLimitsLimits>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublicProviderRestartSettings {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(default)]
    pub memory_high_mb: Option<u32>,
    #[serde(default)]
    pub memory_max_mb: Option<u32>,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dictation: Option<UpdateDictationSettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<UpdateTelemetrySettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title_generation: Option<UpdateTitleGenerationSettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_governance: Option<UpdateResourceGovernanceSettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_guard: Option<UpdateProviderGuardSettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_limits: Option<UpdateToolLimitsSettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_restart: Option<UpdateProviderRestartSettingsRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagents: Option<UpdateSubagentSettingsRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateDictationSettingsRequest {
    pub enabled: bool,
    pub provider: DictationProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub livekit: Option<UpdateLiveKitDictationSettingsRequest>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateLiveKitDictationSettingsRequest {
    pub base_url: String,
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_secret: Option<String>,
    pub model: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateTelemetrySettingsRequest {
    pub enabled: bool,
    pub endpoint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateTitleGenerationSettingsRequest {
    pub mode: TitleGenerationMode,
    pub remote: TitleGenerationRemoteSettings,
    pub local: TitleGenerationLocalSettings,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateResourceGovernanceSettingsRequest {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_quota_pct: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProviderGuardSettingsRequest {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateToolLimitsSettingsRequest {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProviderRestartSettingsRequest {
    pub enabled: bool,
    pub mode: ResourceGovernanceMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_high_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_max_mb: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_period_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateSubagentSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_per_call: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateWorkspaceRequest {
    pub root_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateWorkspaceAttachmentRequest {
    pub kind: WorkspaceAttachmentKind,
    pub name: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount_relpath: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<AttachmentMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_policy: Option<AttachmentUpdatePolicy>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeleteWorkspaceAttachmentRequest {
    pub kind: WorkspaceAttachmentKind,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncWorkspaceAttachmentsRequest {
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileTunnelState {
    Idle,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MobileAccessStatus {
    pub enabled: bool,
    #[serde(default)]
    pub tunnel_id: Option<String>,
    #[serde(default)]
    pub public_base_url: Option<String>,
    #[serde(default)]
    pub relay_base_url: Option<String>,
    #[serde(default)]
    pub daemon_public_key: Option<String>,
    pub tunnel_state: MobileTunnelState,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnableMobileAccessResponse {
    pub status: MobileAccessStatus,
    pub qr_payload: Value,
    pub pairing_expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub cpu_pct: f32,
    pub memory_total_bytes: u64,
    pub memory_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskSnapshot {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub file_system: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceChildProcess {
    pub pid: u32,
    #[serde(default)]
    pub parent_pid: Option<u32>,
    pub name: String,
    #[serde(default)]
    pub cmdline: Option<String>,
    pub cpu_pct: f32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceProcess {
    pub label: String,
    pub pid: u32,
    pub cpu_pct: f32,
    pub memory_bytes: u64,
    pub virtual_memory_bytes: u64,
    pub child_count: u64,
    pub children: Vec<ResourceChildProcess>,
    pub children_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceProcesses {
    #[serde(default)]
    pub daemon: Option<ResourceProcess>,
    pub providers: Vec<ResourceProcess>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeDiskSnapshot {
    pub worktree_id: String,
    pub root_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDiskSnapshot {
    pub workspace_id: String,
    pub root_path: String,
    pub size_bytes: u64,
    pub size_collected_at: String,
    pub size_cache_age_ms: u64,
    #[serde(default)]
    pub disk: Option<DiskSnapshot>,
    pub worktrees: Vec<WorktreeDiskSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUtilizationSnapshot {
    pub collected_at: String,
    pub cache_age_ms: u64,
    pub system: SystemSnapshot,
    pub processes: ResourceProcesses,
    pub workspace: WorkspaceDiskSnapshot,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderOptions {
    pub provider_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub installed: Option<bool>,
    #[serde(default)]
    pub probe_ok: Option<bool>,
    #[serde(default)]
    pub probe_error: Option<String>,
    #[serde(default)]
    pub supports_load: bool,
    #[serde(default)]
    pub auth_required: bool,
    #[serde(default)]
    pub auth_methods: Option<Value>,
    #[serde(default)]
    pub modes: Option<Value>,
    #[serde(default)]
    pub models: Option<Value>,
    #[serde(default)]
    pub verify: Option<ProviderAuthCheck>,
    #[serde(default)]
    pub probed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderAuthCheck {
    pub provider_id: String,
    pub workspace_id: String,
    pub status: String,
    #[serde(default)]
    pub auth_required: Option<bool>,
    #[serde(default)]
    pub auth_methods: Option<Value>,
    #[serde(default)]
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallStartResponse {
    pub provider_id: String,
    pub install_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TitleGenerationLocalRuntimeStatus {
    pub version: String,
    pub installed: bool,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TitleGenerationLocalModelStatus {
    pub model_id: String,
    pub file_name: String,
    pub installed: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub installed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TitleGenerationLocalStatus {
    pub ready: bool,
    pub runtime: TitleGenerationLocalRuntimeStatus,
    pub model: TitleGenerationLocalModelStatus,
    #[serde(default)]
    pub install_id: Option<String>,
    #[serde(default)]
    pub install_running: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TitleGenerationLocalInstallResponse {
    pub install_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallEventLevel {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgressEvent {
    pub install_id: String,
    pub provider_id: String,
    pub at: String,
    pub stage: String,
    pub message: String,
    pub level: InstallEventLevel,
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub total_bytes: Option<u64>,
    #[serde(default)]
    pub attempt: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStateKind {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallInfo {
    pub install_id: String,
    pub provider_id: String,
    pub state: InstallStateKind,
    pub started_at: String,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub last_event: Option<InstallProgressEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_with_env_parses() {
        let payload = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "task_id": "33333333-3333-3333-3333-333333333333",
            "workspace_id": "44444444-4444-4444-4444-444444444444",
            "worktree_id": "55555555-5555-5555-5555-555555555555",
            "provider_id": "codex",
            "model_id": "gpt-4",
            "title": "Main session",
            "agent_role": "implementer",
            "status": "active",
            "provider_session_ref": null,
            "created_at": "2024-01-01T00:00:00Z",
            "updated_at": "2024-01-01T00:00:00Z",
            "env_target": "worktree"
        });

        let parsed: SessionWithEnv = serde_json::from_value(payload).unwrap();
        assert_eq!(parsed.env_target, Some(EnvTarget::Worktree));
        assert_eq!(parsed.session.provider_id, "codex");
    }
}
