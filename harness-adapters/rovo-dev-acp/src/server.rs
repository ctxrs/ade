use crate::config::Config;
use crate::prompt::format_prompt_blocks;
use crate::rpc::{
    notification, response_error, response_ok, RpcEnvelope, RpcError, RpcWriter, ERR_AUTH_REQUIRED,
    ERR_INTERNAL_ERROR, ERR_INVALID_PARAMS, ERR_INVALID_REQUEST, ERR_METHOD_NOT_FOUND,
    ERR_PARSE_ERROR, JSONRPC_VERSION,
};
use crate::rovo::{RovoApiError, RovoClient, RovoProcess};
use crate::sse::SseDecoder;
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{oneshot, Mutex};
use tokio::time::{sleep, Duration, Instant};
use uuid::Uuid;

pub async fn run(config: Config) -> Result<()> {
    let writer = RpcWriter::new();
    let state = Arc::new(Mutex::new(ServerState::default()));
    let rovo = Arc::new(Mutex::new(RovoManager::new(config.clone())));
    let prompt_gate = Arc::new(Mutex::new(()));

    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let envelope: RpcEnvelope = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                let error = RpcError {
                    code: ERR_PARSE_ERROR,
                    message: "Parse error".to_string(),
                    data: None,
                };
                let resp = response_error(Value::Null, error);
                let _ = writer.send(&resp).await;
                continue;
            }
        };

        if envelope.jsonrpc.as_deref() != Some(JSONRPC_VERSION) {
            if let Some(id) = envelope.id {
                let error = RpcError {
                    code: ERR_INVALID_REQUEST,
                    message: "Invalid request".to_string(),
                    data: None,
                };
                let resp = response_error(id, error);
                let _ = writer.send(&resp).await;
            }
            continue;
        }

        let method = match envelope.method {
            Some(method) => method,
            None => {
                continue;
            }
        };

        if envelope.id.is_none() {
            let state = Arc::clone(&state);
            let rovo = Arc::clone(&rovo);
            tokio::spawn(async move {
                if let Err(err) = handle_notification(&method, envelope.params, state, rovo).await {
                    eprintln!("[rovo-dev-acp] notification error: {err:#}");
                }
            });
            continue;
        }

        let writer = writer.clone_handle();
        let state = Arc::clone(&state);
        let rovo = Arc::clone(&rovo);
        let prompt_gate = Arc::clone(&prompt_gate);
        tokio::spawn(async move {
            let id = envelope.id.unwrap_or(Value::Null);
            let result = handle_request(
                &method,
                envelope.params,
                Arc::clone(&state),
                Arc::clone(&rovo),
                Arc::clone(&prompt_gate),
                writer.clone_handle(),
            )
            .await;

            match result {
                Ok(value) => {
                    let resp = response_ok(id, value);
                    if let Err(err) = writer.send(&resp).await {
                        eprintln!("[rovo-dev-acp] failed to send response: {err:#}");
                    }
                }
                Err(err) => {
                    let error = map_error(&err);
                    let resp = response_error(id, error);
                    if let Err(err) = writer.send(&resp).await {
                        eprintln!("[rovo-dev-acp] failed to send error: {err:#}");
                    }
                }
            }
        });
    }

    Ok(())
}

async fn handle_notification(
    method: &str,
    params: Option<Value>,
    state: Arc<Mutex<ServerState>>,
    rovo: Arc<Mutex<RovoManager>>,
) -> Result<()> {
    match method {
        "session/cancel" => {
            handle_cancel(state, rovo).await;
            Ok(())
        }
        _ => {
            if params.is_some() {
                eprintln!("[rovo-dev-acp] ignoring notification {method}");
            }
            Ok(())
        }
    }
}

