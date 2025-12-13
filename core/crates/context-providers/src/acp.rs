use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use context_core::models::SessionEventType;

use crate::events::NormalizedEvent;

#[derive(Debug, Clone)]
pub struct AcpMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct AcpClientConfig {
    pub client_name: String,
    pub client_title: String,
    pub client_version: String,
    pub client_capabilities: serde_json::Value,
    pub mcp_servers: Vec<AcpMcpServer>,
}

#[derive(Debug, Clone)]
pub struct AcpAgentConfig {
    pub provider_id: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Default)]
struct StreamState {
    assistant_buf: String,
    saw_assistant_complete: bool,
    saw_done: bool,
}

pub struct AcpSessionPool {
    agent: AcpAgentConfig,
    sessions: Mutex<HashMap<String, Arc<AcpSessionHandle>>>,
    idle_ttl: Duration,
}

struct AcpSessionHandle {
    session: Mutex<AcpSession>,
    last_used: Mutex<std::time::Instant>,
}

impl AcpSessionPool {
    pub fn new(agent: AcpAgentConfig) -> Self {
        Self {
            agent,
            sessions: Mutex::new(HashMap::new()),
            idle_ttl: Duration::from_secs(30 * 60),
        }
    }

    pub fn with_idle_ttl(mut self, ttl: Duration) -> Self {
        self.idle_ttl = ttl;
        self
    }

