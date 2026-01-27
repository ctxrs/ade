use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use reqwest::{header, multipart, Method};
use serde_json::Value;
use url::{form_urlencoded, Url};

use ctx_core::ids::{ArtifactId, SessionId, TaskId, TerminalId, WorkspaceId};
use ctx_core::models::{
    Artifact, Message, Session, SessionEventsPage, SessionHeadSnapshot, SessionHistoryPage,
    SessionSnapshot, SessionState, SessionTurnTool, Task, TerminalSession, Workspace,
    WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, WorkspaceArchivedPage, WorkspaceAttachment,
};
use ctx_providers::adapters::ProviderStatus;

use crate::client::Client;
use crate::types::*;

impl Client {
    pub async fn get_health(&self) -> Result<Health> {
        self.request_json(Method::GET, "/api/health", None::<&()>)
            .await
    }

    pub async fn list_workspaces(&self) -> Result<Vec<Workspace>> {
        self.request_json(Method::GET, "/api/workspaces", None::<&()>)
            .await
    }

    pub async fn create_workspace(
        &self,
        root_path: String,
        name: Option<String>,
    ) -> Result<Workspace> {
        let req = CreateWorkspaceRequest { root_path, name };
        self.request_json(Method::POST, "/api/workspaces", Some(&req))
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

    pub async fn list_workspace_tasks(&self, workspace_id: WorkspaceId) -> Result<Vec<Task>> {
        let path = format!("/api/workspaces/{}/tasks", workspace_id.0);
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

    pub async fn get_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
        params: &WorkspaceActiveSnapshotParams,
    ) -> Result<WorkspaceActiveSnapshot> {
        let mut path = format!("/api/workspaces/{}/active_snapshot", workspace_id.0);
        let mut search = Vec::new();
        if let Some(limit) = params.limit {
            search.push(format!("limit={}", limit));
        }
        if !search.is_empty() {
            path.push('?');
            path.push_str(&search.join("&"));
        }
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveHeadBatch> {
        let path = format!("/api/workspaces/{}/active_heads", workspace_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn list_workspace_archived_task_summaries(
        &self,
        workspace_id: WorkspaceId,
        params: &WorkspaceArchivedPageParams,
    ) -> Result<WorkspaceArchivedPage> {
        let mut path = format!("/api/workspaces/{}/archived_task_summaries", workspace_id.0);
        let mut search = Vec::new();
        if let Some(limit) = params.limit {
            search.push(format!("limit={}", limit));
        }
        if let Some(cursor) = &params.cursor {
            let sort_at = cursor.sort_at.to_rfc3339();
            let sort_at = form_urlencoded::byte_serialize(sort_at.as_bytes()).collect::<String>();
            search.push(format!("cursor_sort_at={}", sort_at));
            search.push(format!("cursor_task_id={}", cursor.task_id.0));
        }
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
            format!("/api/workspaces/{}/active_snapshot/stream", workspace_id.0)
        } else {
            format!(
                "{}/api/workspaces/{}/active_snapshot/stream",
                prefix, workspace_id.0
            )
        };
        url.set_path(&path);
        url.set_query(None);
        Ok(url.to_string())
    }

    /// Terminal websocket auth uses `?token=` for browser compatibility; headers are deprecated.
    /// Append `tail=<bytes>` to cap the initial snapshot size.
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
        if let Some(token) = &self.auth_token {
            url.query_pairs_mut().append_pair("token", token);
        }
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

    pub async fn create_session(
        &self,
        task_id: TaskId,
        req: &CreateSessionRequest,
    ) -> Result<SessionWithEnv> {
        let path = format!("/api/tasks/{}/sessions", task_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn list_task_sessions(&self, task_id: TaskId) -> Result<Vec<Session>> {
        let path = format!("/api/tasks/{}/sessions", task_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn post_message(
        &self,
        session_id: SessionId,
        req: &PostMessageRequest,
    ) -> Result<Message> {
        let path = format!("/api/sessions/{}/messages", session_id.0);
        self.request_json(Method::POST, &path, Some(req)).await
    }

    pub async fn get_session_snapshot(
        &self,
        session_id: SessionId,
        limit: Option<u32>,
        include_events: Option<bool>,
    ) -> Result<SessionSnapshot> {
        let mut path = format!("/api/sessions/{}/snapshot", session_id.0);
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

    pub async fn get_session_head(
        &self,
        session_id: SessionId,
        limit: Option<u32>,
        include_events: Option<bool>,
    ) -> Result<SessionHeadSnapshot> {
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

    pub async fn get_session_state(&self, session_id: SessionId) -> Result<SessionState> {
        let path = format!("/api/sessions/{}/state", session_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_session_diff(&self, session_id: SessionId) -> Result<SessionDiffResponse> {
        let path = format!("/api/sessions/{}/diff", session_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_session_diff_summary(
        &self,
        session_id: SessionId,
    ) -> Result<SessionDiffSummaryResponse> {
        let path = format!("/api/sessions/{}/diff/summary", session_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn get_session_git_status(
        &self,
        session_id: SessionId,
    ) -> Result<SessionGitStatusResponse> {
        let path = format!("/api/sessions/{}/git/status", session_id.0);
        self.request_json(Method::GET, &path, None::<&()>).await
    }

    pub async fn apply_session_diff_patch(
        &self,
        session_id: SessionId,
        action: &str,
        patch: &str,
    ) -> Result<SessionDiffResponse> {
        let path = format!("/api/sessions/{}/diff/apply", session_id.0);
        let req = SessionDiffApplyRequest {
            action: action.to_string(),
            patch: patch.to_string(),
        };
        self.request_json(Method::POST, &path, Some(&req)).await
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

    pub async fn get_title_generation_local_status(&self) -> Result<TitleGenerationLocalStatus> {
        self.request_json(
            Method::GET,
            "/api/title_generation/local/status",
            None::<&()>,
        )
        .await
    }

    pub async fn install_title_generation_local(
        &self,
    ) -> Result<TitleGenerationLocalInstallResponse> {
        self.request_json(
            Method::POST,
            "/api/title_generation/local/install",
            None::<&()>,
        )
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

    pub async fn get_telemetry_summary(&self, params: &TelemetrySummaryParams) -> Result<Value> {
        let mut path = "/api/telemetry/summary".to_string();
        let mut search = Vec::new();
        if let Some(metric) = &params.metric {
            let metric = form_urlencoded::byte_serialize(metric.as_bytes()).collect::<String>();
            search.push(format!("metric={metric}"));
        }
        if let Some(run_id) = &params.run_id {
            let run_id = form_urlencoded::byte_serialize(run_id.as_bytes()).collect::<String>();
            search.push(format!("run_id={run_id}"));
        }
        if let Some(window_ms) = params.window_ms {
            search.push(format!("window_ms={window_ms}"));
        }
        if let Some(limit) = params.limit {
            search.push(format!("limit={limit}"));
        }
        if !search.is_empty() {
            path.push('?');
            path.push_str(&search.join("&"));
        }
        self.request_json(Method::GET, &path, None::<&()>).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(url.ends_with("/active_snapshot/stream"));
    }
}
