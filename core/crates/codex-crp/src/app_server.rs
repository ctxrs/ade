use anyhow::{anyhow, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::{Mutex as StdMutex, OnceLock};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

const CODEX_APP_SERVER_ARGS: [&str; 5] = ["-s", "danger-full-access", "-a", "never", "app-server"];
const CODEX_RAW_EVENT_DUMP_ENV: &str = "CODEX_CRP_DUMP_CODEX_EVENTS_PATH";

static APP_SERVER_EVENT_DUMP: OnceLock<StdMutex<std::io::BufWriter<std::fs::File>>> =
    OnceLock::new();
static APP_SERVER_EVENT_DUMP_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub enum AppServerInbound {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: i64,
        method: String,
        params: Value,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRef {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartLikeResponse {
    pub thread: ThreadRef,
    pub model: String,
    pub cwd: String,
    #[serde(rename = "approvalPolicy")]
    pub _approval_policy: Value,
    #[serde(rename = "sandbox")]
    pub _sandbox: Value,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRef {
    pub id: String,
    pub status: String,
    pub error: Option<TurnError>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartedNotification {
    pub thread_id: String,
    pub turn: TurnRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompletedNotification {
    pub thread_id: String,
    pub turn: TurnRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResponse {
    pub turn: TurnRef,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnError {
    pub message: String,
    pub codex_error_info: Option<Value>,
    pub additional_details: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsageUpdatedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub token_usage: ThreadTokenUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTokenUsage {
    pub total: TokenUsageBreakdown,
    pub last: TokenUsageBreakdown,
    pub model_context_window: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageBreakdown {
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelListResponse {
    pub data: Vec<ModelInfo>,
    #[serde(rename = "nextCursor")]
    pub _next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub hidden: bool,
    pub supported_reasoning_efforts: Vec<ReasoningEffortOption>,
    pub default_reasoning_effort: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEffortOption {
    pub reasoning_effort: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemLifecycleNotification {
    pub item: ThreadItem,
    pub thread_id: String,
    pub turn_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadItem {
    #[serde(rename = "agentMessage")]
    AgentMessage {
        id: String,
        text: String,
        #[serde(rename = "phase")]
        #[serde(default)]
        _phase: Option<String>,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        summary: Vec<String>,
        content: Vec<String>,
    },
    #[serde(rename = "commandExecution")]
    CommandExecution {
        id: String,
        command: String,
        cwd: String,
        #[serde(rename = "processId")]
        #[serde(default)]
        _process_id: Option<String>,
        status: String,
        #[serde(rename = "commandActions")]
        #[serde(default)]
        command_actions: Vec<Value>,
        #[serde(rename = "aggregatedOutput")]
        #[serde(default)]
        aggregated_output: Option<String>,
        #[serde(rename = "exitCode")]
        #[serde(default)]
        exit_code: Option<i64>,
        #[serde(rename = "durationMs")]
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    #[serde(rename = "fileChange")]
    FileChange {
        id: String,
        changes: Vec<FileUpdateChange>,
        status: String,
    },
    #[serde(rename = "mcpToolCall")]
    McpToolCall {
        id: String,
        server: String,
        tool: String,
        status: String,
        arguments: Value,
        #[serde(rename = "result")]
        #[serde(default)]
        result: Option<Value>,
        #[serde(rename = "error")]
        #[serde(default)]
        error: Option<McpToolCallError>,
        #[serde(rename = "durationMs")]
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    #[serde(rename = "webSearch")]
    WebSearch {
        id: String,
        query: String,
        #[serde(rename = "action")]
        #[serde(default)]
        _action: Option<Value>,
    },
    #[serde(rename = "imageView")]
    ImageView { id: String, path: String },
    #[serde(rename = "enteredReviewMode")]
    EnteredReviewMode {
        #[serde(rename = "id")]
        _id: String,
        #[serde(rename = "review")]
        _review: String,
    },
    #[serde(rename = "exitedReviewMode")]
    ExitedReviewMode {
        #[serde(rename = "id")]
        _id: String,
        #[serde(rename = "review")]
        _review: String,
    },
    #[serde(rename = "contextCompaction")]
    ContextCompaction {
        #[serde(rename = "id")]
        _id: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: String,
    pub diff: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpToolCallError {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningSummaryTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    pub summary_index: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningSummaryPartAddedNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub summary_index: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningTextDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
    pub content_index: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandExecutionOutputDeltaNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub delta: String,
}

#[derive(Debug)]
struct PendingResponse {
    respond_to: oneshot::Sender<Result<Value>>,
    method: String,
}

pub struct AppServerClient {
    pending: Arc<Mutex<HashMap<i64, PendingResponse>>>,
    stdin: Arc<Mutex<BufWriter<ChildStdin>>>,
    inbound_rx: mpsc::UnboundedReceiver<AppServerInbound>,
    child: Child,
    next_id: i64,
}

fn maybe_dump_app_server_message(direction: &str, value: &Value) {
    let Ok(path) = std::env::var(CODEX_RAW_EVENT_DUMP_ENV) else {
        return;
    };
    let writer = APP_SERVER_EVENT_DUMP.get_or_init(|| {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("failed to open CODEX_CRP_DUMP_CODEX_EVENTS_PATH");
        StdMutex::new(std::io::BufWriter::new(file))
    });

    let Ok(mut writer) = writer.lock() else {
        return;
    };
    let event = json!({
        "i": APP_SERVER_EVENT_DUMP_SEQ.fetch_add(1, Ordering::Relaxed),
        "direction": direction,
        "event": value,
    });
    if serde_json::to_writer(&mut *writer, &event).is_ok() {
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

impl AppServerClient {
    pub async fn start(workdir: &Path) -> Result<Self> {
        let codex_bin = std::env::var("CTX_CODEX_BIN_PATH")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "codex".to_string());

        let mut command = Command::new(codex_bin);
        command
            .args(CODEX_APP_SERVER_ARGS)
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command.spawn().context("spawning codex app-server")?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("codex app-server stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("codex app-server stdout unavailable"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("codex app-server stderr unavailable"))?;

        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel();

        tokio::spawn(stdout_reader_task(
            stdout,
            inbound_tx.clone(),
            Arc::clone(&pending),
        ));
        tokio::spawn(stderr_reader_task(stderr));

        let mut client = Self {
            pending,
            stdin: Arc::new(Mutex::new(BufWriter::new(stdin))),
            inbound_rx,
            child,
            next_id: 1,
        };

        client
            .request::<Value>(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "codex-crp",
                        "title": "ctx",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "capabilities": {
                        "experimentalApi": true,
                        "optOutNotificationMethods": [],
                    }
                }),
            )
            .await?;
        client.notify("initialized", json!({})).await?;

        Ok(client)
    }

    pub async fn request<T>(&mut self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let id = self.next_id;
        self.next_id += 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            id,
            PendingResponse {
                respond_to: tx,
                method: method.to_string(),
            },
        );
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        let result = rx
            .await
            .map_err(|_| anyhow!("app-server response channel closed for {method}"))??;
        serde_json::from_value(result).with_context(|| format!("decoding `{method}` response"))
    }

    pub async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    pub async fn reject_request(&mut self, id: i64, message: String) -> Result<()> {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32000,
                "message": message,
            }
        }))
        .await
    }

    pub async fn next_inbound(&mut self) -> Option<AppServerInbound> {
        self.inbound_rx.recv().await
    }

    pub async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }

    async fn send_json(&mut self, value: &Value) -> Result<()> {
        maybe_dump_app_server_message("outbound", value);
        let mut stdin = self.stdin.lock().await;
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        stdin.write_all(&bytes).await?;
        stdin.flush().await?;
        Ok(())
    }
}

async fn stdout_reader_task(
    stdout: tokio::process::ChildStdout,
    inbound_tx: mpsc::UnboundedSender<AppServerInbound>,
    pending: Arc<Mutex<HashMap<i64, PendingResponse>>>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let parsed: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(?err, "failed to decode app-server line");
                continue;
            }
        };
        maybe_dump_app_server_message("inbound", &parsed);
        let Some(object) = parsed.as_object() else {
            continue;
        };

        if let Some(id) = object.get("id").and_then(request_id_from_value) {
            if object.get("method").is_none() {
                let pending_entry = pending.lock().await.remove(&id);
                if let Some(pending_entry) = pending_entry {
                    let result = if let Some(error) = object.get("error") {
                        Err(anyhow!(
                            "{}: {}",
                            pending_entry.method,
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown app-server error")
                        ))
                    } else {
                        Ok(object.get("result").cloned().unwrap_or(Value::Null))
                    };
                    let _ = pending_entry.respond_to.send(result);
                }
                continue;
            }

            let method = object
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let params = object.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx.send(AppServerInbound::Request { id, method, params });
            continue;
        }

        if let Some(method) = object.get("method").and_then(Value::as_str) {
            let params = object.get("params").cloned().unwrap_or(Value::Null);
            let _ = inbound_tx.send(AppServerInbound::Notification {
                method: method.to_string(),
                params,
            });
        }
    }
}

async fn stderr_reader_task(stderr: tokio::process::ChildStderr) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        eprintln!("{line}");
    }
}

fn request_id_from_value(value: &Value) -> Option<i64> {
    if let Some(id) = value.as_i64() {
        return Some(id);
    }
    value.as_str().and_then(|id| id.parse::<i64>().ok())
}

#[cfg(test)]
impl AppServerClient {
    pub fn test_stub() -> Self {
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn cat");
        let stdin = child.stdin.take().expect("cat stdin");
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            stdin: Arc::new(Mutex::new(BufWriter::new(stdin))),
            inbound_rx: {
                let (_tx, rx) = mpsc::unbounded_channel();
                rx
            },
            child,
            next_id: 1,
        }
    }
}
