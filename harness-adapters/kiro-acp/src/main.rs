use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;

use kiro_acp::{format_prompt_blocks, strip_ansi};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(long, env = "KIRO_BIN", default_value = "kiro-cli")]
    kiro_bin: String,
    #[arg(long, default_value = "context-acp")]
    agent_prefix: String,
}

struct ServerState {
    kiro_bin: String,
    agent_prefix: String,
    sessions: HashMap<String, Arc<Mutex<SessionState>>>,
}

struct SessionState {
    cwd: PathBuf,
    agent_name: String,
    agent_config_path: PathBuf,
    mcp_servers: BTreeMap<String, Value>,
    model: Option<String>,
    mode_id: Option<String>,
    has_started: bool,
    active_prompt: Option<ActivePrompt>,
}

struct ActivePrompt {
    cancel_tx: oneshot::Sender<()>,
}

#[derive(Serialize)]
struct AgentConfig {
    name: String,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(rename = "mcpServers", skip_serializing_if = "BTreeMap::is_empty")]
    mcp_servers: BTreeMap<String, Value>,
    #[serde(rename = "includeMcpJson")]
    include_mcp_json: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let state = Arc::new(Mutex::new(ServerState {
        kiro_bin: args.kiro_bin,
        agent_prefix: args.agent_prefix,
        sessions: HashMap::new(),
    }));

    let (writer_tx, mut writer_rx) = mpsc::channel::<Value>(128);

    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = writer_rx.recv().await {
            match serde_json::to_string(&message) {
                Ok(line) => {
                    if stdout.write_all(line.as_bytes()).await.is_ok() {
                        let _ = stdout.write_all(b"\n").await;
                    }
                    let _ = stdout.flush().await;
                }
                Err(err) => {
                    eprintln!("failed to serialize ACP message: {err}");
                }
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

        let message: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("invalid ACP input: {err} ({trimmed})");
                continue;
            }
        };

        let writer_tx = writer_tx.clone();
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            if let Err(err) = handle_message(message, state, writer_tx).await {
                eprintln!("ACP handler error: {err}");
            }
        });
    }

    drop(writer_tx);
    let _ = writer.await;
    Ok(())
}

async fn handle_message(
    message: Value,
    state: Arc<Mutex<ServerState>>,
    writer_tx: mpsc::Sender<Value>,
) -> Result<()> {
    let method = match message.get("method").and_then(|v| v.as_str()) {
        Some(method) => method,
        None => return Ok(()),
    };

    let id = message.get("id").cloned();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": false,
                    "promptCapabilities": {
                        "image": false,
                        "audio": false,
                        "embeddedContext": true
                    },
                    "mcpCapabilities": {
                        "http": false,
                        "sse": false
                    }
                },
                "agentInfo": {
                    "name": "kiro-acp",
                    "title": "Kiro ACP Adapter",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "authMethods": []
            });
            send_response(writer_tx, id, result).await?;
        }
        "authenticate" => {
            send_error(writer_tx, id, -32601, "authenticate not supported").await?;
        }
        "session/new" => match handle_session_new(state, &params).await {
            Ok((_session_id, response)) => {
                send_response(writer_tx, id, response).await?;
            }
            Err(err) => {
                send_error(writer_tx, id, -32000, &err.to_string()).await?;
            }
        },
        "session/load" => {
            send_error(writer_tx, id, -32601, "session/load not supported").await?;
        }
        "session/prompt" => {
            let request_id = id;
            tokio::spawn(async move {
                if let Err(err) = handle_session_prompt(params, state, writer_tx, request_id).await {
                    eprintln!("prompt error: {err}");
                }
            });
        }
        "session/cancel" => {
            handle_session_cancel(state, &params).await?;
        }
        "session/set_mode" => match handle_session_set_mode(state, &params).await {
            Ok(response) => {
                send_response(writer_tx, id, response).await?;
            }
            Err(err) => {
                send_error(writer_tx, id, -32000, &err.to_string()).await?;
            }
        },
        "session/set_model" => match handle_session_set_model(state, &params).await {
            Ok(response) => {
                send_response(writer_tx, id, response).await?;
            }
            Err(err) => {
                send_error(writer_tx, id, -32000, &err.to_string()).await?;
            }
        },
        _ => {
            if id.is_some() {
                send_error(writer_tx, id, -32601, "method not supported").await?;
            }
        }
    }

    Ok(())
}

