use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_client_protocol::{
    Agent, AuthMethod, AuthenticateRequest, CancelNotification, Client, ClientCapabilities,
    ClientSideConnection, ContentBlock, EnvVariable, ErrorCode, ImageContent, Implementation,
    InitializeRequest, McpServer, McpServerStdio, NewSessionRequest, PermissionOptionKind,
    PromptRequest, ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, ResourceLink, SelectedPermissionOutcome, SessionModeState,
    SessionNotification, SessionUpdate, SetSessionModeRequest, SetSessionModelRequest, TextContent,
};
use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::crp::{
    CrpChannel, CrpCommand, CrpEnvelope, CrpEvent, CrpMcpServerConfig, CrpModelInfo, CrpTurnError,
    CrpTurnStatus, CrpWriter,
};
use crate::translate::Translator;

struct SessionState {
    translator: Translator,
    cwd: PathBuf,
    model_catalog: ModelCatalogState,
    active_turn_id: Option<String>,
    last_turn_update_at: Option<Instant>,
    turn_update_count: u64,
}

#[derive(Debug, Clone, Default)]
struct ModelCatalogState {
    payload: Option<Value>,
    models: Vec<CrpModelInfo>,
    current_model_id: Option<String>,
    catalog_source: Option<String>,
}

#[derive(Default)]
struct BridgeState {
    auth_methods: Vec<AuthMethod>,
    provider_id: Option<String>,
    prompt_image_supported: bool,
}

#[derive(Default)]
struct Sessions {
    by_acp: HashMap<String, SessionState>,
    by_crp: HashMap<String, String>,
}

struct BridgeClient {
    events_tx: mpsc::Sender<CrpEnvelope>,
    sessions: Arc<Mutex<Sessions>>,
}

fn bridge_trace_filter_matches(filter: Option<&str>, provider_id: Option<&str>) -> bool {
    let Some(filter) = filter.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    if filter == "*" {
        return true;
    }
    provider_id.is_some_and(|provider| provider.eq_ignore_ascii_case(filter))
}

fn bridge_trace_enabled(provider_id: Option<&str>) -> bool {
    if std::env::var("CTX_ACP_BRIDGE_TRACE")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
    {
        return true;
    }
    bridge_trace_filter_matches(
        std::env::var("CTX_ACP_BRIDGE_TRACE_PROVIDER")
            .ok()
            .as_deref(),
        provider_id,
    )
}

fn bridge_trace_enabled_from_env() -> bool {
    let provider_id = std::env::var("CTX_PROVIDER_ID").ok();
    bridge_trace_enabled(provider_id.as_deref())
}

fn session_update_kind(update: &SessionUpdate) -> &'static str {
    match update {
        SessionUpdate::AgentMessageChunk(_) => "agent_message_chunk",
        SessionUpdate::AgentThoughtChunk(_) => "agent_thought_chunk",
        SessionUpdate::ToolCall(_) => "tool_call",
        SessionUpdate::ToolCallUpdate(_) => "tool_call_update",
        _ => "other",
    }
}

fn raw_session_update_kind(line: &str) -> Option<String> {
    let msg: Value = serde_json::from_str(line).ok()?;
    let method = msg.get("method").and_then(Value::as_str)?;
    if method != "session/update" {
        return Some(method.to_string());
    }
    msg.pointer("/params/update/sessionUpdate")
        .and_then(Value::as_str)
        .map(|value| format!("session/update:{value}"))
        .or_else(|| Some("session/update".to_string()))
}

#[async_trait::async_trait(?Send)]
impl Client for BridgeClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> agent_client_protocol::Result<RequestPermissionResponse> {
        let option_id = args
            .options
            .iter()
            .find(|option| {
                matches!(
                    option.kind,
                    PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                )
            })
            .or_else(|| args.options.first())
            .map(|option| option.option_id.clone());

        let outcome = match option_id {
            Some(option_id) => {
                RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(option_id))
            }
            None => RequestPermissionOutcome::Cancelled,
        };

        Ok(RequestPermissionResponse::new(outcome))
    }

    async fn session_notification(
        &self,
        args: SessionNotification,
    ) -> agent_client_protocol::Result<()> {
        let mut sessions = self.sessions.lock().await;
        let session_id = args.session_id.to_string();
        if let Some(state) = sessions.by_acp.get_mut(&session_id) {
            let trace_enabled = bridge_trace_enabled_from_env();
            let update_kind = session_update_kind(&args.update);
            let events = state.translator.apply_update(args.update);
            if state.active_turn_id.is_some() && !events.is_empty() {
                state.last_turn_update_at = Some(Instant::now());
                state.turn_update_count =
                    state.turn_update_count.saturating_add(events.len() as u64);
            }
            if trace_enabled {
                info!(
                    session_id = %session_id,
                    update_kind,
                    emitted_events = events.len(),
                    active_turn_id = ?state.active_turn_id,
                    has_buffered_message = state.translator.has_buffered_message(),
                    turn_update_count = state.turn_update_count,
                    "acp session notification"
                );
            }
            for event in events {
                let _ = self.events_tx.send(event).await;
            }
        }
        Ok(())
    }
}

fn normalize_agent_message_block(value: Value) -> Value {
    match value {
        Value::String(text) => json!({"type":"text","text": text}),
        Value::Object(obj) => {
            if obj.get("type").is_some() {
                return Value::Object(obj);
            }
            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                return json!({"type":"text","text": text});
            }
            if let Some(text) = obj.get("content").and_then(|v| v.as_str()) {
                return json!({"type":"text","text": text});
            }
            Value::Object(obj)
        }
        other => other,
    }
}

