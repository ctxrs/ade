use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use directories::BaseDirs;
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use ctx_core::ids::{SessionId, TaskId, TrackId, WorkspaceId};
use ctx_core::models::{
    Artifact, Message, MessageAttachment, MessageDelivery, Session, SessionEventsPage, SessionHead,
    SessionHistoryPage, SessionTurnTool, Task, Track, TrackDiffSummaryResponse, Workspace,
    WorkspaceCatchupCursor, WorkspaceCatchupSnapshot,
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

    pub async fn list_session_artifacts(&self, session_id: SessionId) -> Result<Vec<Artifact>> {
        let path = format!("/api/sessions/{}/artifacts", session_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn track_diff(&self, track_id: TrackId) -> Result<TrackDiffSummaryResponse> {
        let path = format!("/api/tracks/{}/diff_summary", track_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderStatus>> {
        self.request_json(Method::GET, "/api/providers", None::<&()>)
            .await
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