async fn handle_request(
    method: &str,
    params: Option<Value>,
    state: Arc<Mutex<ServerState>>,
    rovo: Arc<Mutex<RovoManager>>,
    prompt_gate: Arc<Mutex<()>>,
    writer: RpcWriter,
) -> Result<Value> {
    match method {
        "initialize" => handle_initialize(state).await,
        "session/new" => handle_session_new(params, state, rovo).await,
        "session/prompt" => {
            handle_session_prompt(params, state, rovo, prompt_gate, writer).await
        }
        "session/cancel" => {
            handle_cancel(state, rovo).await;
            Ok(Value::Object(Map::new()))
        }
        _ => Err(rpc_method_not_found()),
    }
}

async fn handle_initialize(state: Arc<Mutex<ServerState>>) -> Result<Value> {
    let mut guard = state.lock().await;
    guard.initialized = true;
    Ok(serde_json::json!({
        "protocolVersion": 1,
        "agentCapabilities": {
            "loadSession": false,
            "promptCapabilities": {
                "image": false,
                "audio": false,
                "embeddedContext": false,
            },
            "mcpCapabilities": {
                "http": false,
                "sse": false,
            }
        },
        "agentInfo": {
            "name": "rovo-dev-acp",
            "title": "Rovo Dev ACP Adapter",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "authMethods": []
    }))
}

async fn handle_session_new(
    params: Option<Value>,
    state: Arc<Mutex<ServerState>>,
    rovo: Arc<Mutex<RovoManager>>,
) -> Result<Value> {
    ensure_initialized(&state).await?;
    let params = params.ok_or_else(|| rpc_invalid_params("missing params"))?;
    let cwd = params
        .get("cwd")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_invalid_params("`cwd` is required"))?;
    let cwd_path = PathBuf::from(cwd);
    if !cwd_path.is_absolute() {
        return Err(rpc_invalid_params("`cwd` must be an absolute path"));
    }

    {
        let mut guard = state.lock().await;
        if let Some(workdir) = guard.workdir.as_ref() {
            if workdir != &cwd_path {
                return Err(rpc_invalid_params(format!(
                    "Rovo Dev server already bound to {}, cannot use {}",
                    workdir.display(),
                    cwd_path.display()
                )));
            }
        } else {
            guard.workdir = Some(cwd_path.clone());
        }
    }

    {
        let mut rovo_guard = rovo.lock().await;
        rovo_guard.ensure_running(&cwd_path).await?;
    }

    let rovo_session_id = {
        let rovo_guard = rovo.lock().await;
        rovo_guard.client.create_session().await?
    };

    let session_id = Uuid::new_v4().to_string();
    let mut guard = state.lock().await;
    guard.sessions.insert(
        session_id.clone(),
        SessionState {
            rovo_session_id,
            cwd: cwd_path,
            seen_tool_calls: HashSet::new(),
        },
    );

    Ok(serde_json::json!({ "sessionId": session_id }))
}