fn rewrite_session_update_line(line: &str) -> Option<Vec<String>> {
    let msg: Value = serde_json::from_str(line).ok()?;
    let method = msg.get("method").and_then(|v| v.as_str())?;
    if method != "session/update" {
        return None;
    }
    let update = msg.pointer("/params/update")?;
    let kind = update.get("sessionUpdate").and_then(|v| v.as_str())?;
    if kind == "usage_update" {
        return Some(Vec::new());
    }
    if kind != "agent_message" {
        return None;
    }
    let mut blocks = Vec::new();
    match update.get("content") {
        Some(Value::Array(items)) => {
            for item in items {
                blocks.push(normalize_agent_message_block(item.clone()));
            }
        }
        Some(value) => {
            blocks.push(normalize_agent_message_block(value.clone()));
        }
        None => return None,
    }
    if blocks.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for block in blocks {
        let mut next = msg.clone();
        if let Some(update_obj) = next
            .pointer_mut("/params/update")
            .and_then(|v| v.as_object_mut())
        {
            update_obj.insert(
                "sessionUpdate".to_string(),
                Value::String("agent_message_chunk".to_string()),
            );
            update_obj.insert("content".to_string(), block);
        }
        if let Ok(serialized) = serde_json::to_string(&next) {
            out.push(serialized);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn acp_session_models_to_catalog(
    models: Option<&agent_client_protocol::SessionModelState>,
) -> ModelCatalogState {
    let Some(models) = models else {
        return ModelCatalogState::default();
    };

    let current_model_id = Some(models.current_model_id.to_string());
    let available_models = models
        .available_models
        .iter()
        .map(|model| CrpModelInfo {
            id: model.model_id.to_string(),
            name: Some(model.name.clone()),
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_value(models).ok();
    let catalog_source = (current_model_id.is_some() || !available_models.is_empty())
        .then(|| "live_remote".to_string());

    ModelCatalogState {
        payload,
        models: available_models,
        current_model_id,
        catalog_source,
    }
}

async fn forward_acp_stdout(
    child_stdout: tokio::process::ChildStdout,
    proxy: tokio::io::DuplexStream,
) -> Result<()> {
    let mut lines = BufReader::new(child_stdout).lines();
    let mut writer = BufWriter::new(proxy);
    let trace_enabled = bridge_trace_enabled_from_env();
    while let Some(line) = lines.next_line().await? {
        if trace_enabled {
            if let Some(kind) = raw_session_update_kind(&line) {
                info!(kind = %kind, "acp stdout");
            }
        }
        if let Some(rewrites) = rewrite_session_update_line(&line) {
            if rewrites.is_empty() {
                continue;
            }
            if trace_enabled {
                info!(rewrites = rewrites.len(), "acp stdout rewrite");
            }
            for entry in rewrites {
                writer.write_all(entry.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
        } else {
            writer.write_all(line.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    writer.flush().await?;
    Ok(())
}

pub async fn run_bridge(config: Config) -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut command = Command::new(&config.command);
    command
        .args(&config.args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    if let Some(cwd) = &config.cwd {
        command.current_dir(cwd);
    }
    if !config.env.is_empty() {
        command.envs(&config.env);
    }

    let mut child = command.spawn().context("spawn acp harness")?;
    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("acp stdin unavailable"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("acp stdout unavailable"))?;

    let (events_tx, mut events_rx) = mpsc::channel::<CrpEnvelope>(256);
    let sessions = Arc::new(Mutex::new(Sessions::default()));
    let bridge_state = Arc::new(Mutex::new(BridgeState::default()));

    let client = BridgeClient {
        events_tx: events_tx.clone(),
        sessions: Arc::clone(&sessions),
    };

    let local = LocalSet::new();
    local
        .run_until(async move {
            let (stdout_reader, stdout_writer) = tokio::io::duplex(64 * 1024);
            let stdout_task = tokio::task::spawn_local(async move {
                if let Err(err) = forward_acp_stdout(child_stdout, stdout_writer).await {
                    error!("acp stdout forwarder failed: {err}");
                }
            });

            let (acp, io_task) = ClientSideConnection::new(
                client,
                child_stdin.compat_write(),
                stdout_reader.compat(),
                |fut| {
                    tokio::task::spawn_local(fut);
                },
            );

            tokio::task::spawn_local(async move {
                if let Err(err) = io_task.await {
                    error!("acp io task failed: {err}");
                }
            });

            let init = acp_initialize_request();
            let init_response = match acp.initialize(init).await {
                Ok(response) => response,
                Err(err) => {
                    return Err(anyhow!("acp initialize failed: {err}"));
                }
            };
            {
                let mut state = bridge_state.lock().await;
                state.auth_methods = init_response.auth_methods;
                state.provider_id = std::env::var("CTX_PROVIDER_ID").ok();
                state.prompt_image_supported =
                    init_response.agent_capabilities.prompt_capabilities.image;
            }

            let mut writer = CrpWriter::new(tokio::io::stdout());
            let writer_task = tokio::task::spawn_local(async move {
                while let Some(event) = events_rx.recv().await {
                    if let Err(err) = writer.send(&event).await {
                        error!("failed to write crp event: {err}");
                        break;
                    }
                }
            });

            let stdin = tokio::io::stdin();
            let mut lines = BufReader::new(stdin).lines();

            while let Some(line) = lines.next_line().await? {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let command: CrpCommand = match serde_json::from_str(trimmed) {
                    Ok(command) => command,
                    Err(err) => {
                        warn!("invalid crp command: {err}");
                        continue;
                    }
                };

                if let Err(err) =
                    handle_command(command, &acp, &events_tx, &sessions, &bridge_state, &config)
                        .await
                {
                    warn!("crp command error: {err}");
                }
            }

            writer_task.await.ok();
            stdout_task.await.ok();
            Ok(())
        })
        .await
}

fn acp_initialize_request() -> InitializeRequest {
    InitializeRequest::new(ProtocolVersion::LATEST)
        .client_capabilities(ClientCapabilities::default())
        .client_info(Implementation::new("ctx", env!("CARGO_PKG_VERSION")).title("ctx"))
}

fn requested_acp_session_mode() -> Option<String> {
    std::env::var("CTX_PROVIDER_MODE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn apply_model_catalog_current_model(
    model_catalog: &mut ModelCatalogState,
    current_model_id: &str,
) {
    model_catalog.current_model_id = Some(current_model_id.to_string());
    if let Some(payload) = model_catalog
        .payload
        .as_mut()
        .and_then(Value::as_object_mut)
    {
        payload.insert(
            "currentModelId".to_string(),
            Value::String(current_model_id.to_string()),
        );
        payload.insert(
            "current_model_id".to_string(),
            Value::String(current_model_id.to_string()),
        );
    }
}

fn should_apply_requested_session_mode(
    modes: Option<&SessionModeState>,
    requested_mode: &str,
) -> Result<bool> {
    let Some(modes) = modes else {
        anyhow::bail!(
            "CTX_PROVIDER_MODE='{}' requested but ACP agent did not advertise session modes",
            requested_mode
        );
    };

    if modes.current_mode_id.0.as_ref() == requested_mode {
        return Ok(false);
    }

    if modes
        .available_modes
        .iter()
        .any(|mode| mode.id.0.as_ref() == requested_mode)
    {
        return Ok(true);
    }

    let available = modes
        .available_modes
        .iter()
        .map(|mode| mode.id.0.as_ref())
        .collect::<Vec<_>>()
        .join(", ");
    anyhow::bail!(
        "CTX_PROVIDER_MODE='{}' requested but ACP agent advertised modes [{}]",
        requested_mode,
        available
    );
}

async fn maybe_apply_requested_session_model(
    acp: &ClientSideConnection,
    acp_session_id: &str,
    model_catalog: &mut ModelCatalogState,
    requested_model: Option<&str>,
) -> Result<()> {
    let Some(requested_model) = requested_model
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };

    if !should_apply_requested_session_model(model_catalog, requested_model)? {
        return Ok(());
    }

    acp.set_session_model(SetSessionModelRequest::new(
        acp_session_id.to_string(),
        requested_model.to_string(),
    ))
    .await
    .map_err(|err| anyhow!("acp set_session_model '{}' failed: {err}", requested_model))?;
    apply_model_catalog_current_model(model_catalog, requested_model);
    Ok(())
}

fn should_apply_requested_session_model(
    model_catalog: &ModelCatalogState,
    requested_model: &str,
) -> Result<bool> {
    if model_catalog.current_model_id.as_deref() == Some(requested_model) {
        return Ok(false);
    }
    Ok(true)
}

const CLINE_PROMPT_TAIL_FIRST_UPDATE_WAIT: Duration = Duration::from_secs(4);
const CLINE_PROMPT_TAIL_IDLE_SETTLE: Duration = Duration::from_millis(350);
const CLINE_PROMPT_TAIL_MAX_WAIT: Duration = Duration::from_secs(10);
const CLINE_PROMPT_TAIL_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptTailDecision {
    Wait,
    Done,
}

fn should_wait_cline_prompt_tail(provider_id: Option<&str>) -> bool {
    provider_id
        .map(|value| value.eq_ignore_ascii_case("cline"))
        .unwrap_or(false)
}

fn prompt_tail_decision(
    started_at: Instant,
    now: Instant,
    initial_update_count: u64,
    current_update_count: u64,
    last_update_at: Option<Instant>,
    has_buffered_message: bool,
) -> PromptTailDecision {
    let elapsed = now.saturating_duration_since(started_at);
    if elapsed >= CLINE_PROMPT_TAIL_MAX_WAIT {
        return PromptTailDecision::Done;
    }
    if current_update_count > initial_update_count && has_buffered_message {
        if let Some(last_update_at) = last_update_at {
            let idle = now.saturating_duration_since(last_update_at);
            if idle >= CLINE_PROMPT_TAIL_IDLE_SETTLE {
                return PromptTailDecision::Done;
            }
        }
        return PromptTailDecision::Wait;
    }
    if elapsed >= CLINE_PROMPT_TAIL_FIRST_UPDATE_WAIT {
        return PromptTailDecision::Done;
    }
    PromptTailDecision::Wait
}

async fn wait_for_prompt_tail(
    sessions: &Arc<Mutex<Sessions>>,
    acp_session_id: &str,
    turn_id: &str,
    initial_update_count: u64,
) {
    let started_at = Instant::now();
    loop {
        let now = Instant::now();
        let decision = {
            let sessions_guard = sessions.lock().await;
            let Some(state) = sessions_guard.by_acp.get(acp_session_id) else {
                break;
            };
            if state.active_turn_id.as_deref() != Some(turn_id) {
                break;
            }
            prompt_tail_decision(
                started_at,
                now,
                initial_update_count,
                state.turn_update_count,
                state.last_turn_update_at,
                state.translator.has_buffered_message(),
            )
        };
        if decision == PromptTailDecision::Done {
            break;
        }
        tokio::time::sleep(CLINE_PROMPT_TAIL_POLL).await;
    }
}

async fn fail_active_turn(
    events_tx: &mpsc::Sender<CrpEnvelope>,
    sessions: &Arc<Mutex<Sessions>>,
    crp_session_id: &str,
    acp_session_id: &str,
    turn_id: &str,
    message: String,
) {
    let final_event = {
        let mut sessions_guard = sessions.lock().await;
        match sessions_guard.by_acp.get_mut(acp_session_id) {
            Some(state) => {
                let final_event = state.translator.finish_turn();
                state.translator.clear_turn();
                state.active_turn_id = None;
                state.last_turn_update_at = None;
                state.turn_update_count = 0;
                final_event
            }
            None => None,
        }
    };
    if let Some(final_event) = final_event {
        let _ = events_tx.send(final_event).await;
    }
    let _ = events_tx
        .send(CrpEnvelope {
            channel: CrpChannel::Control,
            event: CrpEvent::TurnCompleted {
                session_id: crp_session_id.to_string(),
                turn_id: turn_id.to_string(),
                status: CrpTurnStatus::Error,
                error: Some(CrpTurnError {
                    message,
                    kind: Some("acp_prompt_failed".to_string()),
                    details: None,
                }),
            },
        })
        .await;
}

async fn handle_command(
    command: CrpCommand,
    acp: &ClientSideConnection,
    events_tx: &mpsc::Sender<CrpEnvelope>,
    sessions: &Arc<Mutex<Sessions>>,
    bridge_state: &Arc<Mutex<BridgeState>>,
    config: &Config,
) -> Result<()> {
    match command {
        CrpCommand::SessionOpen {
            session_id,
            config: crp_config,
        } => {
            let crp_session_id = session_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            let requested_model = crp_config
                .as_ref()
                .and_then(|cfg| cfg.model.as_deref())
                .map(str::to_string);
            let cwd = crp_config
                .as_ref()
                .and_then(|cfg| cfg.cwd.clone())
                .or_else(|| config.cwd.clone())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

            let mcp_servers = crp_config
                .and_then(|cfg| cfg.mcp_servers)
                .map(crp_mcp_servers_to_acp)
                .unwrap_or_default();

            let mut new_session_req = NewSessionRequest::new(cwd.clone());
            if !mcp_servers.is_empty() {
                new_session_req = new_session_req.mcp_servers(mcp_servers);
            }

            let response = match acp.new_session(new_session_req).await {
                Ok(response) => response,
                Err(err) => {
                    if err.code == ErrorCode::AuthRequired {
                        emit_auth_required_notice(events_tx, bridge_state, &crp_session_id, err)
                            .await;
                        return Ok(());
                    }
                    return Err(anyhow!("acp new_session failed: {err}"));
                }
            };

            if let Some(mode_id) = requested_acp_session_mode() {
                if should_apply_requested_session_mode(response.modes.as_ref(), &mode_id)? {
                    acp.set_session_mode(SetSessionModeRequest::new(
                        response.session_id.clone(),
                        mode_id.clone(),
                    ))
                    .await
                    .map_err(|err| anyhow!("acp set_session_mode '{}' failed: {err}", mode_id))?;
                }
            }

            let acp_session_id = response.session_id.to_string();
            let mut model_catalog = acp_session_models_to_catalog(response.models.as_ref());
            maybe_apply_requested_session_model(
                acp,
                &acp_session_id,
                &mut model_catalog,
                requested_model.as_deref(),
            )
            .await?;

            let mut sessions_guard = sessions.lock().await;
            if sessions_guard.by_crp.contains_key(&crp_session_id) {
                warn!("session.open ignored: session already active");
                return Ok(());
            }

            let translator = Translator::new(crp_session_id.clone(), config.reasoning_mode);

            sessions_guard
                .by_crp
                .insert(crp_session_id.clone(), acp_session_id.clone());
            sessions_guard.by_acp.insert(
                acp_session_id.clone(),
                SessionState {
                    translator,
                    cwd,
                    model_catalog: model_catalog.clone(),
                    active_turn_id: None,
                    last_turn_update_at: None,
                    turn_update_count: 0,
                },
            );

            let opened = CrpEnvelope {
                channel: CrpChannel::Control,
                event: CrpEvent::SessionOpened {
                    session_id: crp_session_id,
                    provider_session_id: Some(acp_session_id),
                    models: model_catalog.payload,
                    current_model_id: model_catalog.current_model_id,
                },
            };
            let _ = events_tx.send(opened).await;
        }
        CrpCommand::SessionPrompt {
            session_id,
            turn_id,
            items,
            prompt,
            model,
            cwd,
            ..
        } => {
            let crp_session_id =
                session_id.ok_or_else(|| anyhow!("session.prompt missing session_id"))?;

            let wait_for_cline_tail = {
                let state = bridge_state.lock().await;
                should_wait_cline_prompt_tail(state.provider_id.as_deref())
            };
            let mut sessions_guard = sessions.lock().await;
            let acp_session_id = sessions_guard
                .by_crp
                .get(&crp_session_id)
                .cloned()
                .ok_or_else(|| anyhow!("unknown session_id: {crp_session_id}"))?;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("unknown provider session: {acp_session_id}"))?;

            if let Some(cwd) = cwd {
                state.cwd = cwd;
            }
            let prompt_cwd = state.cwd.clone();
            let mut prompt_model_catalog = state.model_catalog.clone();

            let turn_id = turn_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            let message_id = Uuid::new_v4().to_string();
            state.active_turn_id = Some(turn_id.clone());
            state
                .translator
                .start_turn(turn_id.clone(), message_id.clone());
            state.last_turn_update_at = None;
            state.turn_update_count = 0;
            let initial_update_count = state.turn_update_count;

            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::TurnStarted {
                        session_id: crp_session_id.clone(),
                        turn_id: turn_id.clone(),
                    },
                })
                .await;

            drop(sessions_guard);

            maybe_apply_requested_session_model(
                acp,
                &acp_session_id,
                &mut prompt_model_catalog,
                model.as_deref(),
            )
            .await?;

            let mut sessions_guard = sessions.lock().await;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("unknown provider session: {acp_session_id}"))?;
            state.model_catalog = prompt_model_catalog.clone();
            drop(sessions_guard);

            let (trace_enabled, prompt_image_supported) = {
                let state = bridge_state.lock().await;
                (
                    bridge_trace_enabled(state.provider_id.as_deref()),
                    state.prompt_image_supported,
                )
            };
            let prompt_blocks =
                match build_prompt_blocks(prompt, items, Some(&prompt_cwd), prompt_image_supported)
                {
                    Ok(Some(blocks)) => blocks,
                    Ok(None) => {
                        fail_active_turn(
                            events_tx,
                            sessions,
                            &crp_session_id,
                            &acp_session_id,
                            &turn_id,
                            "session.prompt missing prompt".to_string(),
                        )
                        .await;
                        return Ok(());
                    }
                    Err(err) => {
                        fail_active_turn(
                            events_tx,
                            sessions,
                            &crp_session_id,
                            &acp_session_id,
                            &turn_id,
                            err.to_string(),
                        )
                        .await;
                        return Ok(());
                    }
                };
            if trace_enabled {
                info!(
                    session_id = %crp_session_id,
                    provider_session_id = %acp_session_id,
                    turn_id = %turn_id,
                    prompt_blocks = prompt_blocks.len(),
                    cwd = %prompt_cwd.display(),
                    requested_model = ?model,
                    wait_for_prompt_tail = wait_for_cline_tail,
                    "session.prompt start"
                );
            }
            let prompt_req = PromptRequest::new(acp_session_id.clone(), prompt_blocks);
            let response = match acp.prompt(prompt_req).await {
                Ok(response) => response,
                Err(err) => {
                    if trace_enabled {
                        info!(
                            session_id = %crp_session_id,
                            provider_session_id = %acp_session_id,
                            turn_id = %turn_id,
                            error = %err,
                            "session.prompt failed"
                        );
                    }
                    fail_active_turn(
                        events_tx,
                        sessions,
                        &crp_session_id,
                        &acp_session_id,
                        &turn_id,
                        format!("acp prompt failed: {err}"),
                    )
                    .await;
                    return Ok(());
                }
            };
            if trace_enabled {
                info!(
                    session_id = %crp_session_id,
                    provider_session_id = %acp_session_id,
                    turn_id = %turn_id,
                    stop_reason = ?response.stop_reason,
                    "session.prompt response"
                );
            }

            if wait_for_cline_tail {
                wait_for_prompt_tail(sessions, &acp_session_id, &turn_id, initial_update_count)
                    .await;
            }

            let mut sessions_guard = sessions.lock().await;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("missing session after prompt"))?;

            let final_event = state.translator.finish_turn();
            if trace_enabled {
                info!(
                    session_id = %crp_session_id,
                    provider_session_id = %acp_session_id,
                    turn_id = %turn_id,
                    final_event_emitted = final_event.is_some(),
                    has_buffered_message = state.translator.has_buffered_message(),
                    turn_update_count = state.turn_update_count,
                    "session.prompt finalize"
                );
            }
            if let Some(final_event) = final_event {
                let _ = events_tx.send(final_event).await;
            }
            state.translator.clear_turn();
            state.active_turn_id = None;
            state.last_turn_update_at = None;
            state.turn_update_count = 0;

            let status = if response.stop_reason == agent_client_protocol::StopReason::Cancelled {
                CrpTurnStatus::Canceled
            } else {
                CrpTurnStatus::Success
            };

            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::TurnCompleted {
                        session_id: crp_session_id,
                        turn_id,
                        status,
                        error: None,
                    },
                })
                .await;
        }
        CrpCommand::SessionCancel { session_id, .. } => {
            let crp_session_id =
                session_id.ok_or_else(|| anyhow!("session.cancel missing session_id"))?;
            let sessions_guard = sessions.lock().await;
            let acp_session_id = sessions_guard
                .by_crp
                .get(&crp_session_id)
                .cloned()
                .ok_or_else(|| anyhow!("unknown session_id: {crp_session_id}"))?;
            drop(sessions_guard);

            let cancel = CancelNotification::new(acp_session_id);
            let _ = acp.cancel(cancel).await;
        }
        CrpCommand::SessionAuthenticate {
            session_id,
            method_id,
        } => {
            let crp_session_id =
                session_id.ok_or_else(|| anyhow!("session.authenticate missing session_id"))?;

            let method_id = if let Some(method_id) = method_id {
                method_id
            } else {
                let state = bridge_state.lock().await;
                state
                    .auth_methods
                    .first()
                    .map(|method| method.id.to_string())
                    .ok_or_else(|| anyhow!("session.authenticate missing method_id"))?
            };

            let request = AuthenticateRequest::new(method_id);
            match acp.authenticate(request).await {
                Ok(_) => {
                    emit_auth_notice(events_tx, &crp_session_id, "authenticated", None).await;
                }
                Err(err) => {
                    if err.code == ErrorCode::AuthRequired {
                        emit_auth_required_notice(events_tx, bridge_state, &crp_session_id, err)
                            .await;
                    } else {
                        emit_auth_error_notice(events_tx, bridge_state, &crp_session_id, err).await;
                    }
                }
            }
        }
        CrpCommand::SessionSetModel {
            session_id,
            model_id,
        } => {
            let crp_session_id =
                session_id.ok_or_else(|| anyhow!("session.set_model missing session_id"))?;
            let model_id = model_id.ok_or_else(|| anyhow!("session.set_model missing model_id"))?;

            let mut sessions_guard = sessions.lock().await;
            let acp_session_id = sessions_guard
                .by_crp
                .get(&crp_session_id)
                .cloned()
                .ok_or_else(|| anyhow!("unknown session_id: {crp_session_id}"))?;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("unknown provider session: {acp_session_id}"))?;
            let mut next_model_catalog = state.model_catalog.clone();
            drop(sessions_guard);

            match maybe_apply_requested_session_model(
                acp,
                &acp_session_id,
                &mut next_model_catalog,
                Some(model_id.as_str()),
            )
            .await
            {
                Ok(()) => {
                    let mut sessions_guard = sessions.lock().await;
                    let state = sessions_guard
                        .by_acp
                        .get_mut(&acp_session_id)
                        .ok_or_else(|| anyhow!("unknown provider session: {acp_session_id}"))?;
                    state.model_catalog = next_model_catalog;
                    drop(sessions_guard);
                    let _ = events_tx
                        .send(CrpEnvelope {
                            channel: CrpChannel::Control,
                            event: CrpEvent::SessionNotice {
                                session_id: crp_session_id,
                                turn_id: None,
                                code: "session_model_updated".to_string(),
                                severity: Some("info".to_string()),
                                message: Some(format!("session model updated to {model_id}")),
                                details: Some(json!({
                                    "model_id": model_id,
                                    "provider_session_id": acp_session_id,
                                })),
                                transient: Some(false),
                            },
                        })
                        .await;
                }
                Err(err) => {
                    let _ = events_tx
                        .send(CrpEnvelope {
                            channel: CrpChannel::Control,
                            event: CrpEvent::SessionNotice {
                                session_id: crp_session_id,
                                turn_id: None,
                                code: "session_model_update_failed".to_string(),
                                severity: Some("error".to_string()),
                                message: Some(format!(
                                    "acp set_session_model '{model_id}' failed: {err}"
                                )),
                                details: Some(json!({
                                    "model_id": model_id,
                                    "provider_session_id": acp_session_id,
                                })),
                                transient: Some(false),
                            },
                        })
                        .await;
                }
            }
        }
        CrpCommand::ModelsList { config: crp_config } => {
            let cwd = crp_config
                .as_ref()
                .and_then(|cfg| cfg.cwd.clone())
                .or_else(|| config.cwd.clone())
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
            let mcp_servers = crp_config
                .and_then(|cfg| cfg.mcp_servers)
                .map(crp_mcp_servers_to_acp)
                .unwrap_or_default();
            let mut new_session_req = NewSessionRequest::new(cwd);
            if !mcp_servers.is_empty() {
                new_session_req = new_session_req.mcp_servers(mcp_servers);
            }
            let response = acp
                .new_session(new_session_req)
                .await
                .context("acp new_session for models.list")?;
            let model_catalog = acp_session_models_to_catalog(response.models.as_ref());
            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::ModelsList {
                        models: model_catalog.models,
                        current_model_id: model_catalog.current_model_id,
                        catalog_source: model_catalog.catalog_source,
                    },
                })
                .await;
        }
    }

    Ok(())
}

