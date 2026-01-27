use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use ctx_core::models::SessionEventType;

use crate::adapters::{
    ProviderAdapter, ProviderCapabilities, ProviderHealth, ProviderProcessInfo, ProviderStatus,
    RunHandle, TurnInput,
};
use crate::events::NormalizedEvent;

const CRP_VERSION: u32 = 1;
const DEFAULT_CTX_MCP_TOOL_TIMEOUT_SECS: u64 = 2 * 60 * 60;
const CRP_MODEL_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Tier1CrpAdapter {
    id: String,
    command: String,
    pool: Arc<CrpSessionPool>,
}

impl Tier1CrpAdapter {
    fn new(id: &str, command: &str, args: Vec<String>) -> Self {
        let agent = CrpAgentConfig {
            provider_id: id.to_string(),
            command: command.to_string(),
            args: args.clone(),
        };
        Self {
            id: id.to_string(),
            command: command.to_string(),
            pool: Arc::new(CrpSessionPool::new(agent)),
        }
    }

    pub fn from_raw(id: &str, command: String, args: Vec<String>) -> Self {
        Self::new(id, &command, args)
    }

    pub fn codex() -> Self {
        Self::new("codex-crp", "codex-crp", vec![])
    }
}

#[async_trait]
impl ProviderAdapter for Tier1CrpAdapter {
    async fn inspect(&self) -> Result<ProviderStatus> {
        let detected_path = {
            let p = std::path::Path::new(&self.command);
            if p.is_absolute() || self.command.contains(std::path::MAIN_SEPARATOR) {
                if p.exists() {
                    Some(p.to_path_buf())
                } else {
                    None
                }
            } else {
                which::which(&self.command).ok()
            }
        };
        let installed = detected_path.is_some();
        let mut diagnostics = Vec::new();
        if !installed {
            diagnostics.push(format!(
                "CRP runtime executable not found: {}",
                self.command
            ));
        }

        Ok(ProviderStatus {
            provider_id: self.id.clone(),
            installed,
            detected_path: detected_path.map(|p| p.to_string_lossy().to_string()),
            version: None,
            capabilities: if installed {
                Some(default_caps(&self.id))
            } else {
                None
            },
            health: if installed {
                ProviderHealth::Ok
            } else {
                ProviderHealth::Missing
            },
            diagnostics,
            details: HashMap::new(),
        })
    }

    async fn run(
        &self,
        input: TurnInput,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<RunHandle> {
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let pool = Arc::clone(&self.pool);
        let error_sink = event_sink.clone();
        let join = tokio::spawn(async move {
            let session_key = env
                .get("CTX_SESSION_ID")
                .cloned()
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let request = CrpPromptRequest {
                session_key,
                input,
                workdir,
                env,
                event_sink,
                cancel_rx,
            };
            if let Err(err) = pool.prompt(request).await {
                let _ = error_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::Error,
                        payload_json: json!({ "message": err.to_string() }),
                    })
                    .await;
            }
            let _ = done_tx.send(());
        });
        let abort = join.abort_handle();

        Ok(RunHandle {
            done: done_rx,
            cancel: Some(cancel_tx),
            abort: Some(abort),
        })
    }

    async fn cancel(&self, mut handle: RunHandle) -> Result<()> {
        if let Some(cancel) = handle.cancel.take() {
            let _ = cancel.send(());
        }
        let done = tokio::time::timeout(std::time::Duration::from_secs(2), &mut handle.done).await;
        if done.is_err() {
            if let Some(abort) = handle.abort.take() {
                abort.abort();
            }
        }
        Ok(())
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        self.pool.list_processes().await
    }

    async fn has_live_session(&self, session_key: &str) -> bool {
        self.pool.has_session(session_key).await
    }
}

fn default_caps(id: &str) -> ProviderCapabilities {
    ProviderCapabilities {
        stream_events: true,
        stream_format: "crp-jsonl".into(),
        has_turn_boundaries: true,
        has_tool_call_ids: true,
        has_file_change_events: false,
        has_command_events: false,
        supports_resume: matches!(id, "codex-crp"),
        supports_stable_session_id: true,
        supports_fork_or_rewind: false,
        supports_headless: true,
        supports_server_mode: false,
        supports_acp: false,
        supports_interactive_tui: false,
        supports_private_state_dir: false,
        supports_sandbox_flags: false,
        supports_approval_flags: false,
        notes: vec![],
    }
}