async fn handle_session_prompt(
    params: Option<Value>,
    state: Arc<Mutex<ServerState>>,
    rovo: Arc<Mutex<RovoManager>>,
    prompt_gate: Arc<Mutex<()>>,
    writer: RpcWriter,
) -> Result<Value> {
    ensure_initialized(&state).await?;
    let params = params.ok_or_else(|| rpc_invalid_params("missing params"))?;
    let session_id = params
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| rpc_invalid_params("`sessionId` is required"))?
        .to_string();
    let prompt_blocks = params
        .get("prompt")
        .and_then(|v| v.as_array())
        .ok_or_else(|| rpc_invalid_params("`prompt` must be an array"))?;

    let message = format_prompt_blocks(prompt_blocks);
    if message.trim().is_empty() {
        return Err(rpc_invalid_params("prompt is empty"));
    }

    let session = {
        let guard = state.lock().await;
        guard
            .sessions
            .get(&session_id)
            .cloned()
            .ok_or_else(|| rpc_invalid_params("unknown session"))?
    };

    let _gate = prompt_gate.lock().await;
    let enable_deep_plan = {
        let rovo_guard = rovo.lock().await;
        rovo_guard.config.enable_deep_plan
    };

    {
        let mut rovo_guard = rovo.lock().await;
        rovo_guard.ensure_running(&session.cwd).await?;
        rovo_guard
            .client
            .restore_session(&session.rovo_session_id)
            .await?;
        rovo_guard
            .client
            .set_chat_message(&message, enable_deep_plan)
            .await?;
    }

    let response = open_stream_with_backoff(&rovo).await?;

    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    {
        let mut guard = state.lock().await;
        guard.active_prompt = Some(ActivePrompt { cancel_tx });
    }

    let mut decoder = SseDecoder::new();
    let mut stream = response.bytes_stream();
    let mut stop_reason = "end_turn".to_string();

    let stream_result = 'stream: loop {
        tokio::select! {
            biased;
            _ = &mut cancel_rx => {
                stop_reason = "cancelled".to_string();
                let _ = rovo.lock().await.client.cancel().await;
                break 'stream Ok(());
            }
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        let events = decoder.push_bytes(&bytes);
                        for event in events {
                            if let Err(err) = handle_sse_event(&session_id, event.data, &state, &writer).await {
                                break 'stream Err(err);
                            }
                        }
                    }
                    Some(Err(err)) => {
                        break 'stream Err(anyhow!("stream error: {err}"));
                    }
                    None => {
                        if let Some(event) = decoder.flush() {
                            if let Err(err) = handle_sse_event(&session_id, event.data, &state, &writer).await {
                                break 'stream Err(err);
                            }
                        }
                        break 'stream Ok(());
                    }
                }
            }
        }
    };

    {
        let mut guard = state.lock().await;
        guard.active_prompt = None;
    }

    if let Err(err) = stream_result {
        return Err(err);
    }

    Ok(serde_json::json!({ "stopReason": stop_reason }))
}

async fn handle_cancel(state: Arc<Mutex<ServerState>>, rovo: Arc<Mutex<RovoManager>>) {
    let cancel_tx = {
        let mut guard = state.lock().await;
        guard.active_prompt.take().map(|prompt| prompt.cancel_tx)
    };

    if let Some(cancel_tx) = cancel_tx {
        let _ = cancel_tx.send(());
    }

    let _ = rovo.lock().await.client.cancel().await;
}

async fn handle_sse_event(
    session_id: &str,
    data: String,
    state: &Arc<Mutex<ServerState>>,
    writer: &RpcWriter,
) -> Result<()> {
    let parsed = serde_json::from_str::<Value>(&data).unwrap_or(Value::String(data.clone()));
    if is_done_event(&parsed) {
        return Ok(());
    }
    let mut updates = Vec::new();

    if let Some(text) = extract_text(&parsed) {
        let mut update = Map::new();
        update.insert("sessionUpdate".to_string(), Value::String("agent_message_chunk".to_string()));
        update.insert(
            "content".to_string(),
            serde_json::json!({"type": "text", "text": text}),
        );
        update.insert(
            "_meta".to_string(),
            serde_json::json!({"rovoEvent": parsed.clone()}),
        );
        updates.push(Value::Object(update));
    }

    if let Some(tool_updates) = extract_tool_updates(&parsed, state, session_id).await {
        updates.extend(tool_updates);
    }

    for update in updates {
        let params = serde_json::json!({
            "sessionId": session_id,
            "update": update,
        });
        let msg = notification("session/update", params);
        writer.send(&msg).await?;
    }

    Ok(())
}

async fn extract_tool_updates(
    parsed: &Value,
    state: &Arc<Mutex<ServerState>>,
    session_id: &str,
) -> Option<Vec<Value>> {
    let tool = find_tool_event(parsed)?;

    let mut guard = state.lock().await;
    let session = guard.sessions.get_mut(session_id)?;
    let is_new = session.seen_tool_calls.insert(tool.id.clone());

    let mut updates = Vec::new();
    if is_new {
        updates.push(build_tool_call_update(tool.clone(), true));
    } else {
        updates.push(build_tool_call_update(tool.clone(), false));
    }

    Some(updates)
}

