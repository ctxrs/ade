use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use directories::BaseDirs;
use reqwest::{header, multipart, Method};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::{form_urlencoded, Url};

use ctx_core::ids::{ArtifactId, SessionId, TaskId, TerminalId, TrackId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    Artifact, AttachmentMode, AttachmentUpdatePolicy, Message, MessageAttachment, MessageDelivery,
    Session, SessionEventsPage, SessionHead, SessionHistoryPage, SessionTurnTool, Task,
    TerminalSession, Track, TrackDiffSummaryResponse, Workspace, WorkspaceAttachment,
    WorkspaceAttachmentKind, WorkspaceCatchupCursor, WorkspaceCatchupSnapshot,
};
use ctx_providers::adapters::ProviderStatus;

const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:4399";
const DAEMON_AUTH_FILENAME: &str = "daemon_auth.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonAuthFile {
    token: String,
    #[serde(default)]
    daemon_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub base_url: String,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Health {
    pub version: String,
    pub pid: i64,
    pub data_root: String,
    pub daemon_url: String,
    pub auth_required: bool,
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
    pub create_default_track: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_track_label: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateTaskTitleRequest<'a> {
    pub title: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTrackRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_target: Option<EnvTarget>,
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
    pub initial_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateTerminalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
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
struct TrackDiffApplyRequest {
    action: String,
    patch: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TrackDiffResponse {
    diff: String,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceCatchupParams {
    pub limit: Option<u32>,
    pub include_archived: Option<bool>,
    pub archived_only: Option<bool>,
    pub active_cursor: Option<WorkspaceCatchupCursor>,
    pub archived_cursor: Option<WorkspaceCatchupCursor>,
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

#[derive(Debug, Clone, Deserialize)]
pub struct PublicTitleGenerationSettings {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub use_json: bool,
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
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub use_json: bool,
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
pub struct UpdateSubagentSettingsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_per_call: Option<u32>,
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
    pub acp_error: Option<Value>,
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
    pub acp_error: Option<Value>,
    #[serde(default)]
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallStartResponse {
    pub provider_id: String,
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

fn normalize_base_url(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("daemon base URL is empty"));
    }
    Url::parse(trimmed).with_context(|| format!("invalid daemon URL: {trimmed}"))?;
    Ok(trimmed.trim_end_matches('/').to_string())
}

fn default_data_dir() -> Result<PathBuf> {
    let base = BaseDirs::new().context("resolving home directory")?;
    Ok(base.home_dir().join(".ctx"))
}

fn read_daemon_auth_file(path: &Path) -> Result<Option<DaemonAuthFile>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let auth: DaemonAuthFile = serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing daemon auth file {}", path.display()))?;
            if auth.token.trim().is_empty() {
                anyhow::bail!("daemon auth file {} contains empty token", path.display());
            }
            Ok(Some(auth))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => {
            Err(err).with_context(|| format!("reading daemon auth file {}", path.display()))
        }
    }
}

fn resolve_daemon_config_with(
    data_dir: Option<&Path>,
    override_url: Option<&str>,
) -> Result<DaemonConfig> {
    let auth = if let Some(dir) = data_dir {
        read_daemon_auth_file(&dir.join(DAEMON_AUTH_FILENAME))?
    } else {
        None
    };

    let base_url = override_url
        .map(str::to_string)
        .or_else(|| auth.as_ref().and_then(|a| a.daemon_url.clone()))
        .unwrap_or_else(|| DEFAULT_DAEMON_URL.to_string());

    Ok(DaemonConfig {
        base_url: normalize_base_url(&base_url)?,
        auth_token: auth.map(|a| a.token),
    })
}

pub fn resolve_daemon_config() -> Result<DaemonConfig> {
    let override_url = env::var("CTX_DAEMON_URL").ok();
    let data_dir = match env::var("CTX_DATA_DIR") {
        Ok(value) => Some(PathBuf::from(value)),
        Err(_) => Some(default_data_dir()?),
    };
    resolve_daemon_config_with(data_dir.as_deref(), override_url.as_deref())
}

pub struct Client {
    base_url: String,
    auth_token: Option<String>,
    http: reqwest::Client,
}

impl Client {
    pub fn new(config: DaemonConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;
        Ok(Self {
            base_url: config.base_url,
            auth_token: config.auth_token,
            http,
        })
    }

    pub fn from_env() -> Result<Self> {
        Self::new(resolve_daemon_config()?)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn url_for(&self, path: &str) -> Result<String> {
        if !path.starts_with('/') {
            return Err(anyhow!("path must start with '/'"));
        }
        Ok(format!("{}{}", self.base_url, path))
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let url = self.url_for(path)?;
        let mut req = self.http.request(method, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        if let Some(value) = body {
            req = req.json(value);
        }
        let resp = req.send().await.context("sending request")?;
        let status = resp.status();
        let text = resp.text().await.context("reading response body")?;
        if !status.is_success() {
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        if text.trim().is_empty() {
            return Err(anyhow!("empty response body from {}", path));
        }
        serde_json::from_str(&text).with_context(|| format!("parsing JSON response from {}", path))
    }

    async fn request_empty(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<()> {
        let url = self.url_for(path)?;
        let mut req = self.http.request(method, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        if let Some(value) = body {
            req = req.json(value);
        }
        let resp = req.send().await.context("sending request")?;
        let status = resp.status();
        let text = resp.text().await.context("reading response body")?;
        if !status.is_success() {
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        Ok(())
    }

    pub async fn get_health(&self) -> Result<Health> {
        self.request_json(Method::GET, "/api/health", None::<&()>)
            .await
    }

    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        self.request_json(Method::GET, "/api/workspaces", None::<&()>)
            .await
    }

    pub async fn list_web_sessions(&self) -> Result<Vec<WebSessionInfo>> {
        self.request_json(Method::GET, "/api/sessions/web", None::<&()>)
            .await
    }

    pub async fn get_workspace(&self, workspace_id: WorkspaceId) -> Result<Workspace> {
        let path = format!("/api/workspaces/{}", workspace_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn create_task(
        &self,
        workspace_id: WorkspaceId,
        req: &CreateTaskRequest,
    ) -> Result<Task> {
        let path = format!("/api/workspaces/{}/tasks", workspace_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn update_task_title(&self, task_id: TaskId, title: &str) -> Result<Task> {
        let path = format!("/api/tasks/{}/title", task_id.0);
        let req = UpdateTaskTitleRequest { title };
        self.request_json(Method::POST, &path, Some(&req)).await
    }

    pub async fn delete_task(&self, task_id: TaskId) -> Result<()> {
        let path = format!("/api/tasks/{}", task_id.0);
        self.request_empty(Method::DELETE, &path, None::<&()>).await
    }

    pub async fn archive_task(&self, task_id: TaskId) -> Result<Task> {
        let path = format!("/api/tasks/{}/archive", task_id.0);
        self.request_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn unarchive_task(&self, task_id: TaskId) -> Result<Task> {
        let path = format!("/api/tasks/{}/unarchive", task_id.0);
        self.request_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn mark_task_read(&self, task_id: TaskId) -> Result<Task> {
        let path = format!("/api/tasks/{}/mark_read", task_id.0);
        self.request_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn mark_task_unread(&self, task_id: TaskId) -> Result<Task> {
        let path = format!("/api/tasks/{}/mark_unread", task_id.0);
        self.request_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn get_workspace_catchup(
        &self,
        workspace_id: WorkspaceId,
        params: &WorkspaceCatchupParams,
    ) -> Result<WorkspaceCatchupSnapshot> {
        let mut search = Vec::new();
        if let Some(limit) = params.limit {
            search.push(format!("limit={}", limit));
        }
        if let Some(include_archived) = params.include_archived {
            search.push(format!(
                "include_archived={}",
                if include_archived { "1" } else { "0" }
            ));
        }
        if let Some(archived_only) = params.archived_only {
            search.push(format!(
                "archived_only={}",
                if archived_only { "1" } else { "0" }
            ));
        }
        if let Some(cursor) = params.active_cursor.as_ref() {
            search.push(format!(
                "active_cursor_sort_at={}",
                cursor.sort_at.to_rfc3339()
            ));
            search.push(format!("active_cursor_task_id={}", cursor.task_id.0));
        }
        if let Some(cursor) = params.archived_cursor.as_ref() {
            search.push(format!(
                "archived_cursor_sort_at={}",
                cursor.sort_at.to_rfc3339()
            ));
            search.push(format!("archived_cursor_task_id={}", cursor.task_id.0));
        }
        let mut path = format!("/api/workspaces/{}/catchup", workspace_id.0);
        if !search.is_empty() {
            path.push('?');
            path.push_str(&search.join("&"));
        }
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub fn workspace_stream_url(&self, workspace_id: WorkspaceId) -> Result<String> {
        let mut url = Url::parse(&self.base_url)
            .with_context(|| format!("invalid base url: {}", self.base_url))?;
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            other => return Err(anyhow!("unsupported base url scheme: {}", other)),
        };
        url.set_scheme(scheme)
            .map_err(|_| anyhow!("failed to set websocket scheme"))?;
        let prefix = url.path().trim_end_matches('/');
        let path = if prefix.is_empty() {
            format!("/api/workspaces/{}/stream", workspace_id.0)
        } else {
            format!("{}/api/workspaces/{}/stream", prefix, workspace_id.0)
        };
        url.set_path(&path);
        url.set_query(None);
        Ok(url.to_string())
    }

    pub fn terminal_stream_url(&self, terminal_id: TerminalId) -> Result<String> {
        let mut url = Url::parse(&self.base_url)
            .with_context(|| format!("invalid base url: {}", self.base_url))?;
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            other => return Err(anyhow!("unsupported base url scheme: {}", other)),
        };
        url.set_scheme(scheme)
            .map_err(|_| anyhow!("failed to set websocket scheme"))?;
        let prefix = url.path().trim_end_matches('/');
        let path = if prefix.is_empty() {
            format!("/api/terminals/{}/stream", terminal_id.0)
        } else {
            format!("{}/api/terminals/{}/stream", prefix, terminal_id.0)
        };
        url.set_path(&path);
        url.set_query(None);
        Ok(url.to_string())
    }

    pub async fn list_workspace_terminals(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<TerminalSession>> {
        let path = format!("/api/workspaces/{}/terminals", workspace_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn create_workspace_terminal(
        &self,
        workspace_id: WorkspaceId,
        req: &CreateTerminalRequest,
    ) -> Result<TerminalSession> {
        let path = format!("/api/workspaces/{}/terminals", workspace_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn delete_terminal(&self, terminal_id: TerminalId) -> Result<()> {
        let path = format!("/api/terminals/{}", terminal_id.0);
        self.request_empty(Method::DELETE, &path, None::<&()>).await
    }

    pub async fn create_track(&self, task_id: TaskId, req: &CreateTrackRequest) -> Result<Track> {
        let path = format!("/api/tasks/{}/tracks", task_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn create_session(
        &self,
        track_id: TrackId,
        req: &CreateSessionRequest,
    ) -> Result<SessionWithEnv> {
        let path = format!("/api/tracks/{}/sessions", track_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn post_message(
        &self,
        session_id: SessionId,
        req: &PostMessageRequest,
    ) -> Result<Message> {
        let path = format!("/api/sessions/{}/messages", session_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn get_session_head(
        &self,
        session_id: SessionId,
        limit: Option<u32>,
        include_events: Option<bool>,
    ) -> Result<SessionHead> {
        let mut path = format!("/api/sessions/{}/head", session_id.0);
        let mut params = Vec::new();
        if let Some(limit) = limit {
            params.push(format!("limit={}", limit));
        }
        if let Some(include_events) = include_events {
            params.push(format!(
                "include_events={}",
                if include_events { "1" } else { "0" }
            ));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_session_history(
        &self,
        session_id: SessionId,
        before_seq: Option<i64>,
        limit: Option<u32>,
    ) -> Result<SessionHistoryPage> {
        let mut path = format!("/api/sessions/{}/history", session_id.0);
        let mut params = Vec::new();
        if let Some(before_seq) = before_seq {
            params.push(format!("before_seq={}", before_seq));
        }
        if let Some(limit) = limit {
            params.push(format!("limit={}", limit));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_session_events(
        &self,
        session_id: SessionId,
        after_seq: Option<i64>,
        limit: Option<u32>,
        tail: Option<u32>,
    ) -> Result<SessionEventsPage> {
        let mut path = format!("/api/sessions/{}/events", session_id.0);
        let mut params = Vec::new();
        if let Some(after_seq) = after_seq {
            params.push(format!("after_seq={}", after_seq));
        }
        if let Some(limit) = limit {
            params.push(format!("limit={}", limit));
        }
        if let Some(tail) = tail {
            params.push(format!("tail={}", tail));
        }
        if !params.is_empty() {
            path.push('?');
            path.push_str(&params.join("&"));
        }
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_turn_tools(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Result<Vec<SessionTurnTool>> {
        let path = format!("/api/sessions/{}/turns/{}/tools", session_id.0, turn_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_session_file_completions(
        &self,
        session_id: SessionId,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<String>> {
        let path = {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("query", query);
            if let Some(limit) = limit {
                serializer.append_pair("limit", &limit.to_string());
            }
            let qs = serializer.finish();
            format!("/api/sessions/{}/completions/files?{}", session_id.0, qs)
        };
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_workspace_file_completions(
        &self,
        workspace_id: WorkspaceId,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<String>> {
        let path = {
            let mut serializer = form_urlencoded::Serializer::new(String::new());
            serializer.append_pair("query", query);
            if let Some(limit) = limit {
                serializer.append_pair("limit", &limit.to_string());
            }
            let qs = serializer.finish();
            format!(
                "/api/workspaces/{}/completions/files?{}",
                workspace_id.0, qs
            )
        };
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_session_artifacts(&self, session_id: SessionId) -> Result<Vec<Artifact>> {
        let path = format!("/api/sessions/{}/artifacts", session_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_artifact_bytes(
        &self,
        artifact_id: ArtifactId,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<u8>> {
        let path = format!("/api/artifacts/{}", artifact_id.0);
        let url = self.url_for(&path)?;
        let mut req = self.http.request(Method::GET, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        if let Some((start, end)) = range {
            let value = format!("bytes={start}-{end}");
            req = req.header(header::RANGE, value);
        }
        let resp = req.send().await.context("sending request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.context("reading response body")?;
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        let bytes = resp.bytes().await.context("reading response body")?;
        Ok(bytes.to_vec())
    }

    pub async fn upload_blob(
        &self,
        bytes: Vec<u8>,
        mime_type: &str,
        name: Option<&str>,
    ) -> Result<BlobUploadResp> {
        let url = self.url_for("/api/blobs")?;
        let mut req = self.http.request(Method::POST, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }

        let mut part = multipart::Part::bytes(bytes)
            .mime_str(mime_type)
            .context("invalid blob mime type")?;
        if let Some(name) = name {
            part = part.file_name(name.to_string());
        }
        let form = multipart::Form::new().part("file", part);
        let resp = req
            .multipart(form)
            .send()
            .await
            .context("sending request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.context("reading response body")?;
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        let text = resp.text().await.context("reading response body")?;
        let resp = if text.trim().is_empty() {
            return Err(anyhow!("empty response when uploading blob"));
        } else {
            serde_json::from_str::<BlobUploadResp>(&text).context("decoding blob response")?
        };
        Ok(resp)
    }

    pub async fn get_blob(&self, blob_id: &str) -> Result<Vec<u8>> {
        let path = format!("/api/blobs/{blob_id}");
        let url = self.url_for(&path)?;
        let mut req = self.http.request(Method::GET, url);
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await.context("sending request")?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.context("reading response body")?;
            let snippet = text.trim();
            let msg = if snippet.is_empty() {
                format!("request failed with status {}", status.as_u16())
            } else {
                format!(
                    "request failed with status {}: {}",
                    status.as_u16(),
                    snippet
                )
            };
            return Err(anyhow!(msg));
        }
        let bytes = resp.bytes().await.context("reading response body")?;
        Ok(bytes.to_vec())
    }

    pub async fn track_diff(&self, track_id: TrackId) -> Result<TrackDiffSummaryResponse> {
        let path = format!("/api/tracks/{}/diff_summary", track_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_track_diff(&self, track_id: TrackId) -> Result<String> {
        let path = format!("/api/tracks/{}/diff", track_id.0);
        let resp: TrackDiffResponse = self.request_json(Method::GET, &path, None::<&()>).await?;
        Ok(resp.diff)
    }

    pub async fn apply_track_diff_patch(
        &self,
        track_id: TrackId,
        action: &str,
        patch: &str,
    ) -> Result<String> {
        let path = format!("/api/tracks/{}/diff/apply", track_id.0);
        let req = TrackDiffApplyRequest {
            action: action.to_string(),
            patch: patch.to_string(),
        };
        let resp: TrackDiffResponse = self.request_json(Method::POST, &path, Some(&req)).await?;
        Ok(resp.diff)
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderStatus>> {
        self.request_json(Method::GET, "/api/providers", None::<&()>)
            .await
    }

    pub async fn get_settings(&self) -> Result<PublicSettings> {
        self.request_json(Method::GET, "/api/settings", None::<&()>)
            .await
    }

    pub async fn update_settings(&self, req: &UpdateSettingsRequest) -> Result<PublicSettings> {
        self.request_json(Method::POST, "/api/settings", Some(req))
            .await
    }

    pub async fn list_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let path = format!("/api/workspaces/{}/attachments", workspace_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn sync_workspace_attachments(
        &self,
        workspace_id: WorkspaceId,
        refresh: bool,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let path = format!("/api/workspaces/{}/attachments/sync", workspace_id.0);
        let req = SyncWorkspaceAttachmentsRequest { refresh };
        self.request_json(Method::POST, &path, Some(&req)).await
    }

    pub async fn create_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        req: &CreateWorkspaceAttachmentRequest,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let path = format!("/api/workspaces/{}/attachments", workspace_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn delete_workspace_attachment(
        &self,
        workspace_id: WorkspaceId,
        req: &DeleteWorkspaceAttachmentRequest,
    ) -> Result<Vec<WorkspaceAttachment>> {
        let path = format!("/api/workspaces/{}/attachments", workspace_id.0);
        self.request_json(Method::DELETE, &path, Some(req)).await
    }

    pub async fn get_resource_utilization(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<ResourceUtilizationSnapshot> {
        let path = format!("/api/resource_utilization?workspace_id={}", workspace_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_mobile_access_status(&self) -> Result<MobileAccessStatus> {
        self.request_json(Method::GET, "/api/mobile/access/status", None::<&()>)
            .await
    }

    pub async fn enable_mobile_access(
        &self,
        supabase_token: &str,
    ) -> Result<EnableMobileAccessResponse> {
        let req = serde_json::json!({ "supabase_token": supabase_token });
        self.request_json(Method::POST, "/api/mobile/access/enable", Some(&req))
            .await
    }

    pub async fn disable_mobile_access(&self, supabase_token: &str) -> Result<()> {
        let req = serde_json::json!({ "supabase_token": supabase_token });
        self.request_empty(Method::POST, "/api/mobile/access/disable", Some(&req))
            .await
    }

    pub async fn get_provider_options(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
    ) -> Result<ProviderOptions> {
        let path = format!(
            "/api/workspaces/{}/providers/{}/options",
            workspace_id.0, provider_id
        );
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn authenticate_provider_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
        method_id: Option<&str>,
    ) -> Result<ProviderAuthCheck> {
        let path = format!(
            "/api/workspaces/{}/providers/{}/authenticate",
            workspace_id.0, provider_id
        );
        let req = AuthenticateProviderRequest {
            method_id: method_id.map(|value| value.to_string()),
        };
        self.request_json(Method::POST, &path, Some(&req)).await
    }

    pub async fn verify_provider_for_workspace(
        &self,
        workspace_id: WorkspaceId,
        provider_id: &str,
    ) -> Result<ProviderAuthCheck> {
        let path = format!(
            "/api/workspaces/{}/providers/{}/verify",
            workspace_id.0, provider_id
        );
        self.request_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn install_provider(&self, provider_id: &str) -> Result<InstallStartResponse> {
        let path = format!("/api/providers/{}/install", provider_id);
        self.request_json(Method::POST, &path, None::<&()>).await
    }

    pub async fn install_all_providers(&self) -> Result<Vec<InstallStartResponse>> {
        self.request_json(Method::POST, "/api/providers/install_all", None::<&()>)
            .await
    }

    pub async fn get_install(&self, install_id: &str) -> Result<InstallInfo> {
        let path = format!("/api/providers/install/{}", install_id);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_install_events(&self, install_id: &str) -> Result<Vec<InstallProgressEvent>> {
        let path = format!("/api/providers/install/{}/events", install_id);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn cancel_session(&self, session_id: SessionId) -> Result<()> {
        let path = format!("/api/sessions/{}/cancel", session_id.0);
        self.request_empty(Method::POST, &path, None::<&()>).await
    }

    pub async fn interrupt_session(&self, session_id: SessionId) -> Result<()> {
        let path = format!("/api/sessions/{}/interrupt", session_id.0);
        self.request_empty(Method::POST, &path, None::<&()>).await
    }

    pub async fn set_session_model(
        &self,
        session_id: SessionId,
        model_id: &str,
    ) -> Result<Session> {
        let path = format!("/api/sessions/{}/model", session_id.0);
        let req = SetSessionModelRequest {
            model_id: model_id.to_string(),
        };
        self.request_json(Method::POST, &path, Some(&req)).await
    }

    pub async fn set_session_mode(&self, session_id: SessionId, mode_id: &str) -> Result<()> {
        let path = format!("/api/sessions/{}/mode", session_id.0);
        let req = SetSessionModeRequest {
            mode_id: mode_id.to_string(),
        };
        self.request_empty(Method::POST, &path, Some(&req)).await
    }

    pub async fn authenticate_session(
        &self,
        session_id: SessionId,
        method_id: Option<&str>,
    ) -> Result<()> {
        let path = format!("/api/sessions/{}/authenticate", session_id.0);
        let req = AuthenticateSessionRequest {
            method_id: method_id.map(|value| value.to_string()),
        };
        self.request_empty(Method::POST, &path, Some(&req)).await
    }

    pub async fn submit_ask_user_question(
        &self,
        session_id: SessionId,
        req: &AskUserQuestionRequest,
    ) -> Result<()> {
        let path = format!("/api/sessions/{}/ask_user_question", session_id.0);
        self.request_empty(Method::POST, &path, Some(req)).await
    }

    pub async fn ask_user_question(
        &self,
        session_id: SessionId,
        tool_call_id: &str,
        outcome: AskUserQuestionOutcome,
        answers: Option<HashMap<String, String>>,
    ) -> Result<()> {
        let req = AskUserQuestionRequest {
            tool_call_id: tool_call_id.to_string(),
            outcome,
            answers,
        };
        self.submit_ask_user_question(session_id, &req).await
    }

    pub async fn post_client_telemetry(&self, batch: &ClientTelemetryBatch) -> Result<()> {
        self.request_empty(Method::POST, "/api/telemetry/client", Some(batch))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_base_url_trims() {
        let url = normalize_base_url("http://127.0.0.1:4399/").unwrap();
        assert_eq!(url, "http://127.0.0.1:4399");
    }

    #[test]
    fn normalize_base_url_rejects_empty() {
        let err = normalize_base_url(" ").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn resolve_daemon_config_prefers_override() {
        let dir = tempfile::tempdir().unwrap();
        let auth = DaemonAuthFile {
            token: "token".to_string(),
            daemon_url: Some("http://127.0.0.1:1234".to_string()),
        };
        let path = dir.path().join(DAEMON_AUTH_FILENAME);
        std::fs::write(&path, serde_json::to_vec(&auth).unwrap()).unwrap();

        let cfg =
            resolve_daemon_config_with(Some(dir.path()), Some("http://127.0.0.1:5678")).unwrap();
        assert_eq!(cfg.base_url, "http://127.0.0.1:5678");
        assert_eq!(cfg.auth_token.as_deref(), Some("token"));
    }

    #[test]
    fn resolve_daemon_config_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = resolve_daemon_config_with(Some(dir.path()), None).unwrap();
        assert_eq!(cfg.base_url, DEFAULT_DAEMON_URL);
        assert!(cfg.auth_token.is_none());
    }

    #[test]
    fn read_daemon_auth_file_rejects_empty_token() {
        let dir = tempfile::tempdir().unwrap();
        let auth = DaemonAuthFile {
            token: "".to_string(),
            daemon_url: None,
        };
        let path = dir.path().join(DAEMON_AUTH_FILENAME);
        std::fs::write(&path, serde_json::to_vec(&auth).unwrap()).unwrap();

        let err = read_daemon_auth_file(&path).unwrap_err();
        assert!(err.to_string().contains("empty token"));
    }

    #[test]
    fn session_with_env_parses() {
        let payload = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "track_id": "22222222-2222-2222-2222-222222222222",
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

    #[test]
    fn workspace_stream_url_builds() {
        let client = Client {
            base_url: "https://example.com/base".to_string(),
            auth_token: None,
            http: reqwest::Client::new(),
        };
        let workspace_id = WorkspaceId::new();
        let url = client.workspace_stream_url(workspace_id).unwrap();
        assert!(url.starts_with("wss://example.com/base/api/workspaces/"));
        assert!(url.ends_with("/stream"));
    }
}
