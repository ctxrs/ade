use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex as StdMutex,
};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::time::timeout;

use ctx_core::models::SessionEventType;

use crate::ask_user_question::{
    AskUserQuestionAnswer, AskUserQuestionBroker, AskUserQuestionOutcome,
};
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
    ask_user_question: Option<Arc<AskUserQuestionBroker>>,
    process: Mutex<Option<Arc<AcpProcess>>>,
    sessions: Mutex<HashMap<String, AcpContextSession>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

#[derive(Debug, Clone)]
struct AcpContextSession {
    acp_session_id: String,
}

#[derive(Debug, Clone)]
struct CreatedAcpSession {
    session_id: String,
}

struct ActivePromptGuard {
    session_key: String,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
}

impl ActivePromptGuard {
    fn new(active_prompts: Arc<StdMutex<HashSet<String>>>, session_key: String) -> Self {
        Self {
            session_key,
            active_prompts,
        }
    }
}

impl Drop for ActivePromptGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active_prompts.lock() {
            active.remove(&self.session_key);
        }
    }
}

pub struct AcpPromptRequest {
    pub session_key: String,
    pub client: AcpClientConfig,
    pub prompt: Vec<serde_json::Value>,
    pub workdir: PathBuf,
    pub env: HashMap<String, String>,
    pub event_sink: mpsc::Sender<NormalizedEvent>,
    pub cancel_rx: oneshot::Receiver<()>,
}

impl AcpSessionPool {
    pub fn new(agent: AcpAgentConfig) -> Self {
        Self {
            agent,
            ask_user_question: None,
            process: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    pub fn new_with_ask_user_question(
        agent: AcpAgentConfig,
        ask_user_question: Arc<AskUserQuestionBroker>,
    ) -> Self {
        Self {
            agent,
            ask_user_question: Some(ask_user_question),
            process: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
        }
    }

    pub fn with_idle_ttl(self, ttl: Duration) -> Self {
        // No-op for now: the current “warming” runtime keeps provider processes alive.
        //
        // Follow-up: implement TTL/LRU eviction for both provider processes and per-ctx session
        // ACP session mappings.
        let _ = ttl;
        self
    }

    pub fn spawn_reaper(self: &Arc<Self>) {
        // Intentionally a no-op: this runtime keeps provider processes warm for the daemon lifetime.
        //
        // Follow-up: add TTL/LRU session/process eviction to cap memory growth.
        let _ = self;
    }

    /// Best-effort warm-up: ensures the underlying ACP agent process is spawned and initialized.
    ///
    /// This does **not** create an ACP `sessionId`; it only pays the process spawn + `initialize`
    /// cost so the first real `session/new` is faster.
    pub async fn prewarm(
        &self,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        self.ensure_process(&client, workdir, env, event_sink)
            .await?;
        Ok(())
    }

    pub async fn prompt(&self, request: AcpPromptRequest) -> Result<()> {
        let AcpPromptRequest {
            session_key,
            client,
            prompt,
            workdir,
            env,
            event_sink,
            cancel_rx,
        } = request;

        self.ensure_process(&client, workdir.clone(), env.clone(), event_sink.clone())
            .await?;

        let process = {
            let proc_guard = self.process.lock().await;
            proc_guard
                .as_ref()
                .cloned()
                .context("no active ACP process")?
        };

        let active_guard = {
            let mut active = self
                .active_prompts
                .lock()
                .expect("active_prompts lock poisoned");
            if active.contains(&session_key) {
                None
            } else {
                active.insert(session_key.clone());
                Some(ActivePromptGuard::new(
                    Arc::clone(&self.active_prompts),
                    session_key.clone(),
                ))
            }
        };
        if active_guard.is_none() {
            anyhow::bail!("prompt already running for session {session_key}");
        }

        let acp_session_id = match self
            .ensure_context_session(&session_key, &process, &client, &workdir, &env, &event_sink)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                return Err(e);
            }
        };

        let result = process
            .prompt(
                &session_key,
                &acp_session_id,
                prompt,
                event_sink.clone(),
                cancel_rx,
            )
            .await;

        match result {
            Ok(()) => Ok(()),
            Err(e) => {
                // If the agent process crashed, drop it so the next turn can recreate it.
                let is_alive = process.is_alive().await.unwrap_or(false);
                if !is_alive {
                    let mut proc_guard = self.process.lock().await;
                    let same = proc_guard
                        .as_ref()
                        .map(|p| Arc::ptr_eq(p, &process))
                        .unwrap_or(false);
                    if same {
                        *proc_guard = None;
                    }
                    drop(proc_guard);
                    self.sessions.lock().await.clear();
                }
                Err(e)
            }
        }
    }

    pub async fn has_session(&self, session_key: &str) -> bool {
        let process = {
            let proc_guard = self.process.lock().await;
            proc_guard.as_ref().cloned()
        };
        let Some(process) = process else {
            return false;
        };
        let alive = process.is_alive().await.unwrap_or(false);
        if !alive {
            let mut proc_guard = self.process.lock().await;
            let same = proc_guard
                .as_ref()
                .map(|p| Arc::ptr_eq(p, &process))
                .unwrap_or(false);
            if same {
                *proc_guard = None;
            }
            drop(proc_guard);
            self.sessions.lock().await.clear();
            return false;
        }
        self.sessions.lock().await.contains_key(session_key)
    }

    pub async fn set_model(&self, session_key: String, model_id: String) -> Result<()> {
        let acp_session_id = {
            let map = self.sessions.lock().await;
            map.get(&session_key)
                .map(|s| s.acp_session_id.clone())
                .context("no active ACP session for this ctx session")?
        };
        let (tx, _rx) = mpsc::channel::<NormalizedEvent>(1);
        let process = {
            let proc_guard = self.process.lock().await;
            proc_guard
                .as_ref()
                .cloned()
                .context("no active ACP process")?
        };
        process.set_model(&acp_session_id, model_id, tx).await
    }