fn crp_mcp_servers_to_acp(servers: HashMap<String, CrpMcpServerConfig>) -> Vec<McpServer> {
    let mut out = Vec::new();
    for (name, config) in servers {
        let Some(command) = config.command else {
            continue;
        };
        let mut server = McpServerStdio::new(name, command);
        if let Some(args) = config.args {
            server = server.args(args);
        }
        if let Some(env) = config.env {
            let env_vars = env
                .into_iter()
                .map(|(key, value)| EnvVariable::new(key, value))
                .collect::<Vec<_>>();
            server = server.env(env_vars);
        }
        out.push(McpServer::Stdio(server));
    }
    out
}

async fn emit_auth_required_notice(
    events_tx: &mpsc::Sender<CrpEnvelope>,
    bridge_state: &Arc<Mutex<BridgeState>>,
    session_id: &str,
    err: agent_client_protocol::Error,
) {
    let (auth_methods, provider_id) = {
        let state = bridge_state.lock().await;
        (state.auth_methods.clone(), state.provider_id.clone())
    };
    let details = auth_notice_details(&auth_methods, provider_id.as_deref(), err.data.as_ref());
    let message = auth_notice_message(&err.message, err.data.as_ref());
    emit_auth_notice(
        events_tx,
        session_id,
        "auth_required",
        Some((message, details)),
    )
    .await;
}