    pub fn spawn_reaper(self: &Arc<Self>) {
        let pool = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                pool.reap_idle().await;
            }
        });
    }

    pub async fn prompt(
        &self,
        session_key: String,
        client: AcpClientConfig,
        prompt: Vec<serde_json::Value>,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
        cancel_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        let handle = self
            .get_or_create(session_key.clone(), client, workdir, env, event_sink.clone())
            .await?;

        {
            let mut last_used = handle.last_used.lock().await;
            *last_used = std::time::Instant::now();
        }

        let mut session = handle.session.lock().await;
        match session.prompt(prompt, event_sink.clone(), cancel_rx).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // If the agent process crashed, drop the session so the next turn can recreate it.
                let is_alive = session.is_alive().await.unwrap_or(false);
                drop(session);
                if !is_alive {
                    let mut map = self.sessions.lock().await;
                    map.remove(&session_key);
                }
                Err(e)
            }
        }
    }

    pub async fn set_model(&self, session_key: String, model_id: String) -> Result<()> {
        let handle = {
            let map = self.sessions.lock().await;
            map.get(&session_key)
                .cloned()
                .context("no active ACP session for this Context session")?
        };
        let (tx, _rx) = mpsc::channel::<NormalizedEvent>(1);
        let mut session = handle.session.lock().await;
        session.set_model(model_id, tx).await
    }

    pub async fn set_mode(&self, session_key: String, mode_id: String) -> Result<()> {
        let handle = {
            let map = self.sessions.lock().await;
            map.get(&session_key)
                .cloned()
                .context("no active ACP session for this Context session")?
        };
        let (tx, _rx) = mpsc::channel::<NormalizedEvent>(1);
        let mut session = handle.session.lock().await;
        session.set_mode(mode_id, tx).await
    }

    async fn get_or_create(
        &self,
        session_key: String,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<Arc<AcpSessionHandle>> {
        {
            let map = self.sessions.lock().await;
            if let Some(h) = map.get(&session_key) {
                return Ok(Arc::clone(h));
            }
        }

        let session = AcpSession::spawn(self.agent.clone(), client, workdir, env, event_sink)
            .await
            .context("creating ACP session")?;

        let handle = Arc::new(AcpSessionHandle {
            session: Mutex::new(session),
            last_used: Mutex::new(std::time::Instant::now()),
        });

        let mut map = self.sessions.lock().await;
        map.insert(session_key, Arc::clone(&handle));
        Ok(handle)
    }

    async fn reap_idle(&self) {
        let now = std::time::Instant::now();
        let keys: Vec<String> = {
            let map = self.sessions.lock().await;
            map.iter()
                .filter_map(|(k, v)| {
                    let last_used = v.last_used.try_lock().ok()?;
                    if now.duration_since(*last_used) > self.idle_ttl {
                        Some(k.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        for key in keys {
            let handle = {
                let mut map = self.sessions.lock().await;
                map.remove(&key)
            };
            if let Some(handle) = handle {
                if let Ok(mut session) = handle.session.try_lock() {
                    let _ = session.shutdown().await;
                }
            }
        }
    }
}

struct AcpSession {
    agent: AcpAgentConfig,
    workdir: PathBuf,
    child: Child,
    write_tx: mpsc::UnboundedSender<String>,
    writer: tokio::task::JoinHandle<()>,
    stdout_reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    stderr_reader: tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
    next_id: u64,
    pending: HashMap<u64, oneshot::Sender<serde_json::Value>>,
    acp_session_id: String,
    stderr_lines: Vec<String>,
    stdout_non_json: Vec<String>,
}

impl AcpSession {
    async fn spawn(
        agent: AcpAgentConfig,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<Self> {
        let mut cmd = Command::new(&agent.command);
        cmd.args(&agent.args);
        cmd.current_dir(&workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("spawning ACP agent {} ({})", agent.provider_id, agent.command))?;

        let stdin = child.stdin.take().context("capturing agent stdin")?;
        let stdout = child.stdout.take().context("capturing agent stdout")?;
        let stderr = child.stderr.take().context("capturing agent stderr")?;

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let stdout_reader = BufReader::new(stdout).lines();
        let stderr_reader = BufReader::new(stderr).lines();

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let writer = tokio::spawn(async move {
            while let Some(line) = write_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        let mut session = Self {
            agent,
            workdir: workdir.clone(),
            child,
            write_tx,
            writer,
            stdout_reader,
            stderr_reader,
            next_id: 1,
            pending: HashMap::new(),
            acp_session_id: String::new(),
            stderr_lines: Vec::new(),
            stdout_non_json: Vec::new(),
        };

        session.initialize_and_new_session(client, event_sink).await?;
        Ok(session)
    }

    async fn initialize_and_new_session(
        &mut self,
        client: AcpClientConfig,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        // initialize
        let (init_rx, init_line, _init_id) = make_request(
            &mut self.next_id,
            &mut self.pending,
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": client.client_capabilities,
                "clientInfo": {
                    "name": client.client_name,
                    "title": client.client_title,
                    "version": client.client_version,
                }
            }),
        )?;
        self.write_tx
            .send(init_line)
            .map_err(|_| anyhow::anyhow!("ACP writer task unavailable"))?;
        let init_resp = self
            .drive_until_response(init_rx, None, event_sink.clone(), None)
            .await
            .context("waiting for initialize response")?;
        if let Some(err) = init_resp.get("error") {
            anyhow::bail!("ACP initialize error: {err}");
        }

        // session/new
        let cwd = self
            .workdir
            .canonicalize()
            .unwrap_or_else(|_| self.workdir.clone())
            .to_string_lossy()
            .to_string();
        let mcp_servers = client
            .mcp_servers
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "env": s.env.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();

        let (new_rx, new_line, _new_id) = make_request(
            &mut self.next_id,
            &mut self.pending,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )?;
        self.write_tx
            .send(new_line)
            .map_err(|_| anyhow::anyhow!("ACP writer task unavailable"))?;
        let new_resp = self
            .drive_until_response(new_rx, None, event_sink.clone(), None)
            .await
            .context("waiting for session/new response")?;
        if let Some(err) = new_resp.get("error") {
            anyhow::bail!("ACP session/new error: {err}");
        }
        let session_id = new_resp
            .get("result")
            .and_then(|v| v.get("sessionId"))
            .and_then(|v| v.as_str())
            .context("missing sessionId in session/new response")?
            .to_string();
        self.acp_session_id = session_id.clone();

        // The NewSessionResponse may contain optional `modes` and `models` (Zed-style adapters).
        let modes = new_resp.get("result").and_then(|v| v.get("modes")).cloned();
        let models = new_resp.get("result").and_then(|v| v.get("models")).cloned();

        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Init,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "acp_session_id": session_id,
                    "modes": modes,
                    "models": models,
                }),
            })
            .await;

        Ok(())
    }

    async fn prompt(
        &mut self,
        prompt: Vec<serde_json::Value>,
        event_sink: mpsc::Sender<NormalizedEvent>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        let mut state = StreamState::default();

        let (prompt_rx, prompt_line, _prompt_id) = make_request(
            &mut self.next_id,
            &mut self.pending,
            "session/prompt",
            json!({"sessionId": self.acp_session_id, "prompt": prompt}),
        )?;
        self.write_tx
            .send(prompt_line)
            .map_err(|_| anyhow::anyhow!("ACP writer task unavailable"))?;

        let prompt_resp = self
            .drive_until_response(
                prompt_rx,
                Some(&mut cancel_rx),
                event_sink.clone(),
                Some(&mut state),
            )
            .await
            .context("waiting for session/prompt response")?;
        state.saw_done = true;

        if let Some(err) = prompt_resp.get("error") {
            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::Error,
                    payload_json: json!({"provider": self.agent.provider_id, "acp_error": err}),
                })
                .await;
        }

        if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
            state.saw_assistant_complete = true;
            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::AssistantComplete,
                    payload_json: json!({"full_content": state.assistant_buf}),
                })
                .await;
        }

        let stop_reason = prompt_resp
            .get("result")
            .and_then(|v| v.get("stopReason"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "acp_session_id": self.acp_session_id,
                    "status": if prompt_resp.get("error").is_some() { "error" } else { "success" },
                    "stop_reason": stop_reason,
                }),
            })
            .await;

        // Give the agent a short grace period to flush any last session/update notifications.
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }

    async fn set_model(
        &mut self,
        model_id: String,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let (rx, line, _id) = make_request(
            &mut self.next_id,
            &mut self.pending,
            "session/set_model",
            json!({"sessionId": self.acp_session_id, "modelId": model_id}),
        )?;
        self.write_tx
            .send(line)
            .map_err(|_| anyhow::anyhow!("ACP writer task unavailable"))?;
        let resp = self
            .drive_until_response(rx, None, event_sink, None)
            .await
            .context("waiting for session/set_model response")?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP set_model error: {err}");
        }
        Ok(())
    }

    async fn set_mode(
        &mut self,
        mode_id: String,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let (rx, line, _id) = make_request(
            &mut self.next_id,
            &mut self.pending,
            "session/set_mode",
            json!({"sessionId": self.acp_session_id, "modeId": mode_id}),
        )?;
        self.write_tx
            .send(line)
            .map_err(|_| anyhow::anyhow!("ACP writer task unavailable"))?;
        let resp = self
            .drive_until_response(rx, None, event_sink, None)
            .await
            .context("waiting for session/set_mode response")?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP set_mode error: {err}");
        }
        Ok(())
    }

    async fn drive_until_response(
        &mut self,
        mut target_rx: oneshot::Receiver<serde_json::Value>,
        mut cancel_rx: Option<&mut oneshot::Receiver<()>>,
        event_sink: mpsc::Sender<NormalizedEvent>,
        mut stream_state: Option<&mut StreamState>,
    ) -> Result<serde_json::Value> {
        loop {
            tokio::select! {
                _ = async { if let Some(rx) = cancel_rx.as_mut() { rx.await.ok(); } }, if cancel_rx.is_some() => {
                    let _ = self.send_cancel_notification();
                    let _ = event_sink.send(NormalizedEvent {
                        event_type: SessionEventType::InterruptRequested,
                        payload_json: json!({"provider": self.agent.provider_id}),
                    }).await;
                    return Ok(json!({"result": { "stopReason": "cancelled" }}));
                }
                line = self.stdout_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            let parsed = match serde_json::from_str::<serde_json::Value>(&l) {
                                Ok(v) => v,
                                Err(_) => {
                                    self.stdout_non_json.push(l);
                                    continue;
                                }
                            };

                            // Responses to our requests
                            if let Some(id) = parsed.get("id").and_then(|v| v.as_u64()) {
                                if let Some(tx) = self.pending.remove(&id) {
                                    let _ = tx.send(parsed);
                                }
                                continue;
                            }

                            // Agent -> Client request: session/request_permission
                            if parsed.get("method").and_then(|v| v.as_str()) == Some("session/request_permission") {
                                if let Some(line) = build_request_permission_response(&self.agent.provider_id, &parsed)? {
                                    let _ = self.write_tx.send(line);
                                }
                                continue;
                            }

                            // Agent -> Client notification: session/update
                            if parsed.get("method").and_then(|v| v.as_str()) == Some("session/update") {
                                if let Some(state) = stream_state.as_deref_mut() {
                                    let events = normalize_session_update(&parsed, state);
                                    for ev in events {
                                        let _ = event_sink.send(ev).await;
                                    }
                                } else {
                                    // No per-turn state (initialize/new_session); keep raw updates for debugging.
                                    let _ = event_sink.send(NormalizedEvent {
                                        event_type: SessionEventType::Init,
                                        payload_json: json!({"provider": self.agent.provider_id, "acp_event": parsed}),
                                    }).await;
                                }
                                continue;
                            }

                            // Unknown notification: keep for debugging.
                            let _ = event_sink.send(NormalizedEvent {
                                event_type: SessionEventType::Init,
                                payload_json: json!({"provider": self.agent.provider_id, "acp_event": parsed}),
                            }).await;
                        }
                        Ok(None) => {
                            anyhow::bail!("agent stdout closed");
                        }
                        Err(e) => {
                            let _ = event_sink.send(NormalizedEvent {
                                event_type: SessionEventType::Error,
                                payload_json: json!({"provider": self.agent.provider_id, "stream":"stdout", "message": e.to_string()}),
                            }).await;
                            anyhow::bail!("agent stdout read error: {e}");
                        }
                    }
                }
                line = self.stderr_reader.next_line() => {
                    match line {
                        Ok(Some(l)) => self.stderr_lines.push(l),
                        Ok(None) => {},
                        Err(e) => self.stderr_lines.push(e.to_string()),
                    }
                    // Cap stderr accumulation.
                    if self.stderr_lines.len() > 500 {
                        let start = self.stderr_lines.len().saturating_sub(250);
                        self.stderr_lines = self.stderr_lines[start..].to_vec();
                    }
                }
                resp = &mut target_rx => {
                    let resp = resp.context("awaiting ACP response")?;
                    return Ok(resp);
                }
            }
        }
    }

    fn send_cancel_notification(&self) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": self.acp_session_id }
        });
        let line = serde_json::to_string(&msg).context("serializing ACP cancel notification")?;
        let _ = self.write_tx.send(line);
        Ok(())
    }

    async fn is_alive(&mut self) -> Result<bool> {
        Ok(self.child.try_wait()?.is_none())
    }

    async fn shutdown(&mut self) -> Result<()> {
        let _ = self.child.kill().await;
        self.writer.abort();
        Ok(())
    }
}

