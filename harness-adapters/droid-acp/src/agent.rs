use crate::events::{parse_event, DroidEvent};
use crate::mapping::{
    extract_location, raw_value_text, text_content, tool_kind_from_name, tool_title,
};
use agent_client_protocol as acp;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, watch, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoLevel {
    Low,
    Medium,
    High,
}

impl AutoLevel {
    fn as_str(&self) -> &'static str {
        match self {
            AutoLevel::Low => "low",
            AutoLevel::Medium => "medium",
            AutoLevel::High => "high",
        }
    }
}

#[derive(Clone, Debug)]
struct SessionMode {
    use_spec: bool,
    auto: Option<AutoLevel>,
}

impl SessionMode {
    fn read_only() -> Self {
        Self {
            use_spec: false,
            auto: None,
        }
    }
}

struct ModeSpec {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    mode: SessionMode,
}

const MODES: &[ModeSpec] = &[
    ModeSpec {
        id: "read_only",
        name: "Read Only",
        description: "Run in read-only mode without auto approvals.",
        mode: SessionMode {
            use_spec: false,
            auto: None,
        },
    },
    ModeSpec {
        id: "auto_low",
        name: "Auto Low",
        description: "Allow low-risk edits with auto approvals.",
        mode: SessionMode {
            use_spec: false,
            auto: Some(AutoLevel::Low),
        },
    },
    ModeSpec {
        id: "auto_medium",
        name: "Auto Medium",
        description: "Allow development operations with auto approvals.",
        mode: SessionMode {
            use_spec: false,
            auto: Some(AutoLevel::Medium),
        },
    },
    ModeSpec {
        id: "auto_high",
        name: "Auto High",
        description: "Allow higher-risk operations with auto approvals.",
        mode: SessionMode {
            use_spec: false,
            auto: Some(AutoLevel::High),
        },
    },
    ModeSpec {
        id: "spec",
        name: "Spec Mode",
        description: "Start in spec mode before executing actions.",
        mode: SessionMode {
            use_spec: true,
            auto: None,
        },
    },
];

#[derive(Debug)]
struct SessionState {
    cwd: PathBuf,
    mode: SessionMode,
    model: Option<String>,
    droid_session_id: Option<String>,
    prompt_active: bool,
    cancel_tx: Option<watch::Sender<bool>>,
}

impl SessionState {
    fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            mode: SessionMode::read_only(),
            model: None,
            droid_session_id: None,
            prompt_active: false,
            cancel_tx: None,
        }
    }
}

#[derive(Clone, Debug)]
struct SessionSnapshot {
    cwd: PathBuf,
    mode: SessionMode,
    model: Option<String>,
    droid_session_id: Option<String>,
}

pub struct DroidAcpAgent {
    droid_path: String,
    default_model: Option<String>,
    session_update_tx: mpsc::UnboundedSender<(acp::SessionNotification, oneshot::Sender<()>)>,
    sessions: Arc<Mutex<HashMap<String, SessionState>>>,
    next_session_id: AtomicU64,
}

impl DroidAcpAgent {
    fn should_drop_text_block_for_droid(text: &str) -> bool {
        text.contains("The commands below were executed at the start of all sessions")
            || text.contains("TodoWrite was not called yet")
    }

    fn normalize_text_block_for_droid(text: String) -> Option<String> {
        if Self::should_drop_text_block_for_droid(&text) {
            return None;
        }
        let normalized = text.replace(
            "Always prefer using the absolute paths when using tools, to avoid any ambiguity.",
            "Always prefer using paths relative to the current working directory when using tools.",
        );
        let trimmed = normalized.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn droid_path_guidance() -> &'static str {
        "Droid is already running with the correct current working directory. Use paths relative to that working directory for all tool arguments. Do not synthesize or prepend absolute paths unless the user explicitly provided an absolute path."
    }

    pub fn new(
        droid_path: String,
        default_model: Option<String>,
        session_update_tx: mpsc::UnboundedSender<(acp::SessionNotification, oneshot::Sender<()>)>,
    ) -> Self {
        Self {
            droid_path,
            default_model,
            session_update_tx,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            next_session_id: AtomicU64::new(1),
        }
    }