async fn emit_auth_error_notice(
    events_tx: &mpsc::Sender<CrpEnvelope>,
    bridge_state: &Arc<Mutex<BridgeState>>,
    session_id: &str,
    err: agent_client_protocol::Error,
) {
    let (auth_methods, provider_id) = {
        let state = bridge_state.lock().await;
        (state.auth_methods.clone(), state.provider_id.clone())
    };
    let details = auth_notice_details(&auth_methods, provider_id.as_deref(), err.data.as_ref());
    let message = auth_notice_message(&err.message, err.data.as_ref());
    emit_auth_notice(
        events_tx,
        session_id,
        "auth_error",
        Some((message, details)),
    )
    .await;
}

async fn emit_auth_notice(
    events_tx: &mpsc::Sender<CrpEnvelope>,
    session_id: &str,
    code: &str,
    payload: Option<(String, Option<Value>)>,
) {
    let (message, details) = match payload {
        Some((message, details)) => (Some(message), details),
        None => (None, None),
    };

    let notice = CrpEnvelope {
        channel: CrpChannel::Control,
        event: CrpEvent::SessionNotice {
            session_id: session_id.to_string(),
            turn_id: None,
            code: code.to_string(),
            severity: None,
            message,
            details,
            transient: None,
        },
    };
    let _ = events_tx.send(notice).await;
}