fn build_request_permission_response(
    provider_id: &str,
    msg: &serde_json::Value,
) -> Result<Option<String>> {
    let id = msg
        .get("id")
        .and_then(|v| v.as_u64())
        .context("request_permission missing id")?;
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let options = params
        .get("options")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let selected = options
        .iter()
        .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some("allow_once"))
        .or_else(|| {
            options
                .iter()
                .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some("allow_always"))
        })
        .or_else(|| options.first());

    let option_id = selected
        .and_then(|o| o.get("optionId").and_then(|v| v.as_str()))
        .unwrap_or("allow_once");

    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "_meta": {
                "context": {
                    "autoApproved": true,
                    "provider": provider_id,
                }
            },
            "outcome": {
                "outcome": "selected",
                "optionId": option_id
            }
        }
    });
    let line = serde_json::to_string(&resp)?;
    Ok(Some(line))
}

fn normalize_session_update(msg: &serde_json::Value, state: &mut StreamState) -> Vec<NormalizedEvent> {
    let Some(params) = msg.get("params") else {
        return vec![];
    };
    let update = params.get("update").cloned().unwrap_or(json!({}));
    let kind = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    match kind {
        "agent_message_chunk" => {
            let mut out = Vec::new();
            if let Some(content) = update.get("content") {
                if let Some(text) = content_text(content) {
                    state.assistant_buf.push_str(&text);
                    out.push(NormalizedEvent {
                        event_type: SessionEventType::AssistantChunk,
                        payload_json: json!({
                            "content_fragment": text,
                            "acp_update": update,
                        }),
                    });
                }
            }
            out
        }
        "agent_thought_chunk" => {
            if let Some(content) = update.get("content") {
                if let Some(text) = content_text(content) {
                    return vec![NormalizedEvent {
                        event_type: SessionEventType::ThoughtChunk,
                        payload_json: json!({
                            "content_fragment": text,
                            "acp_update": update,
                        }),
                    }];
                }
            }
            vec![]
        }
        "agent_message" => {
            let mut out = Vec::new();
            if let Some(chunks) = update.get("content").and_then(|v| v.as_array()) {
                for block in chunks {
                    if let Some(text) = content_text(block) {
                        state.assistant_buf.push_str(&text);
                        out.push(NormalizedEvent {
                            event_type: SessionEventType::AssistantChunk,
                            payload_json: json!({
                                "content_fragment": text,
                                "acp_update": update,
                            }),
                        });
                    }
                }
            }
            out
        }
        "tool_call" => {
            let tool_call_id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolCall,
                payload_json: json!({
                    "tool_call_id": tool_call_id,
                    "acp_update": update,
                }),
            }]
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let mut out = vec![NormalizedEvent {
                event_type: SessionEventType::ToolCallUpdate,
                payload_json: json!({
                    "tool_call_id": tool_call_id,
                    "acp_update": update,
                }),
            }];
            if matches!(status, "completed" | "failed") {
                out.push(NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: json!({
                        "tool_call_id": tool_call_id,
                        "acp_update": update,
                    }),
                });
            }
            out
        }
        "plan" => vec![NormalizedEvent {
            event_type: SessionEventType::Plan,
            payload_json: json!({"acp_update": update}),
        }],
        "error" => vec![NormalizedEvent {
            event_type: SessionEventType::Error,
            payload_json: json!({"acp_update": update}),
        }],
        _ => vec![],
    }
}

