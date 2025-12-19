use crate::cody_client::{CodyClient, CodyModelAvailability};
use crate::config::Config;
use crate::prompt::build_prompt;
use crate::session::{
    extract_assistant_text, extract_processes, ProcessSnapshot, SessionState, TranscriptEnvelope,
    TranscriptProcess, WebviewPostMessage,
};
use crate::ACP_CLIENT;
use agent_client_protocol::{
    Agent, AgentCapabilities, AuthMethod, AuthenticateRequest, AuthenticateResponse,
    CancelNotification, Client, ClientCapabilities, ContentChunk, Error, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    McpCapabilities, ModelId, ModelInfo, NewSessionRequest, NewSessionResponse, PromptCapabilities,
    PromptRequest, PromptResponse, ProtocolVersion, SessionId, SessionModelState,
    SessionNotification, SessionUpdate, SetSessionModeRequest, SetSessionModeResponse,
    SetSessionModelRequest, SetSessionModelResponse, StopReason, ToolCall, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields, ToolKind,
};
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::oneshot;
use uuid::Uuid;

pub struct CodyAgent {
    client_capabilities: Arc<std::sync::Mutex<ClientCapabilities>>,
    sessions: Arc<Mutex<HashMap<SessionId, Arc<Mutex<SessionState>>>>>,
    manager: Arc<Mutex<CodyManager>>,
}

impl CodyAgent {
    pub fn new(config: Config) -> Self {
        Self {
            client_capabilities: Arc::default(),
            sessions: Arc::default(),
            manager: Arc::new(Mutex::new(CodyManager::new(config))),
        }
    }

    async fn ensure_client(&self, cwd: &Path) -> Result<Arc<CodyClient>, Error> {
        let mut manager = self.manager.lock().await;
        manager.ensure_client(cwd).await
    }

    async fn get_session(&self, session_id: &SessionId) -> Result<Arc<Mutex<SessionState>>, Error> {
        let sessions = self.sessions.lock().await;
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| Error::resource_not_found(None))
    }

    fn check_auth(&self, client: &CodyClient) -> Result<(), Error> {
        if client.auth_status().authenticated {
            Ok(())
        } else {
            Err(Error::auth_required().data(
                "Cody is not authenticated. Run `cody auth login` or set SRC_ACCESS_TOKEN.".to_string(),
            ))
        }
    }

    async fn build_model_state(
        &self,
        client: &CodyClient,
        current: Option<ModelId>,
    ) -> Result<SessionModelState, Error> {
        let models = client
            .chat_models()
            .await
            .map_err(internal_error)?;
        model_state_from_models(&models, current)
    }

}

struct CodyManager {
    config: Config,
    client: Option<Arc<CodyClient>>,
    workspace_root: Option<PathBuf>,
}

impl CodyManager {
    fn new(config: Config) -> Self {
        Self {
            config,
            client: None,
            workspace_root: None,
        }
    }

    async fn ensure_client(&mut self, cwd: &Path) -> Result<Arc<CodyClient>, Error> {
        if let Some(root) = self.workspace_root.as_ref() {
            if root != cwd {
                return Err(Error::invalid_params().data(format!(
                    "Cody ACP already bound to {}, cannot use {}",
                    root.display(),
                    cwd.display()
                )));
            }
        }

        if let Some(client) = self.client.as_ref() {
            return Ok(Arc::clone(client));
        }

        let client = CodyClient::spawn(&self.config, cwd)
            .await
            .map_err(internal_error)?;
        self.workspace_root = Some(cwd.to_path_buf());
        self.client = Some(Arc::new(client));
        Ok(Arc::clone(self.client.as_ref().unwrap()))
    }
}

async fn handle_transcript_update(
    session_id: SessionId,
    session: Arc<Mutex<SessionState>>,
    envelope: TranscriptEnvelope,
) {
    if envelope.kind != "transcript" {
        return;
    }

    if let Some(chat_id) = envelope.chat_id.clone() {
        let mut guard = session.lock().await;
        if guard.chat_id.as_deref() != Some(chat_id.as_str()) {
            guard.chat_id = Some(chat_id);
        }
    }

    if let Some(text) = extract_assistant_text(&envelope.messages) {
        let delta = {
            let mut guard = session.lock().await;
            let delta = if text.starts_with(&guard.last_assistant_text) {
                text[guard.last_assistant_text.len()..].to_string()
            } else {
                text.clone()
            };
            guard.last_assistant_text = text.clone();
            delta
        };

        if !delta.is_empty() {
            let client = SessionClient::new(session_id.clone());
            client.send_agent_text(delta).await;
        }
    }

    let processes = extract_processes(&envelope.messages);
    if !processes.is_empty() {
        update_processes(session_id, session, processes).await;
    }
}