    pub async fn set_mode(&self, session_key: String, mode_id: String) -> Result<()> {
        let acp_session_id = {
            let map = self.sessions.lock().await;
            map.get(&session_key)
                .map(|s| s.acp_session_id.clone())
                .context("no active ACP session for this ctx session")?
        };
        let (tx, _rx) = mpsc::channel::<NormalizedEvent>(1);
        let process = {
            let proc_guard = self.process.lock().await;
            proc_guard
                .as_ref()
                .cloned()
                .context("no active ACP process")?
        };
        process.set_mode(&acp_session_id, mode_id, tx).await
    }

    pub async fn authenticate(
        &self,
        session_key: String,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        method_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        self.ensure_process(&client, workdir.clone(), env.clone(), event_sink.clone())
            .await?;
        let process = {
            let proc_guard = self.process.lock().await;
            proc_guard
                .as_ref()
                .cloned()
                .context("no active ACP process")?
        };
        let method_id = if let Some(method_id) = method_id {
            method_id
        } else {
            process
                .default_auth_method_id()
                .await
                .context("no auth method id provided and no authMethods advertised")?
        };
        process.authenticate(method_id).await?;
        let _ = self
            .ensure_context_session(&session_key, &process, &client, &workdir, &env, &event_sink)
            .await?;
        Ok(())
    }

    async fn ensure_process(
        &self,
        client: &AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let mut guard = self.process.lock().await;
        if guard.is_none() {
            let process_env = filter_process_env(env);
            let process = AcpProcess::spawn(
                self.agent.clone(),
                client.clone(),
                workdir,
                process_env,
                self.ask_user_question.as_ref().map(Arc::clone),
                event_sink,
            )
            .await
            .context("creating ACP process")?;
            *guard = Some(process);
        }
        Ok(())
    }

    async fn ensure_context_session(
        &self,
        session_key: &str,
        process: &Arc<AcpProcess>,
        client: &AcpClientConfig,
        workdir: &Path,
        env: &HashMap<String, String>,
        event_sink: &mpsc::Sender<NormalizedEvent>,
    ) -> Result<String> {
        {
            let map = self.sessions.lock().await;
            if let Some(s) = map.get(session_key) {
                return Ok(s.acp_session_id.clone());
            }
        }

        let resume_session_id = env.get("CTX_PROVIDER_SESSION_REF").cloned();
        let created = process
            .create_or_load_session(workdir, client, resume_session_id, event_sink.clone())
            .await?;

        let mut map = self.sessions.lock().await;
        map.insert(
            session_key.to_string(),
            AcpContextSession {
                acp_session_id: created.session_id.clone(),
            },
        );
        Ok(created.session_id)
    }

    pub async fn process_pid(&self) -> Option<u32> {
        let process = { self.process.lock().await.clone() };
        if let Some(process) = process {
            process.pid().await
        } else {
            None
        }
    }
}

struct AcpProcess {
    agent: AcpAgentConfig,
    child: Mutex<Child>,
    write_tx: mpsc::UnboundedSender<String>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
    supports_load: AtomicBool,
    supports_resume: AtomicBool,
    auth_methods: Mutex<Option<serde_json::Value>>,
    stderr_lines: Mutex<Vec<String>>,
    stdout_non_json: Mutex<Vec<String>>,
    ask_user_question: Option<Arc<AskUserQuestionBroker>>,
    router: SessionRouter,
}

#[derive(Default)]
struct SessionRouter {
    sessions: RwLock<HashMap<String, mpsc::UnboundedSender<AcpSessionNotification>>>,
}

enum AcpSessionNotification {
    Update(serde_json::Value),
    AskUserQuestion {
        req_id: u64,
        tool_call_id: String,
        input: serde_json::Value,
    },
    Raw(serde_json::Value),
    Shutdown {
        message: String,
    },
}

impl SessionRouter {
    async fn register(
        &self,
        session_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<AcpSessionNotification>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(session_id) {
            anyhow::bail!("session {session_id} already has an active prompt");
        }
        sessions.insert(session_id.to_string(), tx);
        Ok(rx)
    }

    async fn unregister(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
    }

    async fn send(&self, session_id: &str, msg: AcpSessionNotification) -> bool {
        let sender = {
            let sessions = self.sessions.read().await;
            sessions.get(session_id).cloned()
        };
        if let Some(sender) = sender {
            let _ = sender.send(msg);
            true
        } else {
            false
        }
    }

    async fn broadcast_shutdown(&self, message: String) {
        let sessions = self.sessions.read().await;
        for sender in sessions.values() {
            let _ = sender.send(AcpSessionNotification::Shutdown {
                message: message.clone(),
            });
        }
    }
}

impl AcpProcess {
    async fn pid(&self) -> Option<u32> {
        let child = self.child.lock().await;
        child.id()
    }