fn find_tool_event(parsed: &Value) -> Option<ToolEvent> {
    let source = parsed
        .get("tool_call")
        .or_else(|| parsed.get("toolCall"))
        .or_else(|| parsed.get("tool"))
        .unwrap_or(parsed);
    let id = find_string(source, &["toolCallId", "tool_call_id", "id"])?;
    let title = find_string(source, &["title", "tool_name", "toolName", "name"])
        .unwrap_or_else(|| "Tool call".to_string());
    let status = find_string(source, &["status", "state", "phase"])
        .and_then(|value| map_tool_status(&value));
    let raw_input = source
        .get("arguments")
        .or_else(|| source.get("input"))
        .or_else(|| source.get("params"))
        .cloned();
    let raw_output = source
        .get("output")
        .or_else(|| source.get("result"))
        .cloned();

    Some(ToolEvent {
        id,
        title,
        status,
        raw_input,
        raw_output,
    })
}

fn build_tool_call_update(tool: ToolEvent, is_new: bool) -> Value {
    let mut map = Map::new();
    map.insert(
        "sessionUpdate".to_string(),
        Value::String(if is_new {
            "tool_call".to_string()
        } else {
            "tool_call_update".to_string()
        }),
    );
    map.insert("toolCallId".to_string(), Value::String(tool.id));
    map.insert("title".to_string(), Value::String(tool.title));
    map.insert("kind".to_string(), Value::String("other".to_string()));
    if let Some(status) = tool.status {
        map.insert("status".to_string(), Value::String(status));
    }
    if let Some(raw_input) = tool.raw_input {
        map.insert("rawInput".to_string(), raw_input);
    }
    if let Some(raw_output) = tool.raw_output {
        map.insert("rawOutput".to_string(), raw_output);
    }
    Value::Object(map)
}

fn map_tool_status(value: &str) -> Option<String> {
    let value = value.to_lowercase();
    let status = match value.as_str() {
        "pending" | "queued" => "pending",
        "running" | "in_progress" | "in-progress" | "start" | "started" => "in_progress",
        "completed" | "done" | "success" => "completed",
        "failed" | "error" | "cancelled" | "canceled" => "failed",
        _ => return None,
    };
    Some(status.to_string())
}