async fn update_processes(
    session_id: SessionId,
    session: Arc<Mutex<SessionState>>,
    processes: Vec<TranscriptProcess>,
) {
    let client = SessionClient::new(session_id);

    for process in processes {
        let Some(id) = process.id.clone() else {
            continue;
        };
        let status = map_process_status(process.state.as_deref());
        let title = process
            .title
            .clone()
            .unwrap_or_else(|| "Cody process".to_string());
        let content = process.content.clone().unwrap_or_default();

        let mut guard = session.lock().await;
        let previous = guard.processes.get(&id).cloned();
        let snapshot = ProcessSnapshot {
            state: process.state.clone(),
            _title: process.title.clone(),
            content: process.content.clone(),
        };
        guard.processes.insert(id.clone(), snapshot);
        drop(guard);

        match previous {
            None => {
                let mut tool_call = ToolCall::new(id.clone(), title)
                    .kind(ToolKind::Execute)
                    .status(status);
                if !content.is_empty() {
                    tool_call = tool_call.content(vec![content.clone().into()]);
                }
                client.send_notification(SessionUpdate::ToolCall(tool_call)).await;
            }
            Some(prev) => {
                let mut fields = ToolCallUpdateFields::new();
                let mut changed = false;
                if prev.state.as_deref() != process.state.as_deref() {
                    fields = fields.status(status);
                    changed = true;
                }
                if prev.content.as_deref() != process.content.as_deref() && !content.is_empty() {
                    fields = fields.content(vec![content.clone().into()]);
                    changed = true;
                }
                if changed {
                    let update = ToolCallUpdate::new(id.clone(), fields);
                    client
                        .send_notification(SessionUpdate::ToolCallUpdate(update))
                        .await;
                }
            }
        }
    }
}

#[async_trait::async_trait(?Send)]
impl Agent for CodyAgent {
    async fn initialize(&self, request: InitializeRequest) -> Result<InitializeResponse, Error> {
        *self.client_capabilities.lock().unwrap() = request.client_capabilities;

        let agent_capabilities = AgentCapabilities::new()
            .prompt_capabilities(PromptCapabilities::new().embedded_context(true))
            .mcp_capabilities(McpCapabilities::new().http(false).sse(false));

        Ok(InitializeResponse::new(ProtocolVersion::V1)
            .agent_capabilities(agent_capabilities)
            .agent_info(Implementation::new("cody-acp", env!("CARGO_PKG_VERSION")).title("Cody"))
            .auth_methods(Vec::<AuthMethod>::new()))
    }

    async fn authenticate(
        &self,
        _request: AuthenticateRequest,
    ) -> Result<AuthenticateResponse, Error> {
        Err(Error::auth_required().data(
            "Authenticate via `cody auth login` or set SRC_ACCESS_TOKEN.".to_string(),
        ))
    }