fn auth_notice_message(default_message: &str, error_data: Option<&Value>) -> String {
    if let Some(message) = error_data
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            error_data
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
    {
        return message;
    }
    default_message.to_string()
}

fn auth_notice_details(
    auth_methods: &[AuthMethod],
    provider_id: Option<&str>,
    error_data: Option<&Value>,
) -> Option<Value> {
    let mut details = serde_json::Map::new();
    if !auth_methods.is_empty() {
        if let Ok(value) = serde_json::to_value(auth_methods) {
            details.insert("auth_methods".to_string(), value);
        }
    }
    if let Some(provider_id) = provider_id {
        details.insert("provider".to_string(), json!(provider_id));
    }

    if let Some(data) = error_data {
        match data {
            Value::Object(map) => {
                for (key, value) in map {
                    details.insert(key.clone(), value.clone());
                }
            }
            other => {
                details.insert("error_data".to_string(), other.clone());
            }
        }
    }

    if details.is_empty() {
        None
    } else {
        Some(Value::Object(details))
    }
}

fn build_prompt_blocks(
    prompt: Option<String>,
    items: Option<Vec<Value>>,
    cwd: Option<&Path>,
    prompt_image_supported: bool,
) -> Result<Option<Vec<ContentBlock>>> {
    let mut blocks = Vec::new();
    if let Some(items) = items {
        for item in items {
            blocks.extend(crp_item_to_content_blocks(
                &item,
                cwd,
                prompt_image_supported,
            )?);
        }
    }
    if blocks.is_empty() {
        if let Some(prompt) = prompt {
            blocks.push(ContentBlock::Text(TextContent::new(prompt)));
        }
    }
    if blocks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(blocks))
    }
}