    async fn spawn(
        agent: AcpAgentConfig,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        ask_user_question: Option<Arc<AskUserQuestionBroker>>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<Arc<Self>> {
        let mut cmd = Command::new(&agent.command);
        cmd.args(&agent.args);
        cmd.current_dir(&workdir);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().with_context(|| {
            format!(
                "spawning ACP agent {} ({})",
                agent.provider_id, agent.command
            )
        })?;

        let stdin = child.stdin.take().context("capturing agent stdin")?;
        let stdout = child.stdout.take().context("capturing agent stdout")?;
        let stderr = child.stderr.take().context("capturing agent stderr")?;

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let stdout_reader = BufReader::new(stdout).lines();
        let stderr_reader = BufReader::new(stderr).lines();

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let _writer = tokio::spawn(async move {
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

        let process = Arc::new(Self {
            agent,
            child: Mutex::new(child),
            write_tx,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            supports_load: AtomicBool::new(false),
            supports_resume: AtomicBool::new(false),
            auth_methods: Mutex::new(None),
            stderr_lines: Mutex::new(Vec::new()),
            stdout_non_json: Mutex::new(Vec::new()),
            ask_user_question,
            router: SessionRouter::default(),
        });

        let stdout_process = Arc::clone(&process);
        tokio::spawn(async move {
            stdout_pump(stdout_process, stdout_reader).await;
        });

        let stderr_process = Arc::clone(&process);
        tokio::spawn(async move {
            stderr_pump(stderr_process, stderr_reader).await;
        });

        process.initialize(client, event_sink).await?;
        Ok(process)
    }

    async fn initialize(
        &self,
        client: AcpClientConfig,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let init_resp = self
            .send_request(
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
            )
            .await
            .context("waiting for initialize response")?;
        if let Some(err) = init_resp.get("error") {
            anyhow::bail!("ACP initialize error: {err}");
        }

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();
        let capabilities = init_resp
            .get("result")
            .and_then(|v| v.get("capabilities").or_else(|| v.get("agentCapabilities")));
        let supports_load = capabilities
            .and_then(|v| v.get("loadSession"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let supports_resume = capabilities
            .and_then(|v| {
                v.get("sessionCapabilities")
                    .or_else(|| v.get("session_capabilities"))
            })
            .and_then(|v| v.get("resume"))
            .map(|v| match v {
                serde_json::Value::Bool(value) => *value,
                serde_json::Value::Null => false,
                _ => true,
            })
            .unwrap_or(false);

        {
            let mut guard = self.auth_methods.lock().await;
            *guard = auth_methods.clone();
        }
        self.supports_load.store(supports_load, Ordering::SeqCst);
        self.supports_resume
            .store(supports_resume, Ordering::SeqCst);

        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Init,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "auth_methods": auth_methods.clone(),
                    "authMethods": auth_methods,
                    "supports_load": supports_load,
                    "supports_resume": supports_resume,
                }),
            })
            .await;

        Ok(())
    }

    async fn default_auth_method_id(&self) -> Option<String> {
        let methods = { self.auth_methods.lock().await.clone() }?;
        let list = methods.as_array()?;
        for m in list {
            if let Some(id) = m
                .get("methodId")
                .or_else(|| m.get("method_id"))
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())
            {
                if !id.trim().is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        None
    }

    async fn authenticate(&self, method_id: String) -> Result<()> {
        let resp = self
            .send_request("authenticate", json!({"methodId": method_id}))
            .await
            .context("waiting for authenticate response")?;

        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP authenticate error: {err}");
        }

        Ok(())
    }

    async fn create_or_load_session(
        &self,
        workdir: &Path,
        client: &AcpClientConfig,
        resume_session_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<CreatedAcpSession> {
        let cwd = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf())
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

        let mut resumed = false;
        let mut session_id: Option<String> = None;
        let mut modes: Option<serde_json::Value> = None;
        let mut models: Option<serde_json::Value> = None;

        if let Some(resume_id) = resume_session_id.clone() {
            if self.supports_resume.load(Ordering::SeqCst) {
                let resume_resp = self
                    .send_request(
                        "session/resume",
                        json!({"sessionId": resume_id, "cwd": cwd, "mcpServers": mcp_servers}),
                    )
                    .await
                    .context("waiting for session/resume response")?;
                if resume_resp.get("error").is_none() {
                    resumed = true;
                    session_id = Some(resume_id);
                    modes = resume_resp
                        .get("result")
                        .and_then(|v| v.get("modes"))
                        .cloned();
                    models = resume_resp
                        .get("result")
                        .and_then(|v| v.get("models"))
                        .cloned();
                } else if let Some(err) = resume_resp.get("error") {
                    if is_auth_required_error(err) {
                        let auth_methods = self.auth_methods.lock().await.clone();
                        let _ = event_sink
                            .send(NormalizedEvent {
                                event_type: SessionEventType::AuthRequired,
                                payload_json: json!({
                                    "kind": "auth_required",
                                    "provider": self.agent.provider_id,
                                    "message": "Provider requires authentication before resuming a session.",
                                    "auth_methods": auth_methods.clone(),
                                    "authMethods": auth_methods,
                                    "acp_error": err,
                                }),
                            })
                            .await;
                        anyhow::bail!("authentication required");
                    }
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Error,
                            payload_json: json!({
                                "provider": self.agent.provider_id,
                                "message": "session/resume failed; starting a new provider session",
                                "acp_error": resume_resp.get("error"),
                            }),
                        })
                        .await;
                }
            }
        }