    async fn new_session(&self, request: NewSessionRequest) -> Result<NewSessionResponse, Error> {
        let NewSessionRequest { cwd, .. } = request;
        if !cwd.is_absolute() {
            return Err(Error::invalid_params().data(
                "`cwd` must be an absolute path".to_string(),
            ));
        }

        let client = self.ensure_client(&cwd).await?;
        self.check_auth(&client)?;

        let panel_id = client.chat_new().await.map_err(internal_error)?;
        let session_id = SessionId::new(format!("cody-{}", Uuid::new_v4()));

        let model_state = self.build_model_state(&client, None).await.ok();

        let session_state = SessionState {
            cwd,
            panel_id,
            chat_id: None,
            last_assistant_text: String::new(),
            pending_cancel: None,
            processes: HashMap::new(),
            current_model: None,
        };

        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), Arc::new(Mutex::new(session_state)));

        let mut response = NewSessionResponse::new(session_id);
        if let Some(models) = model_state {
            response = response.models(models);
        }

        Ok(response)
    }

    async fn load_session(
        &self,
        request: LoadSessionRequest,
    ) -> Result<LoadSessionResponse, Error> {
        let session = self.get_session(&request.session_id).await?;
        let cwd = { session.lock().await.cwd.clone() };
        let client = self.ensure_client(&cwd).await?;
        self.check_auth(&client)?;

        let current_model = { session.lock().await.current_model.clone() };
        let models = self.build_model_state(&client, current_model).await?;
        Ok(LoadSessionResponse::new().models(models))
    }

    async fn prompt(&self, request: PromptRequest) -> Result<PromptResponse, Error> {
        let session = self.get_session(&request.session_id).await?;
        let cwd = { session.lock().await.cwd.clone() };
        let client = self.ensure_client(&cwd).await?;
        self.check_auth(&client)?;

        let panel_id = { session.lock().await.panel_id.clone() };
        let prompt_input = build_prompt(&request.prompt);
        if prompt_input.text.trim().is_empty() {
            return Err(Error::invalid_params().data(
                "prompt text is empty".to_string(),
            ));
        }

        let (cancel_tx, cancel_rx) = oneshot::channel();
        {
            let mut guard = session.lock().await;
            if let Some(prev) = guard.pending_cancel.take() {
                let _ = prev.send(());
            }
            guard.pending_cancel = Some(cancel_tx);
        }

        let mut notifications = client.subscribe_notifications();
        let session_id = request.session_id.clone();
        let session_id_for_notify = session_id.clone();
        let session_clone = Arc::clone(&session);
        let panel_id_clone = panel_id.clone();
        let (done_tx, mut done_rx) = oneshot::channel::<()>();

        let notify_task = tokio::task::spawn_local(async move {
            loop {
                tokio::select! {
                    _ = &mut done_rx => break,
                    notification = notifications.recv() => {
                        let notification = match notification {
                            Ok(notification) => notification,
                            Err(_) => break,
                        };
                        if notification.method != "webview/postMessage" {
                            continue;
                        }
                        let message: WebviewPostMessage = match serde_json::from_value(notification.params) {
                            Ok(message) => message,
                            Err(_) => continue,
                        };
                        if message.id != panel_id_clone {
                            continue;
                        }
                        handle_transcript_update(
                            session_id_for_notify.clone(),
                            Arc::clone(&session_clone),
                            message.message,
                        )
                        .await;
                    }
                }
            }
        });

        let response = tokio::select! {
            result = client.chat_submit_message(&panel_id, &prompt_input.text, &prompt_input.context_items) => {
                result.map_err(internal_error)?
            }
            _ = cancel_rx => {
                let _ = client
                    .webview_receive_message(&panel_id, json!({"command": "abort"}))
                    .await;
                let _ = done_tx.send(());
                let _ = notify_task.await;
                { session.lock().await.pending_cancel = None; }
                return Ok(PromptResponse::new(StopReason::Cancelled));
            }
        };

        let _ = done_tx.send(());
        let _ = notify_task.await;
        {
            session.lock().await.pending_cancel = None;
        }

        let envelope: TranscriptEnvelope = serde_json::from_value(response).map_err(internal_error)?;
        handle_transcript_update(session_id, Arc::clone(&session), envelope).await;

        Ok(PromptResponse::new(StopReason::EndTurn))
    }

    async fn cancel(&self, args: CancelNotification) -> Result<(), Error> {
        let session = self.get_session(&args.session_id).await?;
        let (panel_id, cwd, pending_cancel) = {
            let mut guard = session.lock().await;
            (
                guard.panel_id.clone(),
                guard.cwd.clone(),
                guard.pending_cancel.take(),
            )
        };
        if let Some(cancel_tx) = pending_cancel {
            let _ = cancel_tx.send(());
        }
        if let Ok(client) = self.ensure_client(&cwd).await {
            let _ = client
                .webview_receive_message(&panel_id, json!({"command": "abort"}))
                .await;
        }
        Ok(())
    }

    async fn set_session_mode(
        &self,
        _args: SetSessionModeRequest,
    ) -> Result<SetSessionModeResponse, Error> {
        Ok(SetSessionModeResponse::default())
    }

    async fn set_session_model(
        &self,
        args: SetSessionModelRequest,
    ) -> Result<SetSessionModelResponse, Error> {
        let session = self.get_session(&args.session_id).await?;
        let panel_id = { session.lock().await.panel_id.clone() };
        let cwd = { session.lock().await.cwd.clone() };
        let client = self.ensure_client(&cwd).await?;
        self.check_auth(&client)?;
        client
            .chat_set_model(&panel_id, args.model_id.0.as_ref())
            .await
            .map_err(internal_error)?;
        session.lock().await.current_model = Some(args.model_id.clone());
        Ok(SetSessionModelResponse::default())
    }
}

struct SessionClient {
    session_id: SessionId,
    client: Arc<dyn Client>,
}

impl SessionClient {
    fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            client: ACP_CLIENT.get().expect("ACP client not initialized").clone(),
        }
    }

    async fn send_notification(&self, update: SessionUpdate) {
        let _ = self
            .client
            .session_notification(SessionNotification::new(self.session_id.clone(), update))
            .await;
    }

    async fn send_agent_text(&self, text: String) {
        self.send_notification(SessionUpdate::AgentMessageChunk(ContentChunk::new(
            text.into(),
        )))
        .await;
    }
}

fn map_process_status(state: Option<&str>) -> ToolCallStatus {
    match state {
        Some("pending") => ToolCallStatus::Pending,
        Some("success") => ToolCallStatus::Completed,
        Some("error") => ToolCallStatus::Failed,
        _ => ToolCallStatus::InProgress,
    }
}

fn model_state_from_models(
    models: &[CodyModelAvailability],
    current: Option<ModelId>,
) -> Result<SessionModelState, Error> {
    let mut available_models = Vec::new();
    for entry in models {
        let model_id = ModelId::new(entry.model.id.clone());
        let mut info = ModelInfo::new(model_id.clone(), entry.model.title.clone());
        if let Some(description) = entry.model.description.clone() {
            info = info.description(description);
        }
        available_models.push(info);
    }

    let selected = if let Some(current) = current {
        current
    } else {
        models
            .iter()
            .find(|entry| entry.is_model_available != Some(false))
            .map(|entry| ModelId::new(entry.model.id.clone()))
            .or_else(|| models.first().map(|entry| ModelId::new(entry.model.id.clone())))
            .ok_or_else(|| {
                Error::internal_error().data("No Cody models returned".to_string())
            })?
    };

    Ok(SessionModelState::new(selected, available_models))
}

fn internal_error(err: impl std::fmt::Display) -> Error {
    Error::internal_error().data(err.to_string())
}