fn crp_item_to_content_blocks(
    item: &Value,
    cwd: Option<&Path>,
    prompt_image_supported: bool,
) -> Result<Vec<ContentBlock>> {
    if let Some(text) = item.as_str() {
        return Ok(vec![ContentBlock::Text(TextContent::new(text))]);
    }
    let Some(obj) = item.as_object() else {
        return Ok(Vec::new());
    };

    if let Some(kind) = obj.get("type").and_then(|v| v.as_str()) {
        match kind {
            "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    return Ok(vec![ContentBlock::Text(TextContent::new(text))]);
                }
                anyhow::bail!("CRP text item missing text");
            }
            "image" => {
                require_prompt_image_support(prompt_image_supported)?;
                if let Some(block) = image_block_from_item(obj) {
                    return Ok(vec![block]);
                }
                anyhow::bail!("CRP image item missing data or mime_type");
            }
            "image_ref" => {
                require_prompt_image_support(prompt_image_supported)?;
                return Ok(vec![image_ref_block(obj)?]);
            }
            "local_image" => {
                if let Some(block) = local_image_block(obj, cwd) {
                    return Ok(vec![block]);
                }
                anyhow::bail!("CRP local_image item missing path");
            }
            "skill" => {
                if let Some(block) = skill_block(obj) {
                    return Ok(vec![block]);
                }
                anyhow::bail!("CRP skill item missing name/content");
            }
            _ => {}
        }
    }

    if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
        return Ok(vec![ContentBlock::Text(TextContent::new(text))]);
    }
    if let Some(text) = obj.get("content").and_then(|v| v.as_str()) {
        return Ok(vec![ContentBlock::Text(TextContent::new(text))]);
    }
    if let Some(url_value) = obj.get("image_url") {
        if let Some(url) = url_value.as_str() {
            if parse_data_url(url).is_some() {
                require_prompt_image_support(prompt_image_supported)?;
            }
            return Ok(vec![image_block_from_url(url)]);
        }
        if let Some(url) = url_value.get("url").and_then(|v| v.as_str()) {
            if parse_data_url(url).is_some() {
                require_prompt_image_support(prompt_image_supported)?;
            }
            return Ok(vec![image_block_from_url(url)]);
        }
    }

    Ok(Vec::new())
}

fn image_block_from_item(obj: &serde_json::Map<String, Value>) -> Option<ContentBlock> {
    if let Some(url_value) = obj.get("image_url") {
        if let Some(url) = url_value.as_str() {
            return Some(image_block_from_url(url));
        }
        if let Some(url) = url_value.get("url").and_then(|v| v.as_str()) {
            return Some(image_block_from_url(url));
        }
    }
    let data = obj.get("data").and_then(|v| v.as_str());
    let mime = obj
        .get("mimeType")
        .or_else(|| obj.get("mime_type"))
        .and_then(|v| v.as_str());
    match (data, mime) {
        (Some(data), Some(mime)) => Some(ContentBlock::Image(ImageContent::new(data, mime))),
        _ => None,
    }
}

fn image_block_from_url(url: &str) -> ContentBlock {
    if let Some((mime, data)) = parse_data_url(url) {
        ContentBlock::Image(ImageContent::new(data, mime))
    } else {
        ContentBlock::ResourceLink(ResourceLink::new("image", url.to_string()))
    }
}

fn require_prompt_image_support(prompt_image_supported: bool) -> Result<()> {
    if prompt_image_supported {
        return Ok(());
    }
    anyhow::bail!("ACP agent did not advertise promptCapabilities.image for image prompt items");
}