        if session_id.is_none() {
            let new_resp = self
                .send_request(
                    "session/new",
                    json!({"cwd": cwd, "mcpServers": mcp_servers}),
                )
                .await
                .context("waiting for session/new response")?;
            if let Some(err) = new_resp.get("error") {
                if is_auth_required_error(err) {
                    let auth_methods = self.auth_methods.lock().await.clone();
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::AuthRequired,
                            payload_json: json!({
                                "kind": "auth_required",
                                "provider": self.agent.provider_id,
                                "message": "Provider requires authentication before starting a session.",
                                "auth_methods": auth_methods.clone(),
                                "authMethods": auth_methods,
                                "acp_error": err,
                            }),
                        })
                        .await;
                    anyhow::bail!("authentication required");
                }
                anyhow::bail!("ACP session/new error: {err}");
            }
            let id = new_resp
                .get("result")
                .and_then(|v| v.get("sessionId"))
                .and_then(|v| v.as_str())
                .context("missing sessionId in session/new response")?
                .to_string();
            session_id = Some(id.clone());
            modes = new_resp.get("result").and_then(|v| v.get("modes")).cloned();
            models = new_resp
                .get("result")
                .and_then(|v| v.get("models"))
                .cloned();
        }

        let session_id = session_id.context("missing ACP sessionId")?;
        let auth_methods = self.auth_methods.lock().await.clone();
        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Init,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "acp_session_id": session_id,
                    "resumed": resumed,
                    "supports_load": self.supports_load.load(Ordering::SeqCst),
                    "supports_resume": self.supports_resume.load(Ordering::SeqCst),
                    "modes": modes,
                    "models": models,
                    "auth_methods": auth_methods.clone(),
                    "authMethods": auth_methods,
                }),
            })
            .await;
        Ok(CreatedAcpSession { session_id })
    }

    async fn prompt(
        &self,
        context_session_id: &str,
        acp_session_id: &str,
        prompt: Vec<serde_json::Value>,
        event_sink: mpsc::Sender<NormalizedEvent>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        let mut state = StreamState::default();
        let mut session_rx = self.router.register(acp_session_id).await?;

        let mut prompt_rx = self
            .send_request_raw(
                "session/prompt",
                json!({"sessionId": acp_session_id, "prompt": prompt}),
            )
            .await?;

        let emit_raw_notifications = true;
        let mut ask_req_id: Option<u64> = None;
        let mut ask_tool_call_id: Option<String> = None;
        let mut ask_rx: Option<oneshot::Receiver<AskUserQuestionAnswer>> = None;

        let prompt_resp = loop {
            tokio::select! {
                answer = async {
                    if let Some(rx) = ask_rx.as_mut() {
                        rx.await
                    } else {
                        std::future::pending::<
                            Result<AskUserQuestionAnswer, tokio::sync::oneshot::error::RecvError>,
                        >()
                        .await
                    }
                } => {
                    let req_id = ask_req_id.take().context("missing AskUserQuestion request id")?;
                    let tool_call_id = ask_tool_call_id.take().unwrap_or_default();

                    let answer = match answer {
                        Ok(v) => v,
                        Err(_) => AskUserQuestionAnswer {
                            outcome: AskUserQuestionOutcome::Cancelled,
                            answers: Default::default(),
                        },
                    };

                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "outcome": answer.outcome.as_str(),
                            "answers": answer.answers,
                        }
                    });
                    let line = serde_json::to_string(&resp).context("serializing AskUserQuestion response")?;
                    let _ = self.write_tx.send(line);

                    if let Some(broker) = self.ask_user_question.as_ref() {
                        broker.abandon(context_session_id, &tool_call_id).await;
                    }

                    ask_rx = None;
                    continue;
                }
                _ = &mut cancel_rx => {
                    let _ = self.send_cancel_notification(acp_session_id);

                    if let Some(req_id) = ask_req_id.take() {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "outcome": AskUserQuestionOutcome::Cancelled.as_str(),
                                "answers": {},
                            }
                        });
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = self.write_tx.send(line);
                        }
                    }
                    if let (Some(tool_call_id), Some(broker)) =
                        (ask_tool_call_id.as_deref(), self.ask_user_question.as_ref())
                    {
                        broker.abandon(context_session_id, tool_call_id).await;
                    }

                    let _ = event_sink.send(NormalizedEvent {
                        event_type: SessionEventType::InterruptRequested,
                        payload_json: json!({"provider": self.agent.provider_id}),
                    }).await;
                    break json!({"result": { "stopReason": "cancelled" }});
                }
                msg = session_rx.recv() => {
                    match msg {
                        Some(AcpSessionNotification::Update(parsed)) => {
                            let events = normalize_session_update(&parsed, &mut state);
                            for ev in events {
                                let _ = event_sink.send(ev).await;
                            }
                        }
                        Some(AcpSessionNotification::AskUserQuestion { req_id, tool_call_id, input }) => {
                            if ask_rx.is_some() {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "nested AskUserQuestion not supported"}});
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            }

                            let Some(broker) = self.ask_user_question.as_ref() else {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32601, "message": "AskUserQuestion not supported by this client"}} );
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            };

                            ask_req_id = Some(req_id);
                            ask_tool_call_id = Some(tool_call_id.clone());
                            ask_rx = Some(broker.begin(context_session_id.to_string(), tool_call_id.clone()).await);

                            let _ = event_sink.send(NormalizedEvent {
                                event_type: SessionEventType::Notice,
                                payload_json: json!({
                                    "kind": "ask_user_question",
                                    "provider": self.agent.provider_id,
                                    "tool_call_id": tool_call_id,
                                    "input": input,
                                }),
                            }).await;
                        }
                        Some(AcpSessionNotification::Raw(parsed)) => {
                            if emit_raw_notifications {
                                let _ = event_sink.send(NormalizedEvent {
                                    event_type: SessionEventType::Init,
                                    payload_json: json!({"provider": self.agent.provider_id, "acp_event": parsed}),
                                }).await;
                            }
                        }
                        Some(AcpSessionNotification::Shutdown { message }) => {
                            anyhow::bail!("ACP process closed: {message}");
                        }
                        None => {
                            anyhow::bail!("ACP session channel closed");
                        }
                    }
                }
                resp = &mut prompt_rx => {
                    let resp = resp.context("awaiting ACP response")?;
                    loop {
                        match session_rx.try_recv() {
                            Ok(msg) => {
                                match msg {
                                    AcpSessionNotification::Update(parsed) => {
                                        let events = normalize_session_update(&parsed, &mut state);
                                        for ev in events {
                                            let _ = event_sink.send(ev).await;
                                        }
                                    }
                                    AcpSessionNotification::AskUserQuestion { req_id, tool_call_id, .. } => {
                                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "AskUserQuestion received after prompt completed"}});
                                        let line = serde_json::to_string(&resp)?;
                                        let _ = self.write_tx.send(line);
                                        if let Some(broker) = self.ask_user_question.as_ref() {
                                            broker.abandon(context_session_id, &tool_call_id).await;
                                        }
                                    }
                                    AcpSessionNotification::Raw(parsed) => {
                                        if emit_raw_notifications {
                                            let _ = event_sink.send(NormalizedEvent {
                                                event_type: SessionEventType::Init,
                                                payload_json: json!({"provider": self.agent.provider_id, "acp_event": parsed}),
                                            }).await;
                                        }
                                    }
                                    AcpSessionNotification::Shutdown { message } => {
                                        anyhow::bail!("ACP process closed: {message}");
                                    }
                                }
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                        }
                    }
                    break resp;
                }
            }
        };

        self.router.unregister(acp_session_id).await;
        state.saw_done = true;

        if let Some(err) = prompt_resp.get("error") {
            if is_auth_required_error(err) {
                let auth_methods = self.auth_methods.lock().await.clone();
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::AuthRequired,
                        payload_json: json!({
                            "kind": "auth_required",
                            "provider": self.agent.provider_id,
                            "message": "Provider requires authentication to continue.",
                            "auth_methods": auth_methods.clone(),
                            "authMethods": auth_methods,
                            "acp_error": err,
                        }),
                    })
                    .await;
            }
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
                    "acp_session_id": acp_session_id,
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
        &self,
        acp_session_id: &str,
        model_id: String,
        _event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let resp = self
            .send_request(
                "session/set_model",
                json!({"sessionId": acp_session_id, "modelId": model_id}),
            )
            .await
            .context("waiting for session/set_model response")?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP set_model error: {err}");
        }
        Ok(())
    }

    async fn set_mode(
        &self,
        acp_session_id: &str,
        mode_id: String,
        _event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let resp = self
            .send_request(
                "session/set_mode",
                json!({"sessionId": acp_session_id, "modeId": mode_id}),
            )
            .await
            .context("waiting for session/set_mode response")?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP set_mode error: {err}");
        }
        Ok(())
    }

    async fn send_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let rx = self.send_request_raw(method, params).await?;
        rx.await.context("awaiting ACP response")
    }

    async fn send_request_raw(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<oneshot::Receiver<serde_json::Value>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&msg).context("serializing ACP request")?;
        if self.write_tx.send(line).is_err() {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
            anyhow::bail!("ACP writer task unavailable");
        }
        Ok(rx)
    }

    fn send_cancel_notification(&self, acp_session_id: &str) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": acp_session_id }
        });
        let line = serde_json::to_string(&msg).context("serializing ACP cancel notification")?;
        let _ = self.write_tx.send(line);
        Ok(())
    }

    async fn is_alive(&self) -> Result<bool> {
        let mut child = self.child.lock().await;
        Ok(child.try_wait()?.is_none())
    }
}