async fn handle_session_new(state: Arc<Mutex<ServerState>>, params: &Value) -> Result<(String, Value)> {
    let cwd = params
        .get("cwd")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/new missing cwd"))?;
    let cwd_path = PathBuf::from(cwd);
    if !cwd_path.is_absolute() {
        return Err(anyhow!("cwd must be absolute"));
    }

    let mcp_servers = params
        .get("mcpServers")
        .and_then(|v| v.as_array())
        .map(|servers| parse_mcp_servers(servers))
        .unwrap_or_default();

    let mut state_guard = state.lock().await;
    let session_id = format!("kiro-{}", Uuid::new_v4());
    let agent_name = format!("{}-{}", state_guard.agent_prefix, session_id);
    let agent_config_path = cwd_path
        .join(".kiro")
        .join("agents")
        .join(format!("{}.json", agent_name));

    let session_state = SessionState {
        cwd: cwd_path.clone(),
        agent_name,
        agent_config_path: agent_config_path.clone(),
        mcp_servers,
        model: None,
        mode_id: None,
        has_started: false,
        active_prompt: None,
    };

    let session = Arc::new(Mutex::new(session_state));
    write_agent_config(&session).await?;
    state_guard.sessions.insert(session_id.clone(), session);

    let response = json!({
        "sessionId": session_id
    });

    Ok((session_id, response))
}

async fn handle_session_prompt(
    params: Value,
    state: Arc<Mutex<ServerState>>,
    writer_tx: mpsc::Sender<Value>,
    request_id: Option<Value>,
) -> Result<()> {
    let response_id = request_id.clone();
    if let Err(err) = handle_session_prompt_inner(params, state, writer_tx.clone(), request_id).await
    {
        if let Some(id) = response_id {
            let _ = send_error(writer_tx, Some(id), -32000, &err.to_string()).await;
        }
    }
    Ok(())
}

async fn handle_session_prompt_inner(
    params: Value,
    state: Arc<Mutex<ServerState>>,
    writer_tx: mpsc::Sender<Value>,
    request_id: Option<Value>,
) -> Result<()> {
    let request_id = match request_id {
        Some(id) => id,
        None => return Ok(()),
    };

    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/prompt missing sessionId"))?
        .to_string();

    let prompt_blocks = params
        .get("prompt")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let session = {
        let state_guard = state.lock().await;
        state_guard
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown session {session_id}"))?
    };

    let mut session_guard = session.lock().await;
    if session_guard.active_prompt.is_some() {
        send_error(writer_tx, Some(request_id), -32000, "prompt already running").await?;
        return Ok(());
    }

    let prompt_text = format_prompt_blocks(&prompt_blocks);
    if prompt_text.trim().is_empty() {
        send_response(writer_tx, Some(request_id), json!({"stopReason": "end_turn"})).await?;
        return Ok(());
    }

    let kiro_bin = {
        let state_guard = state.lock().await;
        state_guard.kiro_bin.clone()
    };

    write_agent_config(&session).await?;

    let mut command = Command::new(&kiro_bin);
    command.arg("chat");
    command.arg("--no-interactive");
    command.arg("--trust-all-tools");
    command.arg("--agent");
    command.arg(&session_guard.agent_name);
    if session_guard.has_started {
        command.arg("--resume");
    }
    command.arg("--");
    command.arg(prompt_text);
    command.current_dir(&session_guard.cwd);
    command.env("NO_COLOR", "1");

    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn {kiro_bin}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("failed to capture kiro stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("failed to capture kiro stderr"))?;

    let (cancel_tx, cancel_rx) = oneshot::channel();
    session_guard.active_prompt = Some(ActivePrompt { cancel_tx });
    drop(session_guard);

    let stdout_task = spawn_stdout_reader(session_id.clone(), stdout, writer_tx.clone());
    let stderr_task = spawn_stderr_reader(stderr);

    let mut cancelled = false;
    let exit_status = tokio::select! {
        _ = cancel_rx => {
            cancelled = true;
            let _ = child.kill().await;
            child.wait().await.ok()
        }
        status = child.wait() => {
            status.ok()
        }
    };

    let stdout_result = stdout_task.await.unwrap_or_default();
    let stderr_lines = stderr_task.await.unwrap_or_default();

    let mut session_guard = session.lock().await;
    session_guard.active_prompt = None;
    session_guard.has_started = true;
    drop(session_guard);

    if cancelled {
        send_response(writer_tx, Some(request_id), json!({"stopReason": "cancelled"})).await?;
        return Ok(());
    }

    if let Some(status) = exit_status {
        if !status.success() {
            let code = status.code().map(|c| c.to_string()).unwrap_or_else(|| "signal".to_string());
            let content = json!({
                "type": "text",
                "text": format!("[kiro-cli exit status: {code}]")
            });
            send_update(
                writer_tx.clone(),
                &session_id,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": content
                }),
            )
            .await?;
        }
    }

    if !stderr_lines.is_empty() {
        let snippet = stderr_lines.join("\n");
        let content = json!({
            "type": "text",
            "text": format!("[kiro-cli stderr]\n{snippet}")
        });
        send_update(
            writer_tx.clone(),
            &session_id,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": content
            }),
        )
        .await?;
    }

    if stdout_result.is_empty() && stderr_lines.is_empty() {
        let content = json!({
            "type": "text",
            "text": "[kiro-cli produced no output]"
        });
        send_update(
            writer_tx.clone(),
            &session_id,
            json!({
                "sessionUpdate": "agent_message_chunk",
                "content": content
            }),
        )
        .await?;
    }

    send_response(writer_tx, Some(request_id), json!({"stopReason": "end_turn"})).await?;
    Ok(())
}

