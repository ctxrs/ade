use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

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
    stdout_non_json: Vec<String>,
    stderr_lines: Vec<String>,
}

pub async fn run_acp_turn(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    input: String,
    workdir: PathBuf,
    env: HashMap<String, String>,
    event_sink: mpsc::Sender<NormalizedEvent>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<()> {
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
    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let mut next_id: u64 = 1;
    let mut pending: HashMap<u64, oneshot::Sender<serde_json::Value>> = HashMap::new();

    let mut state = StreamState::default();
    let mut acp_session_id: Option<String> = None;

    let mut make_request = |method: &str,
                            params: serde_json::Value|
     -> Result<(oneshot::Receiver<serde_json::Value>, String, u64)> {
        let id = next_id;
        next_id += 1;
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
    };

    let mut send_notification =
        |method: &str, params: serde_json::Value| -> Result<String> {
            let msg = json!({
                "jsonrpc": "2.0",
                "method": method,
                "params": params,
            });
            serde_json::to_string(&msg).context("serializing ACP notification")
        };

    // initialize
    let (init_rx, init_line, _init_id) = make_request(
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
    stdin
        .write_all(init_line.as_bytes())
        .await
        .context("writing initialize")?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;

    // session/new
    let cwd = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.clone())
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

    // Wait for initialize response before creating a session; some agents need to negotiate.
    let mut initialized = false;
    let mut init_rx = Some(init_rx);
    let mut new_rx: Option<oneshot::Receiver<serde_json::Value>> = None;
    let mut prompt_rx: Option<oneshot::Receiver<serde_json::Value>> = None;

    loop {
        tokio::select! {
            _ = &mut cancel_rx => {
                // If we already have a session, ask the agent to cancel the prompt turn.
                if let Some(session_id) = acp_session_id.clone() {
                    if let Ok(line) = send_notification("session/cancel", json!({"sessionId": session_id})) {
                        let _ = stdin.write_all(line.as_bytes()).await;
                        let _ = stdin.write_all(b"\n").await;
                        let _ = stdin.flush().await;
                    }
                }
                let _ = child.kill().await;
                let _ = event_sink.send(NormalizedEvent {
                    event_type: SessionEventType::InterruptRequested,
                    payload_json: json!({"provider": agent.provider_id}),
                }).await;
                return Ok(());
            }
            line = stdout_reader.next_line() => {
                match line {
                    Ok(Some(l)) => {
                        let parsed = match serde_json::from_str::<serde_json::Value>(&l) {
                            Ok(v) => v,
                            Err(_) => {
                                state.stdout_non_json.push(l);
                                continue;
                            }
                        };

                        // Responses to our requests
                        if let Some(id) = parsed.get("id").and_then(|v| v.as_u64()) {
                            if let Some(tx) = pending.remove(&id) {
                                let _ = tx.send(parsed);
                            }
                            continue;
                        }

                        // Agent -> Client request: session/request_permission
                        if parsed.get("method").and_then(|v| v.as_str()) == Some("session/request_permission") {
                            handle_request_permission(&agent.provider_id, &mut stdin, &parsed).await?;
                            continue;
                        }

                        // Agent -> Client notification: session/update
                        if parsed.get("method").and_then(|v| v.as_str()) == Some("session/update") {
                            let events = normalize_session_update(&parsed, &mut state);
                            for ev in events {
                                let _ = event_sink.send(ev).await;
                            }
                            continue;
                        }

                        // Unknown notification: ignore but keep for debugging.
                        let _ = event_sink.send(NormalizedEvent {
                            event_type: SessionEventType::Init,
                            payload_json: json!({"provider": agent.provider_id, "acp_event": parsed}),
                        }).await;
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let _ = event_sink.send(NormalizedEvent {
                            event_type: SessionEventType::Error,
                            payload_json: json!({"provider": agent.provider_id, "stream":"stdout", "message": e.to_string()}),
                        }).await;
                        break;
                    }
                }
            }
            line = stderr_reader.next_line() => {
                match line {
                    Ok(Some(l)) => state.stderr_lines.push(l),
                    Ok(None) => {},
                    Err(e) => state.stderr_lines.push(e.to_string()),
                }
            }
            resp = async { init_rx.as_mut().unwrap().await }, if !initialized && init_rx.is_some() => {
                let resp = resp.context("waiting for initialize response")?;
                if let Some(err) = resp.get("error") {
                    anyhow::bail!("ACP initialize error: {err}");
                }
                initialized = true;

                let (rx, line, _id) = make_request("session/new", json!({"cwd": cwd, "mcpServers": mcp_servers}))?;
                new_rx = Some(rx);
                stdin.write_all(line.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;
            }
            resp = async { new_rx.as_mut().unwrap().await }, if initialized && acp_session_id.is_none() && new_rx.is_some() => {
                let resp = resp.context("waiting for session/new response")?;
                if let Some(err) = resp.get("error") {
                    anyhow::bail!("ACP session/new error: {err}");
                }
                let session_id = resp
                    .get("result")
                    .and_then(|v| v.get("sessionId"))
                    .and_then(|v| v.as_str())
                    .context("missing sessionId in session/new response")?
                    .to_string();
                acp_session_id = Some(session_id.clone());
                let _ = event_sink.send(NormalizedEvent {
                    event_type: SessionEventType::Init,
                    payload_json: json!({"provider": agent.provider_id, "acp_session_id": session_id}),
                }).await;

                let (rx, line, _id) = make_request(
                    "session/prompt",
                    json!({"sessionId": acp_session_id.clone().unwrap(), "prompt": [{"type":"text","text": input}]}),
                )?;
                prompt_rx = Some(rx);
                stdin.write_all(line.as_bytes()).await?;
                stdin.write_all(b"\n").await?;
                stdin.flush().await?;
            }
            resp = async { prompt_rx.as_mut().unwrap().await }, if initialized && acp_session_id.is_some() && !state.saw_done && prompt_rx.is_some() => {
                let resp = resp.context("waiting for session/prompt response")?;
                state.saw_done = true;
                if let Some(err) = resp.get("error") {
                    let _ = event_sink.send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({"provider": agent.provider_id, "acp_error": err}),
                    }).await;
                }

                if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
                    state.saw_assistant_complete = true;
                    let _ = event_sink.send(NormalizedEvent {
                        event_type: SessionEventType::AssistantComplete,
                        payload_json: json!({"full_content": state.assistant_buf}),
                    }).await;
                }

                let stop_reason = resp
                    .get("result")
                    .and_then(|v| v.get("stopReason"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let _ = event_sink.send(NormalizedEvent {
                    event_type: SessionEventType::Done,
                    payload_json: json!({
                        "provider": agent.provider_id,
                        "status": if resp.get("error").is_some() { "error" } else { "success" },
                        "stop_reason": stop_reason,
                    }),
                }).await;
            }
        }

        if state.saw_done {
            // Give the agent a short grace period to flush any last session/update notifications.
            tokio::time::sleep(Duration::from_millis(200)).await;
            break;
        }
    }

    let status = child.wait().await.context("waiting for agent process exit")?;
    if !status.success() {
        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Error,
                payload_json: json!({
                    "provider": agent.provider_id,
                    "message": "agent process exited non-zero",
                    "provider_exit_code": status.code(),
                    "stderr_tail": tail_lines(&state.stderr_lines, 50),
                    "stdout_non_json_tail": tail_lines(&state.stdout_non_json, 50),
                }),
            })
            .await;
        if !state.saw_done {
            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::Done,
                    payload_json: json!({"provider": agent.provider_id, "status":"error"}),
                })
                .await;
        }
    }

    Ok(())
}

async fn handle_request_permission(
    provider_id: &str,
    stdin: &mut tokio::io::BufWriter<tokio::process::ChildStdin>,
    msg: &serde_json::Value,
) -> Result<()> {
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
    stdin.write_all(line.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;
    Ok(())
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
            if matches!(status, "completed" | "failed") {
                vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: json!({
                        "tool_call_id": tool_call_id,
                        "acp_update": update,
                    }),
                }]
            } else {
                vec![]
            }
        }
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

fn tail_lines(lines: &[String], max: usize) -> Vec<String> {
    let start = lines.len().saturating_sub(max);
    lines[start..].to_vec()
}