impl Drop for AcpProcess {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

async fn stdout_pump(
    process: Arc<AcpProcess>,
    mut stdout_reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) {
    loop {
        match stdout_reader.next_line().await {
            Ok(Some(line)) => {
                let parsed = match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(v) => v,
                    Err(_) => {
                        push_capped(&process.stdout_non_json, line, 500).await;
                        continue;
                    }
                };

                if parsed.get("method").is_none() {
                    if let Some(id) = parsed.get("id").and_then(jsonrpc_id_u64) {
                        let tx = {
                            let mut pending = process.pending.lock().await;
                            pending.remove(&id)
                        };
                        if let Some(tx) = tx {
                            let _ = tx.send(parsed);
                        }
                        continue;
                    }
                }

                if parsed.get("method").and_then(|v| v.as_str())
                    == Some("session/request_permission")
                {
                    if let Ok(Some(line)) =
                        build_request_permission_response(&process.agent.provider_id, &parsed)
                    {
                        let _ = process.write_tx.send(line);
                    }
                    continue;
                }

                let is_ask_user_question = matches!(
                    parsed.get("method").and_then(|v| v.as_str()),
                    Some("_claude_code_acp/ask_user_question")
                        | Some("__claude_code_acp/ask_user_question")
                );
                if is_ask_user_question {
                    let req_id = match parsed.get("id").and_then(jsonrpc_id_u64) {
                        Some(id) => id,
                        None => continue,
                    };
                    let params = parsed.get("params").cloned().unwrap_or(json!({}));
                    let session_id = params
                        .get("sessionId")
                        .or_else(|| params.get("session_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let tool_call_id = params
                        .get("toolCallId")
                        .or_else(|| params.get("tool_call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let input = params.get("input").cloned().unwrap_or(json!({}));

                    if session_id.is_empty() || tool_call_id.is_empty() {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32602, "message": "missing sessionId or toolCallId"}} );
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                        continue;
                    }

                    let routed = process
                        .router
                        .send(
                            &session_id,
                            AcpSessionNotification::AskUserQuestion {
                                req_id,
                                tool_call_id,
                                input,
                            },
                        )
                        .await;
                    if !routed {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "AskUserQuestion received outside of an active session prompt"}} );
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                    }
                    continue;
                }

                if parsed.get("method").and_then(|v| v.as_str()) == Some("session/update") {
                    let session_id = parsed
                        .get("params")
                        .and_then(|v| v.get("sessionId").or_else(|| v.get("session_id")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !session_id.is_empty() {
                        let _ = process
                            .router
                            .send(&session_id, AcpSessionNotification::Update(parsed))
                            .await;
                    }
                    continue;
                }

                let session_id = parsed
                    .get("params")
                    .and_then(|v| v.get("sessionId").or_else(|| v.get("session_id")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !session_id.is_empty() {
                    let _ = process
                        .router
                        .send(&session_id, AcpSessionNotification::Raw(parsed))
                        .await;
                }
            }
            Ok(None) => {
                fail_pending(&process, "agent stdout closed").await;
                process
                    .router
                    .broadcast_shutdown("agent stdout closed".to_string())
                    .await;
                break;
            }
            Err(e) => {
                let message = format!("agent stdout read error: {e}");
                fail_pending(&process, &message).await;
                process.router.broadcast_shutdown(message).await;
                break;
            }
        }
    }
}

async fn stderr_pump(
    process: Arc<AcpProcess>,
    mut stderr_reader: tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
) {
    loop {
        match stderr_reader.next_line().await {
            Ok(Some(line)) => {
                push_capped(&process.stderr_lines, line, 500).await;
            }
            Ok(None) => break,
            Err(e) => {
                push_capped(&process.stderr_lines, e.to_string(), 500).await;
                break;
            }
        }
    }
}

async fn push_capped(store: &Mutex<Vec<String>>, line: String, cap: usize) {
    let mut guard = store.lock().await;
    guard.push(line);
    if guard.len() > cap {
        let start = guard.len().saturating_sub(cap / 2);
        *guard = guard[start..].to_vec();
    }
}

async fn fail_pending(process: &AcpProcess, message: &str) {
    let mut pending = process.pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(json!({"error": {"message": message}}));
    }
}

fn filter_process_env(env: HashMap<String, String>) -> HashMap<String, String> {
    // Per-session CTX_* vars must not be set on a shared provider process.
    // Session-specific values are passed via ACP `session/new` mcpServers env instead.
    env.into_iter()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "CTX_SESSION_ID" | "CTX_MCP_TOKEN" | "CTX_PROVIDER_SESSION_REF"
            )
        })
        .collect()
}