fn image_ref_block(obj: &serde_json::Map<String, Value>) -> Result<ContentBlock> {
    let blob_id = obj
        .get("blob_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("CRP image_ref item missing blob_id"))?;
    let mime = obj
        .get("mimeType")
        .or_else(|| obj.get("mime_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream");

    let data_root = std::env::var("CTX_DATA_ROOT_HOST")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var("CTX_DATA_ROOT")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| anyhow!("CRP image_ref requires CTX_DATA_ROOT_HOST or CTX_DATA_ROOT"))?;
    let path = Path::new(&data_root).join("blobs").join(blob_id);
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading image blob {} from {}", blob_id, path.display()))?;
    let data = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(ContentBlock::Image(ImageContent::new(data, mime)))
}

fn parse_data_url(raw: &str) -> Option<(String, String)> {
    let data_prefix = "data:";
    if !raw.starts_with(data_prefix) {
        return None;
    }
    let rest = &raw[data_prefix.len()..];
    let (meta, data) = rest.split_once(',')?;
    let mut mime = "application/octet-stream".to_string();
    let mut meta_parts = meta.split(';');
    if let Some(first) = meta_parts.next() {
        if !first.is_empty() {
            mime = first.to_string();
        }
    }
    if !meta.contains("base64") {
        return None;
    }
    Some((mime, data.to_string()))
}

fn local_image_block(
    obj: &serde_json::Map<String, Value>,
    cwd: Option<&Path>,
) -> Option<ContentBlock> {
    let path = obj.get("path").and_then(|v| v.as_str())?;
    let raw_path = PathBuf::from(path);
    let resolved = if raw_path.is_absolute() {
        raw_path
    } else if let Some(cwd) = cwd {
        cwd.join(raw_path)
    } else {
        raw_path
    };
    let name = resolved
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("local_image");
    Some(ContentBlock::ResourceLink(ResourceLink::new(
        name,
        resolved.to_string_lossy().to_string(),
    )))
}

fn skill_block(obj: &serde_json::Map<String, Value>) -> Option<ContentBlock> {
    let name = obj.get("name").and_then(|v| v.as_str());
    let content = obj
        .get("content")
        .and_then(|v| v.as_str())
        .or_else(|| obj.get("text").and_then(|v| v.as_str()));
    if name.is_none() && content.is_none() {
        return None;
    }
    let mut out = String::new();
    if let Some(name) = name {
        out.push_str("Skill: ");
        out.push_str(name);
    }
    if let Some(content) = content {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(content);
    }
    Some(ContentBlock::Text(TextContent::new(out)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn session_mode_state(current: &str, available: &[&str]) -> SessionModeState {
        SessionModeState::new(
            current.to_string(),
            available
                .iter()
                .map(|mode| {
                    agent_client_protocol::SessionMode::new(
                        (*mode).to_string(),
                        (*mode).to_string(),
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn acp_session_models_are_converted_to_crp_catalog() {
        let catalog =
            acp_session_models_to_catalog(Some(&agent_client_protocol::SessionModelState::new(
                "cursor-auto",
                vec![
                    agent_client_protocol::ModelInfo::new("cursor-auto", "Auto"),
                    agent_client_protocol::ModelInfo::new("gpt-5.4", "GPT-5.4"),
                ],
            )));

        assert_eq!(catalog.current_model_id.as_deref(), Some("cursor-auto"));
        assert_eq!(catalog.catalog_source.as_deref(), Some("live_remote"));
        assert_eq!(
            catalog.models,
            vec![
                CrpModelInfo {
                    id: "cursor-auto".to_string(),
                    name: Some("Auto".to_string()),
                },
                CrpModelInfo {
                    id: "gpt-5.4".to_string(),
                    name: Some("GPT-5.4".to_string()),
                },
            ]
        );
        assert_eq!(
            catalog
                .payload
                .as_ref()
                .and_then(|value| value.pointer("/availableModels/0/modelId")),
            Some(&json!("cursor-auto"))
        );
        assert_eq!(
            catalog
                .payload
                .as_ref()
                .and_then(|value| value.pointer("/currentModelId")),
            Some(&json!("cursor-auto"))
        );
    }

    #[test]
    fn requested_session_model_skips_when_already_selected() {
        let catalog =
            acp_session_models_to_catalog(Some(&agent_client_protocol::SessionModelState::new(
                "kimi-k2.5",
                vec![
                    agent_client_protocol::ModelInfo::new("kimi-k2.5", "Kimi K2.5"),
                    agent_client_protocol::ModelInfo::new(
                        "kimi-k2.5-thinking",
                        "Kimi K2.5 Thinking",
                    ),
                ],
            )));

        assert!(!should_apply_requested_session_model(&catalog, "kimi-k2.5")
            .expect("matching model should skip"));
    }

    #[test]
    fn requested_session_model_allows_missing_advertised_models() {
        assert!(
            should_apply_requested_session_model(&ModelCatalogState::default(), "kimi-k2.5")
                .expect("missing model catalog should still forward")
        );
    }

    #[test]
    fn requested_session_model_allows_unadvertised_requested_model() {
        let catalog =
            acp_session_models_to_catalog(Some(&agent_client_protocol::SessionModelState::new(
                "kimi-k2.5",
                vec![
                    agent_client_protocol::ModelInfo::new("kimi-k2.5", "Kimi K2.5"),
                    agent_client_protocol::ModelInfo::new(
                        "kimi-k2.5-thinking",
                        "Kimi K2.5 Thinking",
                    ),
                ],
            )));
        assert!(should_apply_requested_session_model(&catalog, "kimi-k3")
            .expect("unadvertised requested model should still forward"));
    }

    #[test]
    fn apply_model_catalog_current_model_updates_payload_and_current_model() {
        let mut catalog =
            acp_session_models_to_catalog(Some(&agent_client_protocol::SessionModelState::new(
                "kimi-k2.5",
                vec![
                    agent_client_protocol::ModelInfo::new("kimi-k2.5", "Kimi K2.5"),
                    agent_client_protocol::ModelInfo::new(
                        "kimi-k2.5-thinking",
                        "Kimi K2.5 Thinking",
                    ),
                ],
            )));

        apply_model_catalog_current_model(&mut catalog, "kimi-k2.5-thinking");

        assert_eq!(
            catalog.current_model_id.as_deref(),
            Some("kimi-k2.5-thinking")
        );
        assert_eq!(
            catalog
                .payload
                .as_ref()
                .and_then(|value| value.pointer("/currentModelId")),
            Some(&json!("kimi-k2.5-thinking"))
        );
        assert_eq!(
            catalog
                .payload
                .as_ref()
                .and_then(|value| value.pointer("/current_model_id")),
            Some(&json!("kimi-k2.5-thinking"))
        );
    }

    #[test]
    fn initialize_request_sets_non_empty_client_identity() {
        let request = acp_initialize_request();
        let client_info = request.client_info.expect("client info should be present");
        assert_eq!(client_info.name, "ctx");
        assert_eq!(client_info.title.as_deref(), Some("ctx"));
        assert!(!client_info.version.trim().is_empty());
    }

    #[test]
    fn requested_session_mode_requires_session_mode_support() {
        let err = should_apply_requested_session_mode(None, "auto_high")
            .expect_err("missing modes should fail");
        assert!(
            err.to_string().contains("did not advertise session modes"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn requested_session_mode_skips_when_already_selected() {
        let modes = session_mode_state("auto_high", &["read_only", "auto_high"]);
        assert!(
            !should_apply_requested_session_mode(Some(&modes), "auto_high")
                .expect("current mode should be accepted")
        );
    }

    #[test]
    fn requested_session_mode_requires_requested_mode_to_exist() {
        let modes = session_mode_state("read_only", &["read_only", "auto_medium"]);
        let err = should_apply_requested_session_mode(Some(&modes), "auto_high")
            .expect_err("unknown mode should fail");
        assert!(
            err.to_string()
                .contains("advertised modes [read_only, auto_medium]"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn requested_session_mode_is_applied_when_advertised() {
        let modes = session_mode_state("read_only", &["read_only", "auto_high"]);
        assert!(
            should_apply_requested_session_mode(Some(&modes), "auto_high")
                .expect("advertised mode should be accepted")
        );
    }

    #[test]
    fn crp_image_item_maps_to_acp_image_block() {
        let obj = serde_json::json!({
            "type": "image",
            "mime_type": "image/png",
            "data": "AQIDBA=="
        })
        .as_object()
        .cloned()
        .unwrap();
        let block = image_block_from_item(&obj).expect("image block");
        match block {
            ContentBlock::Image(img) => {
                assert_eq!(img.mime_type, "image/png");
                assert_eq!(img.data, "AQIDBA==");
            }
            _ => panic!("expected Image block"),
        }
    }

    #[test]
    fn crp_image_ref_maps_to_acp_image_block_via_blob_bytes() {
        let host = tempdir().expect("host root");
        let blob_dir = host.path().join("blobs");
        fs::create_dir_all(&blob_dir).expect("create blobs dir");
        let blob_id = "blob-xyz";
        let bytes = vec![1u8, 2, 3, 4, 5];
        fs::write(blob_dir.join(blob_id), &bytes).expect("write blob");
        std::env::set_var("CTX_DATA_ROOT_HOST", host.path());

        let obj = serde_json::json!({
            "type": "image_ref",
            "blob_id": blob_id,
            "mime_type": "image/webp"
        })
        .as_object()
        .cloned()
        .unwrap();
        let block = image_ref_block(&obj).expect("image_ref block");
        match block {
            ContentBlock::Image(img) => {
                assert_eq!(img.mime_type, "image/webp");
                assert_eq!(
                    img.data,
                    base64::engine::general_purpose::STANDARD.encode(&bytes)
                );
            }
            _ => panic!("expected Image block"),
        }
    }

    #[test]
    fn build_prompt_blocks_rejects_images_without_agent_image_capability() {
        let err = build_prompt_blocks(
            None,
            Some(vec![serde_json::json!({
                "type": "image_ref",
                "blob_id": "blob-1",
                "mime_type": "image/png"
            })]),
            None,
            false,
        )
        .expect_err("image prompt without capability should fail");

        assert!(
            err.to_string().contains("promptCapabilities.image"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn build_prompt_blocks_allows_local_image_without_image_capability() {
        let blocks = build_prompt_blocks(
            None,
            Some(vec![serde_json::json!({
                "type": "local_image",
                "path": "/tmp/example.png"
            })]),
            None,
            false,
        )
        .expect("local_image should remain compatibility-only")
        .expect("prompt blocks");

        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::ResourceLink(link) => {
                assert_eq!(link.uri, "/tmp/example.png");
            }
            other => panic!("expected resource link, got {other:?}"),
        }
    }

    #[test]
    fn cline_tail_wait_is_provider_scoped() {
        assert!(should_wait_cline_prompt_tail(Some("cline")));
        assert!(should_wait_cline_prompt_tail(Some("CLINE")));
        assert!(!should_wait_cline_prompt_tail(Some("qwen")));
        assert!(!should_wait_cline_prompt_tail(None));
    }

    #[test]
    fn bridge_trace_provider_filter_matches_provider_id() {
        assert!(bridge_trace_filter_matches(
            Some("opencode"),
            Some("opencode")
        ));
        assert!(bridge_trace_filter_matches(
            Some("OPENCODE"),
            Some("opencode")
        ));
        assert!(bridge_trace_filter_matches(Some("*"), Some("qwen")));
        assert!(!bridge_trace_filter_matches(Some("qwen"), Some("opencode")));
        assert!(!bridge_trace_filter_matches(None, Some("opencode")));
    }

    #[test]
    fn prompt_tail_waits_for_first_update() {
        let started_at = Instant::now();
        let now = started_at + Duration::from_secs(1);
        let decision = prompt_tail_decision(started_at, now, 0, 0, None, false);
        assert_eq!(decision, PromptTailDecision::Wait);
    }

    #[test]
    fn prompt_tail_done_when_first_update_window_elapsed() {
        let started_at = Instant::now();
        let now = started_at + CLINE_PROMPT_TAIL_FIRST_UPDATE_WAIT + Duration::from_millis(1);
        let decision = prompt_tail_decision(started_at, now, 0, 0, None, false);
        assert_eq!(decision, PromptTailDecision::Done);
    }

    #[test]
    fn prompt_tail_waits_when_updates_exist_but_message_missing() {
        let started_at = Instant::now();
        let now = started_at + Duration::from_millis(200);
        let decision = prompt_tail_decision(
            started_at,
            now,
            0,
            2,
            Some(now - Duration::from_millis(100)),
            false,
        );
        assert_eq!(decision, PromptTailDecision::Wait);
    }

    #[test]
    fn prompt_tail_done_after_idle_settle_with_buffered_message() {
        let started_at = Instant::now();
        let now = started_at + Duration::from_secs(1);
        let decision = prompt_tail_decision(
            started_at,
            now,
            0,
            2,
            Some(now - CLINE_PROMPT_TAIL_IDLE_SETTLE - Duration::from_millis(1)),
            true,
        );
        assert_eq!(decision, PromptTailDecision::Done);
    }

    #[test]
    fn usage_update_lines_are_dropped_before_acp_parse() {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "ses_123",
                "update": {
                    "sessionUpdate": "usage_update",
                    "used": 0,
                    "size": 1024
                }
            }
        })
        .to_string();

        let rewritten = rewrite_session_update_line(&line).expect("line should be recognized");
        assert!(rewritten.is_empty(), "usage updates should be dropped");
    }

    #[test]
    fn agent_message_lines_are_rewritten_to_agent_message_chunks() {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "ses_123",
                "update": {
                    "sessionUpdate": "agent_message",
                    "content": {
                        "type": "text",
                        "text": "hi"
                    }
                }
            }
        })
        .to_string();

        let rewritten = rewrite_session_update_line(&line).expect("line should be rewritten");
        assert_eq!(rewritten.len(), 1);
        let parsed: Value =
            serde_json::from_str(&rewritten[0]).expect("rewritten line should stay valid JSON");
        assert_eq!(
            parsed.pointer("/params/update/sessionUpdate"),
            Some(&Value::String("agent_message_chunk".to_string()))
        );
    }

    #[test]
    fn raw_session_update_kind_reports_method_and_update_kind() {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "update": {
                    "sessionUpdate": "agent_message"
                }
            }
        })
        .to_string();
        assert_eq!(
            raw_session_update_kind(&line).as_deref(),
            Some("session/update:agent_message")
        );

        let other = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/open"
        })
        .to_string();
        assert_eq!(
            raw_session_update_kind(&other).as_deref(),
            Some("session/open")
        );
    }
}