async fn handle_session_cancel(state: Arc<Mutex<ServerState>>, params: &Value) -> Result<()> {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/cancel missing sessionId"))?;

    let session = {
        let state_guard = state.lock().await;
        state_guard.sessions.get(session_id).cloned()
    };

    let Some(session) = session else {
        return Ok(());
    };

    let mut session_guard = session.lock().await;
    if let Some(active) = session_guard.active_prompt.take() {
        let _ = active.cancel_tx.send(());
    }
    Ok(())
}

async fn handle_session_set_mode(state: Arc<Mutex<ServerState>>, params: &Value) -> Result<Value> {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/set_mode missing sessionId"))?;
    let mode_id = params
        .get("modeId")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let session = {
        let state_guard = state.lock().await;
        state_guard.sessions.get(session_id).cloned()
    };

    if let Some(session) = session {
        let mut session_guard = session.lock().await;
        session_guard.mode_id = Some(mode_id);
    }

    Ok(json!(null))
}

async fn handle_session_set_model(state: Arc<Mutex<ServerState>>, params: &Value) -> Result<Value> {
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/set_model missing sessionId"))?;
    let model_id = params
        .get("modelId")
        .or_else(|| params.get("model"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/set_model missing modelId"))?
        .to_string();

    let session = {
        let state_guard = state.lock().await;
        state_guard.sessions.get(session_id).cloned()
    };

    if let Some(session) = session {
        let mut session_guard = session.lock().await;
        session_guard.model = Some(model_id);
        drop(session_guard);
        write_agent_config(&session).await?;
    }

    Ok(json!(null))
}

fn parse_mcp_servers(servers: &[Value]) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for server in servers {
        let Some(name) = server.get("name").and_then(|v| v.as_str()) else {
            continue;
        };

        let server_type = server.get("type").and_then(|v| v.as_str());
        let config = match server_type {
            Some("http") | Some("sse") => {
                let mut cfg = serde_json::Map::new();
                if let Some(kind) = server_type {
                    cfg.insert("type".to_string(), Value::String(kind.to_string()));
                }
                if let Some(url) = server.get("url").and_then(|v| v.as_str()) {
                    cfg.insert("url".to_string(), Value::String(url.to_string()));
                }
                if let Some(headers) = server.get("headers").and_then(|v| v.as_array()) {
                    let mut header_map = serde_json::Map::new();
                    for header in headers {
                        if let (Some(name), Some(value)) = (
                            header.get("name").and_then(|v| v.as_str()),
                            header.get("value").and_then(|v| v.as_str()),
                        ) {
                            header_map.insert(name.to_string(), Value::String(value.to_string()));
                        }
                    }
                    if !header_map.is_empty() {
                        cfg.insert("headers".to_string(), Value::Object(header_map));
                    }
                }
                Value::Object(cfg)
            }
            _ => {
                let mut cfg = serde_json::Map::new();
                if let Some(command) = server.get("command").and_then(|v| v.as_str()) {
                    cfg.insert("command".to_string(), Value::String(command.to_string()));
                }
                if let Some(args) = server.get("args").and_then(|v| v.as_array()) {
                    cfg.insert("args".to_string(), Value::Array(args.clone()));
                } else {
                    cfg.insert("args".to_string(), Value::Array(Vec::new()));
                }
                if let Some(env) = server.get("env").and_then(|v| v.as_array()) {
                    let mut env_map = serde_json::Map::new();
                    for entry in env {
                        if let (Some(name), Some(value)) = (
                            entry.get("name").and_then(|v| v.as_str()),
                            entry.get("value").and_then(|v| v.as_str()),
                        ) {
                            env_map.insert(name.to_string(), Value::String(value.to_string()));
                        }
                    }
                    if !env_map.is_empty() {
                        cfg.insert("env".to_string(), Value::Object(env_map));
                    }
                }
                Value::Object(cfg)
            }
        };

        out.insert(name.to_string(), config);
    }
    out
}

async fn write_agent_config(session: &Arc<Mutex<SessionState>>) -> Result<()> {
    let session_guard = session.lock().await;
    let config = AgentConfig {
        name: session_guard.agent_name.clone(),
        description: "Context ACP session".to_string(),
        model: session_guard.model.clone(),
        mcp_servers: session_guard.mcp_servers.clone(),
        include_mcp_json: false,
    };
    let config_json = serde_json::to_string_pretty(&config)?;
    if let Some(parent) = session_guard.agent_config_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&session_guard.agent_config_path, config_json).await?;
    Ok(())
}