fn build_request_permission_response(
    provider_id: &str,
    msg: &serde_json::Value,
) -> Result<Option<String>> {
    let id = msg
        .get("id")
        .and_then(jsonrpc_id_u64)
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

fn extract_update_field(update: &Value, key: &str) -> Option<Value> {
    update
        .get(key)
        .cloned()
        .or_else(|| update.get("_meta").and_then(|meta| meta.get(key)).cloned())
}

fn add_update_meta_fields(
    payload: &mut Map<String, Value>,
    context_window: &Option<Value>,
    usage: &Option<Value>,
) {
    if let Some(context_window) = context_window.clone() {
        payload.insert("context_window".to_string(), context_window);
    }
    if let Some(usage) = usage.clone() {
        payload.insert("usage".to_string(), usage);
    }
}

fn normalize_session_update(
    msg: &serde_json::Value,
    state: &mut StreamState,
) -> Vec<NormalizedEvent> {
    let Some(params) = msg.get("params") else {
        return vec![];
    };
    let update = params.get("update").cloned().unwrap_or(json!({}));
    let context_window = extract_update_field(&update, "context_window");
    let usage = extract_update_field(&update, "usage");
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
                    let mut payload = Map::new();
                    payload.insert("content_fragment".to_string(), json!(text));
                    payload.insert("acp_update".to_string(), update.clone());
                    add_update_meta_fields(&mut payload, &context_window, &usage);
                    out.push(NormalizedEvent {
                        event_type: SessionEventType::AssistantChunk,
                        payload_json: Value::Object(payload),
                    });
                }
            }
            out
        }
        "agent_thought_chunk" => {
            if let Some(content) = update.get("content") {
                if let Some(text) = content_text(content) {
                    let mut payload = Map::new();
                    payload.insert("content_fragment".to_string(), json!(text));
                    payload.insert("acp_update".to_string(), update.clone());
                    add_update_meta_fields(&mut payload, &context_window, &usage);
                    return vec![NormalizedEvent {
                        event_type: SessionEventType::ThoughtChunk,
                        payload_json: Value::Object(payload),
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
                        let mut payload = Map::new();
                        payload.insert("content_fragment".to_string(), json!(text));
                        payload.insert("acp_update".to_string(), update.clone());
                        add_update_meta_fields(&mut payload, &context_window, &usage);
                        out.push(NormalizedEvent {
                            event_type: SessionEventType::AssistantChunk,
                            payload_json: Value::Object(payload),
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
            let mut payload = Map::new();
            payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::ToolCall,
                payload_json: Value::Object(payload),
            }]
        }
        "tool_call_update" => {
            let tool_call_id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let mut payload = Map::new();
            payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            let mut out = vec![NormalizedEvent {
                event_type: SessionEventType::ToolCallUpdate,
                payload_json: Value::Object(payload),
            }];
            if matches!(status, "completed" | "failed") {
                let mut payload = Map::new();
                payload.insert("tool_call_id".to_string(), json!(tool_call_id));
                payload.insert("acp_update".to_string(), update.clone());
                add_update_meta_fields(&mut payload, &context_window, &usage);
                out.push(NormalizedEvent {
                    event_type: SessionEventType::ToolResult,
                    payload_json: Value::Object(payload),
                });
            }
            out
        }
        "plan" => {
            let mut payload = Map::new();
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::Plan,
                payload_json: Value::Object(payload),
            }]
        }
        "available_commands_update" => {
            let mut payload = Map::new();
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::Notice,
                payload_json: Value::Object(payload),
            }]
        }
        "error" => {
            let mut payload = Map::new();
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            vec![NormalizedEvent {
                event_type: SessionEventType::Error,
                payload_json: Value::Object(payload),
            }]
        }
        _ => vec![],
    }
}

fn jsonrpc_id_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
}