#[derive(Debug, Clone)]
struct CrpAgentConfig {
    provider_id: String,
    command: String,
    args: Vec<String>,
}

struct CrpSessionPool {
    agent: CrpAgentConfig,
    sessions: Mutex<HashMap<String, Arc<CrpSession>>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl CrpSessionPool {
    fn new(agent: CrpAgentConfig) -> Self {
        Self {
            agent,
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    async fn list_processes(&self) -> Vec<ProviderProcessInfo> {
        let sessions = self.sessions.lock().await;
        let mut out = Vec::new();
        for (session_id, session) in sessions.iter() {
            if let Some(pid) = session.process.pid().await {
                out.push(ProviderProcessInfo {
                    provider_id: self.agent.provider_id.clone(),
                    pid,
                    label: Some(session_id.clone()),
                });
            }
        }
        out
    }

    async fn has_session(&self, session_key: &str) -> bool {
        let sessions = self.sessions.lock().await;
        sessions.contains_key(session_key)
    }

    async fn prompt(&self, req: CrpPromptRequest) -> Result<()> {
        let _guard =
            ActivePromptGuard::new(Arc::clone(&self.active_prompts), req.session_key.clone())?;
        let session = self
            .get_or_create_session(&req.session_key, &req.workdir, &req.env)
            .await?;

        let turn_id = format!("crp-{}", Uuid::new_v4());
        let mut rx = session.process.events.subscribe();

        if !session.opened.load(Ordering::SeqCst) {
            let config = build_crp_session_config(&req.env, &req.workdir);
            session
                .process
                .send(CrpCommand::SessionOpen {
                    session_id: Some(req.session_key.clone()),
                    config: Some(config),
                })
                .await?;
            session.opened.store(true, Ordering::SeqCst);
        }
        let items = build_prompt_items(&req.input, &req.workdir, &req.env).await?;
        session
            .process
            .send(CrpCommand::SessionPrompt {
                session_id: Some(req.session_key.clone()),
                turn_id: Some(turn_id.clone()),
                items: Some(items),
                prompt: Some(req.input.content.clone()),
                model: req.input.model_id.clone(),
                cwd: Some(req.workdir.clone()),
            })
            .await?;
        let mut last_seq = 0u64;
        let mut tool_output_cache: HashMap<String, String> = HashMap::new();
        let mut cancel_rx = req.cancel_rx;
        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    let _ = session.process.send(CrpCommand::SessionCancel {
                        session_id: Some(req.session_key.clone()),
                        turn_id: Some(turn_id.clone()),
                    }).await;
                    break;
                }
                recv = rx.recv() => {
                    match recv {
                        Ok(env) => {
                            if !event_matches_session(&env.event, &req.session_key) {
                                continue;
                            }
                            if let Some(event_turn_id) = event_turn_id(&env.event) {
                                if event_turn_id != turn_id {
                                    continue;
                                }
                            }
                            if env.seq <= last_seq {
                                continue;
                            }
                            last_seq = env.seq;
                            let mapped = map_crp_event(env.event, env.seq, &mut tool_output_cache);
                            for event in mapped.events {
                                let _ = req.event_sink.send(event).await;
                            }
                            if mapped.done {
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let payload = json!({
                                "kind": "session_gap",
                                "reason": "crp_receiver_lagged",
                            });
                            let _ = req
                                .event_sink
                                .send(NormalizedEvent {
                                    event_type: SessionEventType::Notice,
                                    payload_json: payload,
                                })
                                .await;
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }

        Ok(())
    }

    async fn get_or_create_session(
        &self,
        session_key: &str,
        workdir: &PathBuf,
        env: &HashMap<String, String>,
    ) -> Result<Arc<CrpSession>> {
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(session_key) {
            return Ok(Arc::clone(existing));
        }

        let process = CrpProcess::spawn(&self.agent, workdir, env)
            .await
            .with_context(|| format!("spawning CRP runtime {}", self.agent.command))?;
        let session = Arc::new(CrpSession {
            process,
            opened: AtomicBool::new(false),
        });
        sessions.insert(session_key.to_string(), Arc::clone(&session));
        Ok(session)
    }
}

struct ActivePromptGuard {
    session_key: String,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl ActivePromptGuard {
    fn new(active_prompts: Arc<StdMutex<HashSet<String>>>, session_key: String) -> Result<Self> {
        if let Ok(mut active) = active_prompts.lock() {
            if active.contains(&session_key) {
                anyhow::bail!("session {session_key} already has an active prompt");
            }
            active.insert(session_key.clone());
        }
        Ok(Self {
            session_key,
            active_prompts,
        })
    }
}

impl Drop for ActivePromptGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_prompts.lock() {
            active.remove(&self.session_key);
        }
    }
}

struct CrpSession {
    process: Arc<CrpProcess>,
    opened: AtomicBool,
}

struct CrpPromptRequest {
    session_key: String,
    input: TurnInput,
    workdir: PathBuf,
    env: HashMap<String, String>,
    event_sink: mpsc::Sender<NormalizedEvent>,
    cancel_rx: oneshot::Receiver<()>,
}

struct CrpProcess {
    agent: CrpAgentConfig,
    child: Mutex<Child>,
    pid: AtomicU32,
    write_tx: mpsc::UnboundedSender<String>,
    events: broadcast::Sender<CrpEventEnvelope>,
}

impl CrpProcess {
    async fn spawn(
        agent: &CrpAgentConfig,
        workdir: &PathBuf,
        env: &HashMap<String, String>,
    ) -> Result<Arc<Self>> {
        let mut cmd = Command::new(&agent.command);
        cmd.args(&agent.args);
        cmd.current_dir(workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);

        let stdin = child.stdin.take().context("capturing CRP stdin")?;
        let stdout = child.stdout.take().context("capturing CRP stdout")?;
        let stderr = child.stderr.take().context("capturing CRP stderr")?;

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let _writer = tokio::spawn(async move {
            let mut stdin = tokio::io::BufWriter::new(stdin);
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

        let (events, _) = broadcast::channel(512);
        let process = Arc::new(Self {
            agent: agent.clone(),
            child: Mutex::new(child),
            pid: AtomicU32::new(pid),
            write_tx,
            events,
        });

        let stdout_process = Arc::clone(&process);
        tokio::spawn(async move {
            stdout_pump(stdout_process, stdout).await;
        });
        let stderr_process = Arc::clone(&process);
        tokio::spawn(async move {
            stderr_pump(stderr_process, stderr).await;
        });

        Ok(process)
    }

    async fn pid(&self) -> Option<u32> {
        let pid = self.pid.load(Ordering::Relaxed);
        if pid != 0 {
            return Some(pid);
        }
        let child = self.child.lock().await;
        child.id()
    }

    async fn send(&self, command: CrpCommand) -> Result<()> {
        let envelope = CrpCommandEnvelope {
            v: CRP_VERSION,
            command,
        };
        let line = serde_json::to_string(&envelope)?;
        self.write_tx
            .send(line)
            .map_err(|_| anyhow::anyhow!("crp runtime stdin closed"))?;
        Ok(())
    }
}

async fn stdout_pump(process: Arc<CrpProcess>, stdout: impl tokio::io::AsyncRead + Unpin) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<CrpEventEnvelope>(trimmed) {
            Ok(env) => {
                let _ = process.events.send(env);
            }
            Err(err) => {
                tracing::warn!(
                    provider_id = %process.agent.provider_id,
                    error = %err,
                    "failed to parse CRP event"
                );
            }
        }
    }
}

async fn stderr_pump(process: Arc<CrpProcess>, stderr: impl tokio::io::AsyncRead + Unpin) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        tracing::debug!(
            provider_id = %process.agent.provider_id,
            "crp stderr: {}",
            trimmed
        );
    }
}

#[derive(Debug, Serialize)]
struct CrpCommandEnvelope {
    v: u32,
    #[serde(flatten)]
    command: CrpCommand,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)]
enum CrpCommand {
    #[serde(rename = "session.open")]
    SessionOpen {
        session_id: Option<String>,
        config: Option<CrpSessionConfig>,
    },
    #[serde(rename = "session.prompt")]
    SessionPrompt {
        session_id: Option<String>,
        turn_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        items: Option<Vec<Value>>,
        prompt: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    #[serde(rename = "session.cancel")]
    SessionCancel {
        session_id: Option<String>,
        turn_id: Option<String>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<CrpSessionConfig>,
    },
}

#[derive(Debug, Serialize)]
struct CrpSessionConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_trace_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mcp_servers: Option<HashMap<String, CrpMcpServerConfig>>,
}

#[derive(Debug, Serialize)]
struct CrpMcpServerConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_timeout_sec: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CrpModelInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CrpModelsProbe {
    pub models: Vec<CrpModelInfo>,
    pub current_model_id: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
struct CrpEventEnvelope {
    #[allow(dead_code)]
    #[serde(default)]
    v: Option<u32>,
    seq: u64,
    #[allow(dead_code)]
    channel: CrpChannel,
    #[serde(flatten)]
    event: CrpEvent,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
enum CrpChannel {
    Control,
    Data,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
enum CrpEvent {
    #[serde(rename = "session.opened")]
    SessionOpened {
        session_id: String,
        provider_session_id: Option<String>,
    },
    #[serde(rename = "turn.started")]
    TurnStarted {
        session_id: String,
        run_id: String,
        turn_id: String,
    },
    #[serde(rename = "message.delta")]
    MessageDelta {
        session_id: String,
        run_id: String,
        turn_id: String,
        message_id: String,
        delta: String,
    },
    #[serde(rename = "message.final")]
    MessageFinal {
        session_id: String,
        run_id: String,
        turn_id: String,
        message_id: String,
        content: String,
    },
    #[serde(rename = "reasoning.summary")]
    ReasoningSummary {
        session_id: String,
        run_id: String,
        turn_id: String,
        text: String,
        #[serde(default)]
        item_id: Option<String>,
    },
    #[serde(rename = "reasoning.trace")]
    ReasoningTrace {
        session_id: String,
        run_id: String,
        turn_id: String,
        chunk: String,
        #[serde(default)]
        encoding: Option<String>,
    },
    #[serde(rename = "tool.started")]
    ToolStarted {
        session_id: String,
        run_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        #[serde(default)]
        input: Option<Value>,
    },
    #[serde(rename = "tool.output.delta")]
    ToolOutputDelta {
        session_id: String,
        run_id: String,
        turn_id: String,
        tool_call_id: String,
        #[serde(default)]
        chunk: String,
    },
    #[serde(rename = "tool.completed")]
    ToolCompleted {
        session_id: String,
        run_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        status: CrpToolStatus,
        #[serde(default)]
        output: Option<Value>,
        #[serde(default)]
        error: Option<String>,
    },
    #[serde(rename = "models.list")]
    ModelsList {
        models: Vec<CrpModelInfo>,
        #[serde(default)]
        current_model_id: Option<String>,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        session_id: String,
        run_id: String,
        turn_id: String,
        status: CrpTurnStatus,
    },
    #[serde(rename = "session.gap")]
    SessionGap {
        session_id: String,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        reason: Option<String>,
    },
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum CrpTurnStatus {
    Success,
    Error,
    Canceled,
    Interrupted,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
enum CrpToolStatus {
    Success,
    Error,
}

struct MappedCrpEvent {
    events: Vec<NormalizedEvent>,
    done: bool,
}

fn map_crp_event(
    event: CrpEvent,
    seq: u64,
    tool_output_cache: &mut HashMap<String, String>,
) -> MappedCrpEvent {
    match event {
        CrpEvent::SessionOpened {
            session_id,
            provider_session_id,
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Init,
                payload_json: json!({
                    "session_id": session_id,
                    "provider_session_id": provider_session_id,
                }),
            }],
            done: false,
        },
        CrpEvent::TurnStarted { .. } => MappedCrpEvent {
            events: Vec::new(),
            done: false,
        },
        CrpEvent::MessageDelta {
            delta, message_id, ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::AssistantChunk,
                payload_json: json!({
                    "content_fragment": delta,
                    "message_id": message_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::MessageFinal {
            content,
            message_id,
            ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::AssistantComplete,
                payload_json: json!({
                    "full_content": content,
                    "message_id": message_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ReasoningSummary { text, item_id, .. } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: json!({
                    "kind": "reasoning_summary",
                    "text": text,
                    "item_id": item_id,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ReasoningTrace {
            chunk, encoding, ..
        } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::ThoughtChunk,
                payload_json: json!({
                    "content_fragment": chunk,
                    "encoding": encoding,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
        CrpEvent::ToolStarted {
            tool_call_id,
            tool_name,
            input,
            ..
        } => {
            let payload = build_tool_started_payload(tool_call_id, tool_name, input, seq);
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCall,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::ToolOutputDelta {
            tool_call_id,
            chunk,
            ..
        } => {
            let output_text = {
                let entry = tool_output_cache.entry(tool_call_id.clone()).or_default();
                entry.push_str(&chunk);
                entry.clone()
            };
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolCallUpdate,
                    payload_json: json!({
                        "tool_call_id": tool_call_id,
                        "outputText": output_text,
                        "status": "running",
                        "crp_seq": seq,
                    }),
                }],
                done: false,
            }
        }
        CrpEvent::ToolCompleted {
            tool_call_id,
            tool_name,
            status,
            output,
            error,
            ..
        } => {
            let tool_call_id_for_cache = tool_call_id.clone();
            let payload =
                build_tool_completed_payload(tool_call_id, tool_name, status, output, error, seq);
            tool_output_cache.remove(&tool_call_id_for_cache);
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: payload,
                }],
                done: false,
            }
        }
        CrpEvent::TurnCompleted { status, .. } => {
            let (event_type, payload) = match status {
                CrpTurnStatus::Success => (
                    SessionEventType::Done,
                    json!({"status": "completed", "crp_seq": seq}),
                ),
                CrpTurnStatus::Error => (
                    SessionEventType::Error,
                    json!({"message": "crp_turn_error", "crp_seq": seq}),
                ),
                CrpTurnStatus::Canceled | CrpTurnStatus::Interrupted => (
                    SessionEventType::TurnInterrupted,
                    json!({"reason": "crp_turn_interrupted", "crp_seq": seq}),
                ),
            };
            MappedCrpEvent {
                events: vec![NormalizedEvent {
                    event_type,
                    payload_json: payload,
                }],
                done: true,
            }
        }
        CrpEvent::ModelsList { .. } => MappedCrpEvent {
            events: Vec::new(),
            done: false,
        },
        CrpEvent::SessionGap { reason, .. } => MappedCrpEvent {
            events: vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: json!({
                    "kind": "session_gap",
                    "reason": reason,
                    "crp_seq": seq,
                }),
            }],
            done: false,
        },
    }
}

fn build_tool_started_payload(
    tool_call_id: String,
    tool_name: String,
    input: Option<Value>,
    seq: u64,
) -> Value {
    let mut payload = serde_json::Map::new();
    let tool_name_for_call = tool_name.clone();
    payload.insert("tool_call_id".to_string(), json!(tool_call_id.clone()));
    payload.insert("kind".to_string(), json!(tool_name.clone()));
    payload.insert("status".to_string(), json!("running"));
    if let Some(input) = input.clone() {
        payload.insert("rawInput".to_string(), input);
    }
    payload.insert(
        "toolCall".to_string(),
        json!({
            "id": tool_call_id,
            "name": tool_name_for_call.clone(),
            "kind": tool_name_for_call,
            "rawInput": input,
            "status": "running",
        }),
    );
    payload.insert("crp_seq".to_string(), json!(seq));
    Value::Object(payload)
}

fn build_tool_completed_payload(
    tool_call_id: String,
    tool_name: String,
    status: CrpToolStatus,
    output: Option<Value>,
    error: Option<String>,
    seq: u64,
) -> Value {
    let mut payload = serde_json::Map::new();
    let tool_name_for_call = tool_name.clone();
    payload.insert("tool_call_id".to_string(), json!(tool_call_id.clone()));
    payload.insert("kind".to_string(), json!(tool_name.clone()));
    payload.insert(
        "status".to_string(),
        json!(match status {
            CrpToolStatus::Success => "completed",
            CrpToolStatus::Error => "failed",
        }),
    );
    if let Some(output) = output.clone() {
        if let Some(text) = extract_output_text(&output) {
            payload.insert("output_text".to_string(), json!(text));
        }
        payload.insert("rawOutput".to_string(), output);
    }
    if let Some(err) = error {
        payload.insert("error".to_string(), json!(err));
    }
    payload.insert(
        "toolCall".to_string(),
        json!({
            "id": tool_call_id,
            "name": tool_name_for_call.clone(),
            "kind": tool_name_for_call,
            "rawOutput": payload.get("rawOutput").cloned(),
            "status": payload.get("status").cloned(),
        }),
    );
    payload.insert("crp_seq".to_string(), json!(seq));
    Value::Object(payload)
}

fn extract_output_text(output: &Value) -> Option<String> {
    if let Some(text) = output.get("formatted_output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("aggregated_output").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("stdout").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    if let Some(text) = output.get("result").and_then(Value::as_str) {
        return Some(text.to_string());
    }
    output.as_str().map(|text| text.to_string())
}

fn event_turn_id(event: &CrpEvent) -> Option<&str> {
    match event {
        CrpEvent::SessionGap { turn_id, .. } => turn_id.as_deref(),
        CrpEvent::TurnStarted { turn_id, .. }
        | CrpEvent::MessageDelta { turn_id, .. }
        | CrpEvent::MessageFinal { turn_id, .. }
        | CrpEvent::ReasoningSummary { turn_id, .. }
        | CrpEvent::ReasoningTrace { turn_id, .. }
        | CrpEvent::ToolStarted { turn_id, .. }
        | CrpEvent::ToolOutputDelta { turn_id, .. }
        | CrpEvent::ToolCompleted { turn_id, .. }
        | CrpEvent::TurnCompleted { turn_id, .. } => Some(turn_id.as_str()),
        CrpEvent::SessionOpened { .. } | CrpEvent::ModelsList { .. } => None,
    }
}

fn event_matches_session(event: &CrpEvent, session_id: &str) -> bool {
    match event {
        CrpEvent::SessionOpened { session_id: id, .. }
        | CrpEvent::TurnStarted { session_id: id, .. }
        | CrpEvent::MessageDelta { session_id: id, .. }
        | CrpEvent::MessageFinal { session_id: id, .. }
        | CrpEvent::ReasoningSummary { session_id: id, .. }
        | CrpEvent::ReasoningTrace { session_id: id, .. }
        | CrpEvent::ToolStarted { session_id: id, .. }
        | CrpEvent::ToolOutputDelta { session_id: id, .. }
        | CrpEvent::ToolCompleted { session_id: id, .. }
        | CrpEvent::TurnCompleted { session_id: id, .. }
        | CrpEvent::SessionGap { session_id: id, .. } => id == session_id,
        CrpEvent::ModelsList { .. } => false,
    }
}

fn build_crp_session_config(env: &HashMap<String, String>, workdir: &Path) -> CrpSessionConfig {
    let mcp_enabled = env
        .get("CTX_MCP_DISABLED")
        .map(|v| v != "1" && v.to_lowercase() != "true")
        .unwrap_or(true);

    let mcp_servers = if mcp_enabled {
        let mut mcp_env = HashMap::new();
        if let Some(url) = env.get("CTX_DAEMON_URL") {
            mcp_env.insert("CTX_DAEMON_URL".to_string(), url.clone());
        }
        if let Some(token) = env.get("CTX_AUTH_TOKEN") {
            mcp_env.insert("CTX_AUTH_TOKEN".to_string(), token.clone());
        }
        if let Some(session_id) = env.get("CTX_SESSION_ID") {
            mcp_env.insert("CTX_SESSION_ID".to_string(), session_id.clone());
        }
        if let Some(token) = env.get("CTX_MCP_TOKEN") {
            mcp_env.insert("CTX_MCP_TOKEN".to_string(), token.clone());
        }

        let mcp_command = env
            .get("CTX_MCP_COMMAND")
            .cloned()
            .unwrap_or_else(|| "ctx-mcp".to_string());
        let tool_timeout_sec = env
            .get("CTX_MCP_TOOL_TIMEOUT_SEC")
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CTX_MCP_TOOL_TIMEOUT_SECS);

        let mut map = HashMap::new();
        map.insert(
            "ctx".to_string(),
            CrpMcpServerConfig {
                command: Some(mcp_command),
                args: Some(vec!["--stdio".to_string()]),
                env: Some(mcp_env),
                tool_timeout_sec: Some(tool_timeout_sec as f64),
            },
        );
        Some(map)
    } else {
        None
    };

    CrpSessionConfig {
        cwd: Some(workdir.to_path_buf()),
        model: env.get("CTX_MODEL_ID").cloned(),
        model_provider: None,
        reasoning_trace_enabled: Some(true),
        mcp_servers,
    }
}

fn build_crp_model_probe_config(env: &HashMap<String, String>, workdir: &Path) -> CrpSessionConfig {
    CrpSessionConfig {
        cwd: Some(workdir.to_path_buf()),
        model: env.get("CTX_MODEL_ID").cloned(),
        model_provider: None,
        reasoning_trace_enabled: None,
        mcp_servers: None,
    }
}

pub async fn probe_crp_models(
    provider_id: &str,
    command: String,
    args: Vec<String>,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<CrpModelsProbe> {
    let mut cmd = Command::new(&command);
    cmd.args(&args);
    cmd.current_dir(&workdir);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    for (k, v) in &env {
        cmd.env(k, v);
    }

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning CRP runtime {provider_id} ({command})"))?;
    let stdin = child.stdin.take().context("capturing CRP stdin")?;
    let stdout = child.stdout.take().context("capturing CRP stdout")?;
    let stderr = child.stderr.take().context("capturing CRP stderr")?;

    let stderr_provider = provider_id.to_string();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            tracing::debug!(
                provider_id = %stderr_provider,
                "crp stderr: {}",
                trimmed
            );
        }
    });