fn spawn_stdout_reader(
    session_id: String,
    stdout: impl tokio::io::AsyncRead + Unpin + Send + 'static,
    writer_tx: mpsc::Sender<Value>,
) -> tokio::task::JoinHandle<Vec<String>> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut collected = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let stripped = strip_ansi(&line).trim_end_matches('\r').to_string();
            collected.push(stripped.clone());
            let content = json!({
                "type": "text",
                "text": format!("{}\n", stripped)
            });
            if send_update(
                writer_tx.clone(),
                &session_id,
                json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": content
                }),
            )
            .await
            .is_err()
            {
                break;
            }
        }
        collected
    })
}

fn spawn_stderr_reader(
    stderr: impl tokio::io::AsyncRead + Unpin + Send + 'static,
) -> tokio::task::JoinHandle<Vec<String>> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut collected = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            let stripped = strip_ansi(&line).trim_end_matches('\r').to_string();
            if stripped.is_empty() {
                continue;
            }
            eprintln!("kiro-cli: {stripped}");
            collected.push(stripped);
        }
        collected
    })
}

async fn send_update(writer_tx: mpsc::Sender<Value>, session_id: &str, update: Value) -> Result<()> {
    let message = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": session_id,
            "update": update
        }
    });
    writer_tx
        .send(message)
        .await
        .map_err(|_| anyhow!("failed to send update"))
}

async fn send_response(writer_tx: mpsc::Sender<Value>, id: Option<Value>, result: Value) -> Result<()> {
    if let Some(id) = id {
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        });
        writer_tx
            .send(message)
            .await
            .map_err(|_| anyhow!("failed to send response"))?;
    }
    Ok(())
}

async fn send_error(
    writer_tx: mpsc::Sender<Value>,
    id: Option<Value>,
    code: i64,
    message: &str,
) -> Result<()> {
    if let Some(id) = id {
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message
            }
        });
        writer_tx
            .send(message)
            .await
            .map_err(|_| anyhow!("failed to send error"))?;
    }
    Ok(())
}

#[allow(dead_code)]
fn ensure_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(anyhow!("path must be absolute"));
    }
    Ok(())
}