#[derive(Debug, Clone)]
pub struct AcpProviderOptionsProbe {
    pub supports_load: bool,
    pub supports_resume: bool,
    pub auth_methods: Option<serde_json::Value>,
    pub modes: Option<serde_json::Value>,
    pub models: Option<serde_json::Value>,
    pub auth_required: bool,
    pub acp_error: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct AcpProviderVerifyProbe {
    /// One of: `ok`, `auth_required`, `network_error`, `error`.
    pub status: String,
    pub auth_required: bool,
    pub auth_methods: Option<serde_json::Value>,
    pub acp_error: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct AcpProviderAuthenticateProbe {
    /// One of: `ok`, `auth_required`, `error`.
    pub status: String,
    pub auth_required: bool,
    pub auth_methods: Option<serde_json::Value>,
    pub acp_error: Option<serde_json::Value>,
}

fn default_auth_method_id(methods: &serde_json::Value) -> Option<String> {
    let list = methods.as_array()?;
    for m in list {
        if let Some(id) = m
            .get("methodId")
            .or_else(|| m.get("method_id"))
            .or_else(|| m.get("id"))
            .and_then(|v| v.as_str())
        {
            if !id.trim().is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Best-effort probe for ACP providers that only expose model/mode lists on `session/new`.
///
/// This intentionally does **not** prompt; it runs `initialize` + `session/new`, extracts
/// `modes/models` from the response, then terminates the child process.
pub async fn probe_provider_options(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<AcpProviderOptionsProbe> {
    let probe_timeout = if agent.provider_id == "gemini" {
        Duration::from_secs(60)
    } else {
        Duration::from_secs(20)
    };

    timeout(probe_timeout, async move {
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

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut next_id: u64 = 1;

        let init_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
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
        )
        .await?;
        if let Some(err) = init_resp.get("error") {
            anyhow::bail!("ACP initialize error: {err}");
        }

        let capabilities = init_resp
            .get("result")
            .and_then(|v| v.get("capabilities").or_else(|| v.get("agentCapabilities")));
        let supports_load = capabilities
            .and_then(|v| v.get("loadSession"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let supports_resume = capabilities
            .and_then(|v| {
                v.get("sessionCapabilities")
                    .or_else(|| v.get("session_capabilities"))
            })
            .and_then(|v| v.get("resume"))
            .map(|v| match v {
                serde_json::Value::Bool(value) => *value,
                serde_json::Value::Null => false,
                _ => true,
            })
            .unwrap_or(false);

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();

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

        let new_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )
        .await?;
        if let Some(err) = new_resp.get("error") {
            let auth_required = is_auth_required_error(err);
            let _ = child.kill().await;
            return Ok(AcpProviderOptionsProbe {
                supports_load,
                supports_resume,
                auth_methods,
                modes: None,
                models: None,
                auth_required,
                acp_error: Some(err.clone()),
            });
        }

        let modes = new_resp.get("result").and_then(|v| v.get("modes")).cloned();
        let models = new_resp.get("result").and_then(|v| v.get("models")).cloned();

        let _ = child.kill().await;

        Ok(AcpProviderOptionsProbe {
            supports_load,
            supports_resume,
            auth_methods,
            modes,
            models,
            auth_required: false,
            acp_error: None,
        })
    })
    .await
    .context("ACP probe timed out")?
}

fn is_network_error(err: &serde_json::Value) -> bool {
    let Some(obj) = err.as_object() else {
        return false;
    };

    let msg = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let data_str = obj
        .get("data")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    let hay = format!("{msg}\n{data_str}");
    [
        "econnrefused",
        "enotfound",
        "ehostunreach",
        "enetunreach",
        "etimedout",
        "timeout",
        "timed out",
        "tls",
        "ssl",
        "certificate",
        "handshake",
        "connection reset",
        "socket hang up",
        "network is unreachable",
        "no route to host",
        "failed to connect",
        "could not resolve",
        "name or service not known",
        "temporary failure in name resolution",
    ]
    .iter()
    .any(|needle| hay.contains(needle))
}

/// Best-effort connectivity check for ACP providers.
///
/// This intentionally **does** prompt with a tiny request so we can detect failures that only
/// surface on `session/prompt` (for example missing BYO API keys/endpoints).
pub async fn verify_provider_connection(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
) -> Result<AcpProviderVerifyProbe> {
    let probe_timeout = if agent.provider_id == "gemini" {
        Duration::from_secs(90)
    } else {
        Duration::from_secs(30)
    };

    timeout(probe_timeout, async move {
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

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut next_id: u64 = 1;

        let init_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
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
        )
        .await?;

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();

        if let Some(err) = init_resp.get("error") {
            let _ = child.kill().await;
            return Ok(AcpProviderVerifyProbe {
                status: "error".to_string(),
                auth_required: is_auth_required_error(err),
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

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

        let new_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )
        .await?;
        if let Some(err) = new_resp.get("error") {
            let _ = child.kill().await;
            let auth_required = is_auth_required_error(err);
            let status = if auth_required {
                "auth_required"
            } else if is_network_error(err) {
                "network_error"
            } else {
                "error"
            };
            return Ok(AcpProviderVerifyProbe {
                status: status.to_string(),
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let acp_session_id = new_resp
            .get("result")
            .and_then(|v| v.get("sessionId"))
            .and_then(|v| v.as_str())
            .context("missing sessionId in session/new response")?
            .to_string();

        let prompt_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/prompt",
            json!({
                "sessionId": acp_session_id,
                "prompt": [
                    {"type":"text","text":"Respond with exactly: OK"}
                ]
            }),
        )
        .await?;

        if let Some(err) = prompt_resp.get("error") {
            let _ = child.kill().await;
            let auth_required = is_auth_required_error(err);
            let status = if auth_required {
                "auth_required"
            } else if is_network_error(err) {
                "network_error"
            } else {
                "error"
            };
            return Ok(AcpProviderVerifyProbe {
                status: status.to_string(),
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let _ = child.kill().await;
        Ok(AcpProviderVerifyProbe {
            status: "ok".to_string(),
            auth_required: false,
            auth_methods,
            acp_error: None,
        })
    })
    .await
    .context("ACP verify timed out")?
}

/// Best-effort provider-level authentication for ACP providers.
///
/// Calls ACP `authenticate` and then attempts `session/new` to confirm auth state.
pub async fn authenticate_provider(
    agent: AcpAgentConfig,
    client: AcpClientConfig,
    workdir: PathBuf,
    env: HashMap<String, String>,
    method_id: Option<String>,
) -> Result<AcpProviderAuthenticateProbe> {
    let auth_timeout = Duration::from_secs(5 * 60);

    timeout(auth_timeout, async move {
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

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let mut stdout_reader = BufReader::new(stdout).lines();

        let mut next_id: u64 = 1;

        let init_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
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
        )
        .await?;

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();

        if let Some(err) = init_resp.get("error") {
            let _ = child.kill().await;
            return Ok(AcpProviderAuthenticateProbe {
                status: "error".to_string(),
                auth_required: is_auth_required_error(err),
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        let Some(method_id) = method_id
            .or_else(|| auth_methods.as_ref().and_then(default_auth_method_id))
        else {
            let _ = child.kill().await;
            anyhow::bail!("no authentication methods advertised by provider");
        };

        let auth_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "authenticate",
            json!({"methodId": method_id}),
        )
        .await?;
        if let Some(err) = auth_resp.get("error") {
            let _ = child.kill().await;
            let auth_required = is_auth_required_error(err);
            return Ok(AcpProviderAuthenticateProbe {
                status: if auth_required {
                    "auth_required".to_string()
                } else {
                    "error".to_string()
                },
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

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

        let new_resp = acp_probe_request(
            &mut stdin,
            &mut stdout_reader,
            &agent.provider_id,
            &mut next_id,
            "session/new",
            json!({"cwd": cwd, "mcpServers": mcp_servers}),
        )
        .await?;

        let _ = child.kill().await;

        if let Some(err) = new_resp.get("error") {
            let auth_required = is_auth_required_error(err);
            return Ok(AcpProviderAuthenticateProbe {
                status: if auth_required {
                    "auth_required".to_string()
                } else {
                    "error".to_string()
                },
                auth_required,
                auth_methods,
                acp_error: Some(err.clone()),
            });
        }

        Ok(AcpProviderAuthenticateProbe {
            status: "ok".to_string(),
            auth_required: false,
            auth_methods,
            acp_error: None,
        })
    })
    .await
    .context("ACP authenticate timed out")?
}

async fn acp_probe_request(
    stdin: &mut tokio::io::BufWriter<tokio::process::ChildStdin>,
    stdout_reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    provider_id: &str,
    next_id: &mut u64,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let id = *next_id;
    *next_id += 1;
    let msg = json!({"jsonrpc":"2.0","id": id, "method": method, "params": params});
    let line = serde_json::to_string(&msg).context("serializing ACP request")?;
    stdin
        .write_all(line.as_bytes())
        .await
        .context("writing ACP request")?;
    stdin.write_all(b"\n").await.ok();
    stdin.flush().await.ok();

    loop {
        let line = stdout_reader
            .next_line()
            .await
            .context("reading ACP response line")?;
        let Some(line) = line else {
            anyhow::bail!("ACP agent exited during probe");
        };
        let parsed: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(mid) = parsed.get("id").and_then(jsonrpc_id_u64) {
            if mid == id {
                return Ok(parsed);
            }
            continue;
        }

        if parsed.get("method").and_then(|v| v.as_str()) == Some("session/request_permission") {
            if let Some(resp) = build_request_permission_response(provider_id, &parsed)? {
                stdin.write_all(resp.as_bytes()).await.ok();
                stdin.write_all(b"\n").await.ok();
                stdin.flush().await.ok();
            }
            continue;
        }
    }
}

fn content_text(block: &serde_json::Value) -> Option<String> {
    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
        return block
            .get("text")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }
    None
}

fn is_auth_required_error(err: &serde_json::Value) -> bool {
    let Some(obj) = err.as_object() else {
        return false;
    };

    let code_matches = obj
        .get("code")
        .and_then(|v| v.as_i64())
        .is_some_and(|code| code == -32001 || code == 401);

    let message = obj
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let msg_matches = message.contains("authrequired")
        || message.contains("auth_required")
        || message.contains("authentication required")
        || message.contains("unauthorized");

    let data_str = obj
        .get("data")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let data_matches = data_str.contains("authrequired") || data_str.contains("auth_required");

    code_matches || msg_matches || data_matches
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
        assert!(matches!(
            events[0].event_type,
            SessionEventType::AssistantChunk
        ));
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
        assert!(matches!(
            ev2[0].event_type,
            SessionEventType::ToolCallUpdate
        ));
        assert!(matches!(ev2[1].event_type, SessionEventType::ToolResult));
    }

    #[test]
    fn normalizes_available_commands_update_as_notice() {
        let mut state = StreamState::default();
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [
                        { "name": "review", "description": "Review changes" }
                    ]
                }
            }
        });

        let events = normalize_session_update(&msg, &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].event_type, SessionEventType::Notice));
        assert_eq!(
            events[0]
                .payload_json
                .get("acp_update")
                .and_then(|v| v.get("sessionUpdate"))
                .and_then(|v| v.as_str()),
            Some("available_commands_update")
        );
        assert_eq!(state.assistant_buf, "");
    }

    #[test]
    fn surfaces_context_window_from_meta() {
        let mut state = StreamState::default();
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "sess_1",
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [],
                    "_meta": {
                        "context_window": {
                            "context_window_tokens": 1000,
                            "context_tokens_estimate": 200,
                            "remaining_fraction": 0.8
                        }
                    }
                }
            }
        });

        let events = normalize_session_update(&msg, &mut state);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]
                .payload_json
                .get("context_window")
                .and_then(|v| v.get("context_window_tokens"))
                .and_then(Value::as_i64),
            Some(1000)
        );
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

    #[test]
    fn parses_jsonrpc_id_as_u64() {
        assert_eq!(jsonrpc_id_u64(&json!(7)), Some(7));
        assert_eq!(jsonrpc_id_u64(&json!("7")), Some(7));
        assert_eq!(jsonrpc_id_u64(&json!(7_i64)), Some(7));
        assert_eq!(jsonrpc_id_u64(&json!(-1)), None);
        assert_eq!(jsonrpc_id_u64(&json!("not-a-number")), None);
    }
}
