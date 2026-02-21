use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_client_protocol::{
    Agent, AuthMethod, AuthenticateRequest, CancelNotification, Client, ClientCapabilities,
    ClientSideConnection, ContentBlock, EnvVariable, ErrorCode, ImageContent, InitializeRequest,
    McpServer, McpServerStdio, NewSessionRequest, PermissionOptionKind, PromptRequest,
    ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResourceLink, SelectedPermissionOutcome, SessionNotification, TextContent,
};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};
use tokio::task::LocalSet;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::crp::{
    CrpChannel, CrpCommand, CrpEnvelope, CrpEvent, CrpMcpServerConfig, CrpTurnStatus, CrpWriter,
};
use crate::translate::Translator;

struct SessionState {
    translator: Translator,
    cwd: PathBuf,
    active_turn_id: Option<String>,
    last_turn_update_at: Option<Instant>,
    turn_update_count: u64,
}

#[derive(Default)]
struct BridgeState {
    auth_methods: Vec<AuthMethod>,
    provider_id: Option<String>,
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
            let events = state.translator.apply_update(args.update);
            if state.active_turn_id.is_some() && !events.is_empty() {
                state.last_turn_update_at = Some(Instant::now());
                state.turn_update_count = state.turn_update_count.saturating_add(events.len() as u64);
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

fn rewrite_agent_message_line(line: &str) -> Option<Vec<String>> {
    let msg: Value = serde_json::from_str(line).ok()?;
    let method = msg.get("method").and_then(|v| v.as_str())?;
    if method != "session/update" {
        return None;
    }
    let update = msg.pointer("/params/update")?;
    let kind = update.get("sessionUpdate").and_then(|v| v.as_str())?;
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

async fn forward_acp_stdout(
    child_stdout: tokio::process::ChildStdout,
    proxy: tokio::io::DuplexStream,
) -> Result<()> {
    let mut lines = BufReader::new(child_stdout).lines();
    let mut writer = BufWriter::new(proxy);
    while let Some(line) = lines.next_line().await? {
        if let Some(rewrites) = rewrite_agent_message_line(&line) {
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

            let init = InitializeRequest::new(ProtocolVersion::LATEST)
                .client_capabilities(ClientCapabilities::default());
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
                        emit_auth_required_notice(
                            events_tx,
                            bridge_state,
                            &crp_session_id,
                            err,
                        )
                        .await;
                        return Ok(());
                    }
                    return Err(anyhow!("acp new_session failed: {err}"));
                }
            };

            let acp_session_id = response.session_id.to_string();

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
                },
            };
            let _ = events_tx.send(opened).await;
        }
        CrpCommand::SessionPrompt {
            session_id,
            turn_id,
            items,
            prompt,
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

            let prompt_blocks = build_prompt_blocks(prompt, items, Some(&prompt_cwd))
                .ok_or_else(|| anyhow!("session.prompt missing prompt"))?;
            let prompt_req = PromptRequest::new(acp_session_id.clone(), prompt_blocks);
            let response = acp.prompt(prompt_req).await.context("acp prompt")?;

            if wait_for_cline_tail {
                wait_for_prompt_tail(
                    sessions,
                    &acp_session_id,
                    &turn_id,
                    initial_update_count,
                )
                .await;
            }

            let mut sessions_guard = sessions.lock().await;
            let state = sessions_guard
                .by_acp
                .get_mut(&acp_session_id)
                .ok_or_else(|| anyhow!("missing session after prompt"))?;

            if let Some(final_event) = state.translator.finish_turn() {
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
                        emit_auth_error_notice(events_tx, bridge_state, &crp_session_id, err)
                            .await;
                    }
                }
            }
        }
        CrpCommand::ModelsList { .. } => {
            let _ = events_tx
                .send(CrpEnvelope {
                    channel: CrpChannel::Control,
                    event: CrpEvent::ModelsList {
                        models: vec![],
                        current_model_id: None,
                    },
                })
                .await;
        }
    }

    Ok(())
}

fn crp_mcp_servers_to_acp(
    servers: HashMap<String, CrpMcpServerConfig>,
) -> Vec<McpServer> {
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
) -> Option<Vec<ContentBlock>> {
    let mut blocks = Vec::new();
    if let Some(items) = items {
        for item in items {
            blocks.extend(crp_item_to_content_blocks(&item, cwd));
        }
    }
    if blocks.is_empty() {
        if let Some(prompt) = prompt {
            blocks.push(ContentBlock::Text(TextContent::new(prompt)));
        }
    }
    if blocks.is_empty() {
        None
    } else {
        Some(blocks)
    }
}

fn crp_item_to_content_blocks(item: &Value, cwd: Option<&Path>) -> Vec<ContentBlock> {
    if let Some(text) = item.as_str() {
        return vec![ContentBlock::Text(TextContent::new(text))];
    }
    let Some(obj) = item.as_object() else {
        return Vec::new();
    };

    if let Some(kind) = obj.get("type").and_then(|v| v.as_str()) {
        match kind {
            "text" => {
                if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                    return vec![ContentBlock::Text(TextContent::new(text))];
                }
            }
            "image" => {
                if let Some(block) = image_block_from_item(obj) {
                    return vec![block];
                }
            }
            "local_image" => {
                if let Some(block) = local_image_block(obj, cwd) {
                    return vec![block];
                }
            }
            "skill" => {
                if let Some(block) = skill_block(obj) {
                    return vec![block];
                }
            }
            _ => {}
        }
    }

    if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
        return vec![ContentBlock::Text(TextContent::new(text))];
    }
    if let Some(text) = obj.get("content").and_then(|v| v.as_str()) {
        return vec![ContentBlock::Text(TextContent::new(text))];
    }
    if let Some(url_value) = obj.get("image_url") {
        if let Some(url) = url_value.as_str() {
            return vec![image_block_from_url(url)];
        }
        if let Some(url) = url_value.get("url").and_then(|v| v.as_str()) {
            return vec![image_block_from_url(url)];
        }
    }

    Vec::new()
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

    #[test]
    fn cline_tail_wait_is_provider_scoped() {
        assert!(should_wait_cline_prompt_tail(Some("cline")));
        assert!(should_wait_cline_prompt_tail(Some("CLINE")));
        assert!(!should_wait_cline_prompt_tail(Some("qwen")));
        assert!(!should_wait_cline_prompt_tail(None));
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
    fn auth_notice_message_prefers_error_data_message() {
        let data = json!({ "message": "auth needed" });
        assert_eq!(
            auth_notice_message("fallback message", Some(&data)),
            "auth needed"
        );
    }

    #[test]
    fn auth_notice_details_merges_error_data_object() {
        let details = auth_notice_details(
            &[],
            Some("amp"),
            Some(&json!({ "auth_url": "https://example.test/login" })),
        );
        let details = details.expect("details");
        assert_eq!(details.get("provider").and_then(Value::as_str), Some("amp"));
        assert_eq!(
            details.get("auth_url").and_then(Value::as_str),
            Some("https://example.test/login")
        );
    }
}