fn content_text(block: &serde_json::Value) -> Option<String> {
    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
        return block.get("text").and_then(|v| v.as_str()).map(|s| s.to_string());
    }
    None
}

fn make_request(
    next_id: &mut u64,
    pending: &mut HashMap<u64, oneshot::Sender<serde_json::Value>>,
    method: &str,
    params: serde_json::Value,
) -> Result<(oneshot::Receiver<serde_json::Value>, String, u64)> {
    let id = *next_id;
    *next_id += 1;
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let line = serde_json::to_string(&msg).context("serializing ACP request")?;
    let (tx, rx) = oneshot::channel();
    pending.insert(id, tx);
    Ok((rx, line, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn normalizes_agent_message_chunk_and_buffers_text() {
        let mut state = StreamState::default();
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": { "type": "text", "text": "hello" }
                }
            }
        });

        let events = normalize_session_update(&msg, &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, SessionEventType::AssistantChunk));
        assert_eq!(state.assistant_buf, "hello");
    }

    #[test]
    fn normalizes_tool_call_and_terminal_tool_result() {
        let mut state = StreamState::default();
        let tool_call = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "tool_call",
                    "toolCallId": "call_1",
                    "title": "Do thing",
                    "kind": "other",
                    "status": "pending"
                }
            }
        });
        let tool_done = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "call_1",
                    "status": "completed"
                }
            }
        });

        let ev1 = normalize_session_update(&tool_call, &mut state);
        assert_eq!(ev1.len(), 1);
        assert!(matches!(ev1[0].event_type, SessionEventType::ToolCall));

        let ev2 = normalize_session_update(&tool_done, &mut state);
        assert_eq!(ev2.len(), 2);
        assert!(matches!(ev2[0].event_type, SessionEventType::ToolCallUpdate));
        assert!(matches!(ev2[1].event_type, SessionEventType::ToolResult));
    }

    #[test]
    fn auto_approves_allow_once_permission() {
        let req = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "session/request_permission",
            "params": {
                "sessionId": "sess_1",
                "toolCall": { "toolCallId": "call_1", "status": "pending", "title": "edit", "kind": "edit" },
                "options": [
                    { "optionId": "reject", "name": "No", "kind": "reject_once" },
                    { "optionId": "allow", "name": "Yes", "kind": "allow_once" }
                ]
            }
        });

        let resp = build_request_permission_response("codex", &req)
            .unwrap()
            .unwrap();
        let resp: Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(resp.get("id").and_then(|v| v.as_i64()), Some(7));
        let option_id = resp
            .get("result")
            .and_then(|v| v.get("outcome"))
            .and_then(|v| v.get("optionId"))
            .and_then(|v| v.as_str());
        assert_eq!(option_id, Some("allow"));
    }
}