fn extract_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.eq_ignore_ascii_case("done") || trimmed == "[DONE]" {
                return None;
            }
            Some(text.to_string())
        }
        Value::Array(items) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = extract_text(item) {
                    if !text.is_empty() {
                        parts.push(text);
                    }
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(""))
            }
        }
        Value::Object(map) => {
            for key in [
                "text",
                "content",
                "message",
                "delta",
                "response",
                "output",
                "assistant_message",
                "assistant",
            ] {
                if let Some(value) = map.get(key) {
                    if let Some(text) = extract_text(value) {
                        return Some(text);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn is_done_event(value: &Value) -> bool {
    let obj = match value.as_object() {
        Some(obj) => obj,
        None => return false,
    };
    for key in ["type", "event", "status"] {
        if let Some(val) = obj.get(key).and_then(|v| v.as_str()) {
            if val.eq_ignore_ascii_case("done") || val.eq_ignore_ascii_case("completed") {
                return true;
            }
        }
    }
    false
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    let obj = value.as_object()?;
    for key in keys {
        if let Some(val) = obj.get(*key).and_then(|v| v.as_str()) {
            return Some(val.to_string());
        }
    }
    None
}

async fn ensure_initialized(state: &Arc<Mutex<ServerState>>) -> Result<()> {
    let guard = state.lock().await;
    if guard.initialized {
        Ok(())
    } else {
        Err(rpc_invalid_request(
            "Must call initialize before session/new or session/prompt",
        ))
    }
}

fn map_error(err: &anyhow::Error) -> RpcError {
    if let Some(failure) = err.downcast_ref::<RpcFailure>() {
        return failure.0.clone();
    }
    if let Some(rovo_error) = err.downcast_ref::<RovoApiError>() {
        if rovo_error.is_auth_required() {
            return RpcError {
                code: ERR_AUTH_REQUIRED,
                message: "Authentication required".to_string(),
                data: Some(Value::String(rovo_error.to_string())),
            };
        }
    }

    RpcError {
        code: ERR_INTERNAL_ERROR,
        message: err.to_string(),
        data: None,
    }
}

async fn open_stream_with_backoff(rovo: &Arc<Mutex<RovoManager>>) -> Result<reqwest::Response> {
    let mut attempt = 0;
    let mut delay = Duration::from_millis(200);
    loop {
        attempt += 1;
        let result = {
            let rovo_guard = rovo.lock().await;
            rovo_guard.client.stream_chat(false).await
        };
        match result {
            Ok(resp) => return Ok(resp),
            Err(err) if attempt < 3 => {
                eprintln!("[rovo-dev-acp] stream_chat failed (attempt {attempt}), retrying: {err:#}");
                sleep(delay).await;
                delay = std::cmp::min(delay * 2, Duration::from_secs(5));
            }
            Err(err) => return Err(err),
        }
    }
}

#[derive(Default)]
struct ServerState {
    initialized: bool,
    sessions: HashMap<String, SessionState>,
    active_prompt: Option<ActivePrompt>,
    workdir: Option<PathBuf>,
}

#[derive(Clone)]
struct SessionState {
    rovo_session_id: String,
    cwd: PathBuf,
    seen_tool_calls: HashSet<String>,
}

struct ActivePrompt {
    cancel_tx: oneshot::Sender<()>,
}

#[derive(Clone)]
struct ToolEvent {
    id: String,
    title: String,
    status: Option<String>,
    raw_input: Option<Value>,
    raw_output: Option<Value>,
}

struct RovoManager {
    config: Config,
    client: RovoClient,
    process: Option<RovoProcess>,
}

impl RovoManager {
    fn new(config: Config) -> Self {
        Self {
            client: RovoClient::new(config.base_url.clone()),
            config,
            process: None,
        }
    }

    async fn ensure_running(&mut self, cwd: &Path) -> Result<()> {
        if !self.config.spawn {
            return self.wait_for_health().await;
        }

        if self.process.is_none() {
            let args = self.config.spawn_args();
            eprintln!(
                "[rovo-dev-acp] starting rovo dev server: {} {}",
                self.config.rovo_command,
                args.join(" ")
            );
            let process = RovoProcess::spawn(&self.config.rovo_command, &args, cwd).await?;
            self.process = Some(process);
        }

        self.wait_for_health().await
    }

    async fn wait_for_health(&self) -> Result<()> {
        let deadline = Instant::now() + self.config.startup_timeout;
        loop {
            match self.client.healthcheck().await {
                Ok(_) => return Ok(()),
                Err(err) => {
                    if Instant::now() >= deadline {
                        return Err(err).context("rovo dev server did not become healthy");
                    }
                }
            }
            sleep(Duration::from_millis(500)).await;
        }
    }
}

#[derive(Debug)]
struct RpcFailure(RpcError);

impl fmt::Display for RpcFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.message)
    }
}

impl std::error::Error for RpcFailure {}

fn rpc_invalid_params(message: impl Into<String>) -> anyhow::Error {
    RpcFailure(RpcError {
        code: ERR_INVALID_PARAMS,
        message: message.into(),
        data: None,
    })
    .into()
}

fn rpc_invalid_request(message: impl Into<String>) -> anyhow::Error {
    RpcFailure(RpcError {
        code: ERR_INVALID_REQUEST,
        message: message.into(),
        data: None,
    })
    .into()
}

fn rpc_method_not_found() -> anyhow::Error {
    RpcFailure(RpcError {
        code: ERR_METHOD_NOT_FOUND,
        message: "Method not found".to_string(),
        data: None,
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_from_object() {
        let value = serde_json::json!({"content": "hello"});
        assert_eq!(extract_text(&value), Some("hello".to_string()));
    }

    #[test]
    fn maps_tool_status() {
        assert_eq!(map_tool_status("running"), Some("in_progress".to_string()));
        assert_eq!(map_tool_status("done"), Some("completed".to_string()));
    }
}