    let mut stdin = BufWriter::new(stdin);
    let mut stdout_reader = BufReader::new(stdout).lines();
    let config = build_crp_model_probe_config(&env, &workdir);
    let envelope = CrpCommandEnvelope {
        v: CRP_VERSION,
        command: CrpCommand::ModelsList {
            config: Some(config),
        },
    };
    let line = serde_json::to_string(&envelope)?;
    stdin.write_all(line.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    stdin.flush().await?;

    let result = timeout(CRP_MODEL_PROBE_TIMEOUT, async {
        loop {
            let Some(line) = stdout_reader.next_line().await? else {
                anyhow::bail!("crp runtime closed before models.list response");
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let env = match serde_json::from_str::<CrpEventEnvelope>(trimmed) {
                Ok(env) => env,
                Err(_) => continue,
            };
            if let CrpEvent::ModelsList {
                models,
                current_model_id,
            } = env.event
            {
                return Ok(CrpModelsProbe {
                    models,
                    current_model_id,
                });
            }
        }
    })
    .await
    .context("CRP models.list probe timed out")??;

    let _ = child.kill().await;
    let _ = child.wait().await;

    Ok(result)
}

async fn build_prompt_items(
    input: &TurnInput,
    _workdir: &PathBuf,
    env: &HashMap<String, String>,
) -> Result<Vec<Value>> {
    let mut items = Vec::new();
    for block in &input.context_blocks {
        if block
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| matches!(t, "text" | "image" | "local_image" | "skill"))
        {
            items.push(block.clone());
        }
    }

    let data_root = env.get("CTX_DATA_ROOT").cloned();
    for att in input.attachments.iter() {
        match att {
            ctx_core::models::MessageAttachment::Image {
                mime_type,
                data_base64,
                ..
            } => {
                items.push(json!({
                    "type": "image",
                    "image_url": format!("data:{mime_type};base64,{data_base64}"),
                }));
            }
            ctx_core::models::MessageAttachment::ImageRef {
                blob_id, mime_type, ..
            } => {
                let Some(data_root) = data_root.as_deref() else {
                    anyhow::bail!("missing CTX_DATA_ROOT for image attachment");
                };
                let path = std::path::Path::new(data_root).join("blobs").join(blob_id);
                let bytes = tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("reading image blob {blob_id}"))?;
                let data_base64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                items.push(json!({
                    "type": "image",
                    "image_url": format!("data:{mime_type};base64,{data_base64}"),
                }));
            }
        }
    }

    items.push(json!({"type":"text","text": input.content}));
    Ok(items)
}