    fn agent_info() -> acp::Implementation {
        acp::Implementation {
            name: "droid-acp".to_string(),
            title: Some("Droid ACP Adapter".to_string()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    fn next_session_id(&self) -> acp::SessionId {
        let id = self.next_session_id.fetch_add(1, Ordering::Relaxed);
        acp::SessionId(format!("droid-session-{id}").into())
    }

    fn mode_state(current: &SessionMode) -> acp::SessionModeState {
        let available_modes = MODES
            .iter()
            .map(|spec| acp::SessionMode {
                id: acp::SessionModeId(spec.id.into()),
                name: spec.name.to_string(),
                description: Some(spec.description.to_string()),
                meta: None,
            })
            .collect::<Vec<_>>();

        let current_id = MODES
            .iter()
            .find(|spec| spec.mode.auto == current.auto && spec.mode.use_spec == current.use_spec)
            .map(|spec| spec.id)
            .unwrap_or("read_only");

        acp::SessionModeState {
            current_mode_id: acp::SessionModeId(current_id.into()),
            available_modes,
            meta: None,
        }
    }

    async fn send_update(
        &self,
        session_id: &acp::SessionId,
        update: acp::SessionUpdate,
    ) -> Result<(), acp::Error> {
        let (tx, rx) = oneshot::channel();
        self.session_update_tx
            .send((
                acp::SessionNotification {
                    session_id: session_id.clone(),
                    update,
                    meta: None,
                },
                tx,
            ))
            .map_err(|_| acp::Error::internal_error())?;
        rx.await.map_err(|_| acp::Error::internal_error())?;
        Ok(())
    }

    fn prompt_to_text(prompt: Vec<acp::ContentBlock>) -> String {
        let mut parts = Vec::new();
        for block in prompt {
            match block {
                acp::ContentBlock::Text(text) => {
                    if let Some(normalized) = Self::normalize_text_block_for_droid(text.text) {
                        parts.push(normalized);
                    }
                }
                acp::ContentBlock::Image(image) => {
                    let label = image
                        .uri
                        .as_ref()
                        .map(|uri| format!("[image: {uri}]"))
                        .unwrap_or_else(|| "[image omitted]".to_string());
                    parts.push(label);
                }
                acp::ContentBlock::Audio(_) => {
                    parts.push("[audio omitted]".to_string());
                }
                acp::ContentBlock::ResourceLink(link) => {
                    parts.push(format!("[resource: {}]({})", link.name, link.uri));
                }
                acp::ContentBlock::Resource(resource) => match resource.resource {
                    acp::EmbeddedResourceResource::TextResourceContents(text_resource) => {
                        parts.push(format!(
                            "[resource: {}]\n{}",
                            text_resource.uri, text_resource.text
                        ));
                    }
                    acp::EmbeddedResourceResource::BlobResourceContents(blob_resource) => {
                        parts.push(format!("[resource: {}]", blob_resource.uri));
                    }
                },
            }
        }

        parts.push(Self::droid_path_guidance().to_string());
        parts.join("\n\n")
    }

    fn build_command(&self, prompt: &str, state: &SessionSnapshot) -> Command {
        let mut cmd = Command::new(&self.droid_path);
        cmd.arg("exec")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--cwd")
            .arg(&state.cwd);

        if let Some(session_id) = &state.droid_session_id {
            cmd.arg("--session-id").arg(session_id);
        }

        if state.mode.use_spec {
            cmd.arg("--use-spec");
        }

        if let Some(auto) = state.mode.auto {
            cmd.arg("--auto").arg(auto.as_str());
        }

        if let Some(model) = state.model.as_ref().or(self.default_model.as_ref()) {
            cmd.arg("--model").arg(model);
        }

        cmd.arg(prompt);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        cmd
    }

    async fn handle_tool_call(
        &self,
        session_id: &acp::SessionId,
        cwd: &Path,
        tool_call: DroidEvent,
    ) -> Result<(), acp::Error> {
        let DroidEvent::ToolCall {
            id,
            tool_id,
            tool_name,
            parameters,
            ..
        } = tool_call
        else {
            return Ok(());
        };

        let name_for_kind = tool_name.as_deref().or(tool_id.as_deref());
        let kind = name_for_kind
            .map(tool_kind_from_name)
            .unwrap_or(acp::ToolKind::Other);

        let locations = parameters
            .as_ref()
            .and_then(|params| extract_location(params, Some(cwd)))
            .map(|location| vec![location])
            .unwrap_or_default();

        let tool_call = acp::ToolCall {
            id: acp::ToolCallId(id.into()),
            title: tool_title(tool_name.as_deref(), tool_id.as_deref()),
            kind,
            status: acp::ToolCallStatus::InProgress,
            content: Vec::new(),
            locations,
            raw_input: parameters,
            raw_output: None,
            meta: None,
        };

        self.send_update(session_id, acp::SessionUpdate::ToolCall(tool_call))
            .await
    }

    async fn handle_tool_result(
        &self,
        session_id: &acp::SessionId,
        tool_result: DroidEvent,
    ) -> Result<(), acp::Error> {
        let DroidEvent::ToolResult {
            id,
            is_error,
            value,
            ..
        } = tool_result
        else {
            return Ok(());
        };

        let status = if is_error.unwrap_or(false) {
            acp::ToolCallStatus::Failed
        } else {
            acp::ToolCallStatus::Completed
        };

        let content = value
            .as_ref()
            .map(|output| vec![text_content(raw_value_text(output))]);

        let update = acp::ToolCallUpdate {
            id: acp::ToolCallId(id.into()),
            meta: None,
            fields: acp::ToolCallUpdateFields {
                kind: None,
                status: Some(status),
                title: None,
                content,
                locations: None,
                raw_input: None,
                raw_output: value,
            },
        };

        self.send_update(session_id, acp::SessionUpdate::ToolCallUpdate(update))
            .await
    }

    async fn run_prompt(
        &self,
        session_id: &acp::SessionId,
        prompt: String,
        state_snapshot: SessionSnapshot,
        mut cancel_rx: watch::Receiver<bool>,
    ) -> Result<(acp::StopReason, Option<String>), acp::Error> {
        let mut cmd = self.build_command(&prompt, &state_snapshot);
        let mut child = cmd
            .spawn()
            .map_err(|err| acp::Error::internal_error().with_data(err.to_string()))?;

        let stdout = child.stdout.take().ok_or_else(acp::Error::internal_error)?;
        let stderr = child.stderr.take().ok_or_else(acp::Error::internal_error)?;

        let cwd = state_snapshot.cwd.clone();
        let mut lines = BufReader::new(stdout).lines();

        let mut droid_session_id: Option<String> = None;
        let mut cancelled = false;

        tokio::spawn(async move {
            let mut stderr_lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = stderr_lines.next_line().await {
                tracing::debug!("droid exec stderr: {line}");
            }
        });

        loop {
            tokio::select! {
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        cancelled = true;
                        let _ = child.kill().await;
                        break;
                    }
                }
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }

                            let parsed: Value = match serde_json::from_str(trimmed) {
                                Ok(value) => value,
                                Err(err) => {
                                    tracing::warn!("Failed to parse Droid output line: {err}");
                                    continue;
                                }
                            };

                            let Some(envelope) = parse_event(parsed) else {
                                tracing::debug!("Ignoring Droid output line: {trimmed}");
                                continue;
                            };

                            match envelope.event {
                                Some(DroidEvent::System { session_id: sid, .. }) => {
                                    if sid.is_some() {
                                        droid_session_id = sid;
                                    }
                                }
                                Some(DroidEvent::Message { role, text, .. }) => {
                                    if role == "assistant" {
                                        self.send_update(
                                            session_id,
                                            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk {
                                                content: acp::ContentBlock::Text(acp::TextContent {
                                                    text,
                                                    annotations: None,
                                                    meta: None,
                                                }),
                                                meta: None,
                                            }),
                                        ).await?;
                                    }
                                }
                                Some(event @ DroidEvent::ToolCall { .. }) => {
                                    self.handle_tool_call(session_id, &cwd, event).await?;
                                }
                                Some(event @ DroidEvent::ToolResult { .. }) => {
                                    self.handle_tool_result(session_id, event).await?;
                                }
                                Some(DroidEvent::Completion { final_text: text, session_id: sid }) => {
                                    if let Some(text) = text {
                                        self.send_update(
                                            session_id,
                                            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk {
                                                content: acp::ContentBlock::Text(acp::TextContent {
                                                    text,
                                                    annotations: None,
                                                    meta: None,
                                                }),
                                                meta: None,
                                            }),
                                        ).await?;
                                    }
                                    if sid.is_some() {
                                        droid_session_id = sid;
                                    }
                                }
                                Some(DroidEvent::Error { message }) => {
                                    if let Some(message) = message {
                                        self.send_update(
                                            session_id,
                                            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk {
                                                content: acp::ContentBlock::Text(acp::TextContent {
                                                    text: format!("Droid error: {message}"),
                                                    annotations: None,
                                                    meta: None,
                                                }),
                                                meta: None,
                                            }),
                                        ).await?;
                                    }
                                }
                                None => {
                                    tracing::debug!("Unhandled Droid event: {trimmed}");
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(err) => {
                            tracing::warn!("Failed to read Droid output: {err}");
                            break;
                        }
                    }
                }
            }
        }

        let status = child.wait().await;
        if let Ok(status) = status {
            if !status.success() && !cancelled {
                let message = format!("Droid exec exited with status {status}");
                self.send_update(
                    session_id,
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk {
                        content: acp::ContentBlock::Text(acp::TextContent {
                            text: message,
                            annotations: None,
                            meta: None,
                        }),
                        meta: None,
                    }),
                )
                .await?;
            }
        }

        if cancelled {
            return Ok((acp::StopReason::Cancelled, droid_session_id));
        }

        Ok((acp::StopReason::EndTurn, droid_session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_block(text: &str) -> acp::ContentBlock {
        acp::ContentBlock::Text(acp::TextContent {
            text: text.to_string(),
            annotations: None,
            meta: None,
        })
    }

    #[test]
    fn prompt_to_text_appends_relative_path_guidance() {
        let prompt = vec![text_block("Create hello-droid.md in the workspace root.")];

        let rendered = DroidAcpAgent::prompt_to_text(prompt);

        assert!(rendered.contains("Create hello-droid.md in the workspace root."));
        assert!(rendered.contains(DroidAcpAgent::droid_path_guidance()));
        assert!(rendered.ends_with(DroidAcpAgent::droid_path_guidance()));
    }

    #[test]
    fn prompt_to_text_strips_ctx_bootstrap_system_reminders() {
        let prompt = vec![
            text_block("<system-reminder>The commands below were executed at the start of all sessions\n% pwd\n/tmp/project</system-reminder>"),
            text_block("<system-reminder>IMPORTANT: TodoWrite was not called yet.</system-reminder>"),
            text_block("Create hello-droid.md in the workspace root."),
        ];

        let rendered = DroidAcpAgent::prompt_to_text(prompt);

        assert!(!rendered.contains("The commands below were executed at the start of all sessions"));
        assert!(!rendered.contains("TodoWrite was not called yet"));
        assert!(rendered.contains("Create hello-droid.md in the workspace root."));
    }

    #[test]
    fn prompt_to_text_rewrites_absolute_path_tool_guidance() {
        let original_guidance =
            "Always prefer using the absolute paths when using tools, to avoid any ambiguity.";
        let prompt = vec![text_block(
            original_guidance,
        )];

        let rendered = DroidAcpAgent::prompt_to_text(prompt);

        assert!(!rendered.contains(original_guidance));
        assert!(rendered.contains("relative to the current working directory"));
        assert!(rendered.contains("Do not synthesize or prepend absolute paths"));
    }

    #[test]
    fn prompt_to_text_ends_with_hard_relative_path_rule() {
        let prompt = vec![
            text_block(
                "Always prefer using the absolute paths when using tools, to avoid any ambiguity.",
            ),
            text_block("Create hello-droid.md in the workspace root with text hi."),
        ];

        let rendered = DroidAcpAgent::prompt_to_text(prompt);

        assert!(rendered.contains("Create hello-droid.md in the workspace root with text hi."));
        assert!(rendered.ends_with(DroidAcpAgent::droid_path_guidance()));
        assert!(rendered.contains("Do not synthesize or prepend absolute paths"));
    }
}

#[async_trait(?Send)]
impl acp::Agent for DroidAcpAgent {
    async fn initialize(
        &self,
        _arguments: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse, acp::Error> {
        Ok(acp::InitializeResponse {
            protocol_version: acp::V1,
            agent_capabilities: acp::AgentCapabilities {
                load_session: false,
                mcp_capabilities: acp::McpCapabilities {
                    http: false,
                    sse: false,
                    meta: None,
                },
                prompt_capabilities: acp::PromptCapabilities {
                    image: false,
                    audio: false,
                    embedded_context: true,
                    meta: None,
                },
                meta: Some(json!({"modes": {"supportsSessionModes": true}})),
            },
            auth_methods: Vec::new(),
            agent_info: Some(Self::agent_info()),
            meta: None,
        })
    }

    async fn authenticate(
        &self,
        _arguments: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse, acp::Error> {
        Ok(acp::AuthenticateResponse { meta: None })
    }

    async fn new_session(
        &self,
        arguments: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse, acp::Error> {
        let session_id = self.next_session_id();
        let cwd = arguments.cwd.clone();
        let state = SessionState::new(cwd);
        let mode_state = Self::mode_state(&state.mode);

        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id.0.to_string(), state);

        Ok(acp::NewSessionResponse {
            session_id,
            modes: Some(mode_state),
            models: None,
            meta: None,
        })
    }

    async fn load_session(
        &self,
        _arguments: acp::LoadSessionRequest,
    ) -> Result<acp::LoadSessionResponse, acp::Error> {
        Err(acp::Error::resource_not_found(Some(
            "Droid ACP adapter does not support session load".to_string(),
        )))
    }

    async fn prompt(
        &self,
        arguments: acp::PromptRequest,
    ) -> Result<acp::PromptResponse, acp::Error> {
        let session_id = arguments.session_id.clone();
        let prompt_text = Self::prompt_to_text(arguments.prompt);

        let (state_snapshot, cancel_rx) = {
            let mut sessions = self.sessions.lock().await;
            let state = sessions
                .get_mut(session_id.0.as_ref())
                .ok_or_else(|| acp::Error::resource_not_found(None))?;

            if state.prompt_active {
                return Err(acp::Error::invalid_params());
            }

            state.prompt_active = true;
            let (cancel_tx, cancel_rx) = watch::channel(false);
            state.cancel_tx = Some(cancel_tx);
            let snapshot = SessionSnapshot {
                cwd: state.cwd.clone(),
                mode: state.mode.clone(),
                model: state.model.clone(),
                droid_session_id: state.droid_session_id.clone(),
            };
            (snapshot, cancel_rx)
        };

        let result = self
            .run_prompt(&session_id, prompt_text, state_snapshot, cancel_rx)
            .await;

        let mut sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get_mut(session_id.0.as_ref()) {
            state.prompt_active = false;
            state.cancel_tx = None;
            if let Ok((_, droid_session_id)) = &result {
                if droid_session_id.is_some() {
                    state.droid_session_id = droid_session_id.clone();
                }
            }
        }

        let (stop_reason, _) = result?;

        Ok(acp::PromptResponse {
            stop_reason,
            meta: None,
        })
    }

    async fn cancel(&self, args: acp::CancelNotification) -> Result<(), acp::Error> {
        let sessions = self.sessions.lock().await;
        if let Some(state) = sessions.get(args.session_id.0.as_ref()) {
            if let Some(cancel_tx) = &state.cancel_tx {
                let _ = cancel_tx.send(true);
            }
        }

        Ok(())
    }

    async fn set_session_mode(
        &self,
        args: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse, acp::Error> {
        let mut sessions = self.sessions.lock().await;
        let state = sessions
            .get_mut(args.session_id.0.as_ref())
            .ok_or_else(|| acp::Error::resource_not_found(None))?;

        let Some(spec) = MODES.iter().find(|spec| spec.id == args.mode_id.0.as_ref()) else {
            return Err(acp::Error::invalid_params());
        };

        state.mode = spec.mode.clone();

        self.send_update(
            &args.session_id,
            acp::SessionUpdate::CurrentModeUpdate(acp::CurrentModeUpdate {
                current_mode_id: args.mode_id.clone(),
                meta: None,
            }),
        )
        .await?;

        Ok(acp::SetSessionModeResponse { meta: None })
    }

    async fn set_session_model(
        &self,
        args: acp::SetSessionModelRequest,
    ) -> Result<acp::SetSessionModelResponse, acp::Error> {
        let mut sessions = self.sessions.lock().await;
        let state = sessions
            .get_mut(args.session_id.0.as_ref())
            .ok_or_else(|| acp::Error::resource_not_found(None))?;

        let model_id = args.model_id.to_string();
        state.model = Some(model_id.clone());

        Ok(acp::SetSessionModelResponse {
            meta: Some(json!({
                "model": {
                    "id": model_id,
                    "display_name": model_id,
                }
            })),
        })
    }

    async fn ext_method(&self, args: acp::ExtRequest) -> Result<acp::ExtResponse, acp::Error> {
        tracing::debug!("Ignoring ext method {}", args.method);
        let raw = serde_json::value::to_raw_value(&json!({}))
            .map_err(|err| acp::Error::internal_error().with_data(err.to_string()))?;
        Ok(raw.into())
    }

    async fn ext_notification(&self, args: acp::ExtNotification) -> Result<(), acp::Error> {
        tracing::debug!("Ignoring ext notification {}", args.method);
        Ok(())
    }
}
