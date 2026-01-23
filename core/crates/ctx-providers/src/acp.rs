use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
#[cfg(target_os = "windows")]
use std::os::windows::io::FromRawHandle;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex as StdMutex,
};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tokio::time::timeout;

use ctx_core::models::SessionEventType;
#[cfg(target_os = "windows")]
use windows_sys::Win32::Foundation::CloseHandle;
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
#[cfg(target_os = "windows")]
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
};

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
    pub meta: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct AcpClientConfig {
    pub client_name: String,
    pub client_title: String,
    pub client_version: String,
    pub client_capabilities: serde_json::Value,
    pub system_prompt_append: Option<String>,
    pub mcp_servers: Vec<AcpMcpServer>,
}

#[derive(Debug, Clone)]
pub struct AcpAgentConfig {
    pub provider_id: String,
    pub command: String,
    pub args: Vec<String>,
}

fn encode_toml_basic_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn apply_system_prompt_append_args(
    mut agent: AcpAgentConfig,
    client: &AcpClientConfig,
) -> AcpAgentConfig {
    if agent.provider_id != "codex" {
        return agent;
    }

    let Some(append) = client.system_prompt_append.as_deref() else {
        return agent;
    };
    let trimmed = append.trim();
    if trimmed.is_empty() {
        return agent;
    }

    // Codex ACP doesn't read ACP _meta systemPrompt, so pass the append via config overrides.
    agent.args.push("-c".to_string());
    agent.args.push(format!(
        "developer_instructions={}",
        encode_toml_basic_string(trimmed)
    ));
    agent
}

fn codex_auth_signature(env: &HashMap<String, String>) -> String {
    if let Some(path) = env.get("CTX_CODEX_AUTH_PATH") {
        return format!("auth:{path}");
    }
    if let Some(home) = env.get("CODEX_HOME") {
        return format!("home:{home}");
    }
    "default".to_string()
}

const ACP_MEMORY_MAX_FRACTION: f64 = 0.9;
const ACP_MEMORY_MIN_MB: u64 = 256;

#[cfg(target_os = "linux")]
const SYSTEMD_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub(crate) struct StreamState {
    pub(crate) assistant_buf: String,
    pub(crate) saw_assistant_complete: bool,
    pub(crate) saw_done: bool,
}

pub struct AcpSessionPool {
    agent: AcpAgentConfig,
    ask_user_question: Option<Arc<AskUserQuestionBroker>>,
    process: Mutex<Option<Arc<AcpProcess>>>,
    sessions: Mutex<HashMap<String, AcpContextSession>>,
    active_prompts: Arc<StdMutex<HashSet<String>>>,
    auth_signature: Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
struct AcpContextSession {
    acp_session_id: String,
    model_id: Option<String>,
    mode_id: Option<String>,
}

#[derive(Debug, Clone)]
struct PermissionOption {
    option_id: String,
    label: String,
    kind: String,
    description: Option<String>,
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
    pub model_id: Option<String>,
}

impl AcpSessionPool {
    pub fn new(agent: AcpAgentConfig) -> Self {
        Self {
            agent,
            ask_user_question: None,
            process: Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            active_prompts: Arc::new(StdMutex::new(HashSet::new())),
            auth_signature: Mutex::new(None),
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
            auth_signature: Mutex::new(None),
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

    pub async fn restart(&self, reason: &str) {
        let process = { self.process.lock().await.clone() };
        if let Some(process) = process {
            process
                .router
                .broadcast_shutdown(format!("provider restart: {reason}"))
                .await;
            process.shutdown().await;
        }
        let mut guard = self.process.lock().await;
        *guard = None;
        drop(guard);
        self.sessions.lock().await.clear();
        if let Ok(mut active) = self.active_prompts.lock() {
            active.clear();
        }
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
            model_id,
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

        if let Some(desired_model_id) = normalize_session_model_id(model_id.as_deref()) {
            let already_set = {
                let map = self.sessions.lock().await;
                map.get(&session_key)
                    .and_then(|s| s.model_id.as_deref())
                    .is_some_and(|m| m == desired_model_id)
            };
            if !already_set
                && process
                    .set_model(
                        &acp_session_id,
                        desired_model_id.clone(),
                        event_sink.clone(),
                    )
                    .await
                    .is_ok()
            {
                let mut map = self.sessions.lock().await;
                if let Some(entry) = map.get_mut(&session_key) {
                    entry.model_id = Some(desired_model_id);
                }
            }
        }

        if let Some(desired_mode_id) =
            normalize_session_mode_id(env.get("CTX_PROVIDER_MODE").map(|s| s.as_str()))
        {
            let already_set = {
                let map = self.sessions.lock().await;
                map.get(&session_key)
                    .and_then(|s| s.mode_id.as_deref())
                    .is_some_and(|m| m == desired_mode_id)
            };
            if !already_set
                && process
                    .set_mode(&acp_session_id, desired_mode_id.clone(), event_sink.clone())
                    .await
                    .is_ok()
            {
                let mut map = self.sessions.lock().await;
                if let Some(entry) = map.get_mut(&session_key) {
                    entry.mode_id = Some(desired_mode_id);
                }
            }
        }

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
        process
            .set_model(&acp_session_id, model_id.clone(), tx)
            .await?;
        let mut map = self.sessions.lock().await;
        if let Some(entry) = map.get_mut(&session_key) {
            entry.model_id = Some(model_id);
        }
        Ok(())
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
        process
            .set_mode(&acp_session_id, mode_id.clone(), tx)
            .await?;
        let mut map = self.sessions.lock().await;
        if let Some(entry) = map.get_mut(&session_key) {
            entry.mode_id = Some(mode_id);
        }
        Ok(())
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
        self.maybe_reset_for_auth(&env).await;
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
                model_id: None,
                mode_id: None,
            },
        );
        Ok(created.session_id)
    }

    async fn reset_for_auth(&self) {
        let mut guard = self.process.lock().await;
        if let Some(proc) = guard.take() {
            proc.shutdown().await;
        }
        let mut sessions = self.sessions.lock().await;
        sessions.clear();
        if let Ok(mut active) = self.active_prompts.lock() {
            active.clear();
        }
    }

    async fn maybe_reset_for_auth(&self, env: &HashMap<String, String>) {
        if self.agent.provider_id != "codex" {
            return;
        }
        let desired = codex_auth_signature(env);
        let mut guard = self.auth_signature.lock().await;
        let changed = guard.as_deref() != Some(desired.as_str());
        if changed {
            *guard = Some(desired);
        }
        drop(guard);
        if changed {
            self.reset_for_auth().await;
        }
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
    pid_override: AtomicU32,
    #[cfg(target_os = "windows")]
    job: Option<std::os::windows::io::OwnedHandle>,
    write_tx: mpsc::UnboundedSender<String>,
    log_path: Option<PathBuf>,
    log_tx: Option<mpsc::UnboundedSender<String>>,
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

struct SpawnedAcpChild {
    child: Child,
    pid_override: Option<u32>,
    #[cfg(target_os = "linux")]
    unit: Option<String>,
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
    RequestPermission {
        req_id: u64,
        tool_call_id: String,
        tool_call: serde_json::Value,
        options: Vec<serde_json::Value>,
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
        let pid = self.pid_override.load(Ordering::Relaxed);
        if pid != 0 {
            return Some(pid);
        }
        let child = self.child.lock().await;
        child.id()
    }

    async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }

    async fn spawn(
        agent: AcpAgentConfig,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        ask_user_question: Option<Arc<AskUserQuestionBroker>>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<Arc<Self>> {
        let (log_tx, log_path) = match acp_log_path(&env, &agent.provider_id) {
            Some(path) => match spawn_acp_log_writer(path.clone()).await {
                Some(tx) => (Some(tx), Some(path)),
                None => (None, None),
            },
            None => (None, None),
        };

        let agent = apply_system_prompt_append_args(agent, &client);

        let spawned = spawn_acp_child(&agent, &workdir, &env)
            .await
            .with_context(|| {
                format!(
                    "spawning ACP agent {} ({})",
                    agent.provider_id, agent.command
                )
            })?;
        let mut child = spawned.child;
        let pid_override = spawned.pid_override;
        #[cfg(target_os = "linux")]
        let unit = spawned.unit;

        #[cfg(target_os = "windows")]
        let job = match attach_acp_job(&agent.provider_id, child.id().unwrap_or(0)) {
            Ok(handle) => handle,
            Err(err) => {
                tracing::warn!(
                    provider_id = %agent.provider_id,
                    "failed to attach ACP process to job object: {err:#}"
                );
                None
            }
        };

        if let Some(tx) = log_tx.as_ref() {
            let pid = pid_override.or(child.id()).unwrap_or(0);
            let _ = tx.send(format!(
                "[meta] started provider={} pid={}",
                agent.provider_id, pid
            ));
        }

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
            pid_override: AtomicU32::new(pid_override.unwrap_or(0)),
            #[cfg(target_os = "windows")]
            job,
            write_tx,
            log_path,
            log_tx,
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

        #[cfg(target_os = "linux")]
        if let Some(unit) = unit {
            if pid_override.is_none() {
                let process = Arc::clone(&process);
                tokio::spawn(async move {
                    if let Some(pid) = wait_for_systemd_main_pid(&unit).await {
                        process.pid_override.store(pid, Ordering::Relaxed);
                    }
                });
            }
        }

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
                let mut server = json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "env": s.env.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                });
                if let Some(meta) = &s.meta {
                    if let Some(obj) = server.as_object_mut() {
                        obj.insert("_meta".to_string(), meta.clone());
                    }
                }
                server
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
            let mut new_payload = json!({"cwd": cwd, "mcpServers": mcp_servers});
            if let Some(append) = client.system_prompt_append.as_deref() {
                let trimmed = append.trim();
                if !trimmed.is_empty() {
                    if let Some(obj) = new_payload.as_object_mut() {
                        obj.insert(
                            "_meta".to_string(),
                            json!({"systemPrompt": {"append": trimmed}}),
                        );
                    }
                }
            }
            let new_resp = self
                .send_request("session/new", new_payload)
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
        let mut perm_req_id: Option<u64> = None;
        let mut perm_tool_call_id: Option<String> = None;
        let mut perm_broker_id: Option<String> = None;
        let mut perm_question: Option<String> = None;
        let mut perm_options: Vec<PermissionOption> = Vec::new();
        let mut perm_rx: Option<oneshot::Receiver<AskUserQuestionAnswer>> = None;

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
                perm_answer = async {
                    if let Some(rx) = perm_rx.as_mut() {
                        rx.await
                    } else {
                        std::future::pending::<
                            Result<AskUserQuestionAnswer, tokio::sync::oneshot::error::RecvError>,
                        >()
                        .await
                    }
                } => {
                    let req_id = perm_req_id.take().context("missing permission request id")?;
                    let tool_call_id = perm_tool_call_id.take().unwrap_or_default();
                    let broker_id = perm_broker_id.take().unwrap_or_else(|| tool_call_id.clone());
                    let question = perm_question.take().unwrap_or_default();
                    let options = std::mem::take(&mut perm_options);

                    let answer = match perm_answer {
                        Ok(v) => v,
                        Err(_) => AskUserQuestionAnswer {
                            outcome: AskUserQuestionOutcome::Cancelled,
                            answers: Default::default(),
                        },
                    };

                    let selected_label = answer
                        .answers
                        .get(&question)
                        .map(|s| s.as_str())
                        .or_else(|| answer.answers.values().next().map(|s| s.as_str()));

                    let option_id = match answer.outcome {
                        AskUserQuestionOutcome::Submitted => select_permission_option_id(
                            &options,
                            selected_label,
                            &["allow_once", "allow_always"],
                        ),
                        AskUserQuestionOutcome::Cancelled => select_permission_option_id(
                            &options,
                            None,
                            &["reject_once", "reject_always"],
                        ),
                    }
                    .unwrap_or_else(|| "reject".to_string());

                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "_meta": {
                                "context": {
                                    "provider": self.agent.provider_id,
                                    "autoApproved": false,
                                }
                            },
                            "outcome": {
                                "outcome": "selected",
                                "optionId": option_id
                            }
                        }
                    });
                    let line = serde_json::to_string(&resp).context("serializing permission response")?;
                    let _ = self.write_tx.send(line);

                    if let Some(broker) = self.ask_user_question.as_ref() {
                        broker.abandon(context_session_id, &broker_id).await;
                    }

                    perm_rx = None;
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

                    if let Some(req_id) = perm_req_id.take() {
                        let options = std::mem::take(&mut perm_options);
                        let option_id = select_permission_option_id(
                            &options,
                            None,
                            &["reject_once", "reject_always"],
                        )
                        .unwrap_or_else(|| "reject".to_string());
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "outcome": {
                                    "outcome": "selected",
                                    "optionId": option_id
                                }
                            }
                        });
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = self.write_tx.send(line);
                        }
                    }
                    if let (Some(broker_id), Some(broker)) =
                        (perm_broker_id.as_deref(), self.ask_user_question.as_ref())
                    {
                        broker.abandon(context_session_id, broker_id).await;
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
                        Some(AcpSessionNotification::RequestPermission { req_id, tool_call_id, tool_call, options }) => {
                            if perm_rx.is_some() {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "nested permission request not supported"}});
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            }

                            let Some(broker) = self.ask_user_question.as_ref() else {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32601, "message": "permission requests not supported by this client"}} );
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            };

                            let normalized = normalize_permission_options(&options);
                            if normalized.is_empty() {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32602, "message": "permission request missing options"}} );
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            }

                            let question = build_permission_question(&tool_call);
                            let prompt_options = normalized
                                .iter()
                                .map(|opt| {
                                    let mut obj = json!({
                                        "label": opt.label,
                                    });
                                    if let Some(desc) = opt.description.as_ref() {
                                        if let Some(map) = obj.as_object_mut() {
                                            map.insert("description".to_string(), json!(desc));
                                        }
                                    }
                                    obj
                                })
                                .collect::<Vec<_>>();
                            let input = json!({
                                "questions": [{
                                    "header": "Permission",
                                    "question": question,
                                    "options": prompt_options,
                                    "multiSelect": false,
                                }],
                                "tool_call": tool_call,
                                "request_type": "permission",
                            });
                            let broker_id = format!("permission:{}", tool_call_id);

                            perm_req_id = Some(req_id);
                            perm_tool_call_id = Some(tool_call_id.clone());
                            perm_broker_id = Some(broker_id.clone());
                            perm_question = Some(question);
                            perm_options = normalized;
                            perm_rx = Some(broker.begin(context_session_id.to_string(), broker_id.clone()).await);

                            let _ = event_sink.send(NormalizedEvent {
                                event_type: SessionEventType::Notice,
                                payload_json: json!({
                                    "kind": "ask_user_question",
                                    "subkind": "permission_request",
                                    "provider": self.agent.provider_id,
                                    "tool_call_id": broker_id,
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
                                    AcpSessionNotification::RequestPermission { req_id, tool_call_id, .. } => {
                                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "permission request received after prompt completed"}});
                                        let line = serde_json::to_string(&resp)?;
                                        let _ = self.write_tx.send(line);
                                        if let Some(broker) = self.ask_user_question.as_ref() {
                                            let broker_id = format!("permission:{}", tool_call_id);
                                            broker.abandon(context_session_id, &broker_id).await;
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

#[cfg(target_os = "linux")]
async fn spawn_acp_child(
    agent: &AcpAgentConfig,
    workdir: &Path,
    env: &HashMap<String, String>,
) -> Result<SpawnedAcpChild> {
    if systemd_run_supports_pid().await {
        let child = spawn_acp_direct(agent, workdir, env)?;
        if let Some(pid) = child.id() {
            if let Err(err) = attach_acp_scope(&agent.provider_id, pid).await {
                tracing::warn!(
                    provider_id = %agent.provider_id,
                    pid,
                    "failed to attach ACP process to systemd scope: {err:#}"
                );
            }
        }
        return Ok(SpawnedAcpChild {
            child,
            pid_override: None,
            unit: None,
        });
    }

    if !systemd_run_available().await {
        tracing::warn!(
            provider_id = %agent.provider_id,
            "systemd-run unavailable; spawning ACP without systemd scope isolation"
        );
        let child = spawn_acp_direct(agent, workdir, env)?;
        return Ok(SpawnedAcpChild {
            child,
            pid_override: None,
            unit: None,
        });
    }

    let unit = acp_scope_unit(
        &agent.provider_id,
        &Utc::now().format("%Y%m%d%H%M%S%3f").to_string(),
    );
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--user")
        .arg("--quiet")
        .arg("--pipe")
        .arg("--wait")
        .arg("--unit")
        .arg(&unit)
        .arg("--working-directory")
        .arg(workdir);
    if let Some(max_mb) = acp_memory_max_mb() {
        cmd.arg("--property").arg(format!("MemoryMax={}M", max_mb));
    }
    for (k, v) in std::env::vars() {
        cmd.arg("--setenv").arg(format!("{k}={v}"));
    }
    for (k, v) in env {
        cmd.arg("--setenv").arg(format!("{k}={v}"));
    }
    cmd.arg("--").arg(&agent.command).args(&agent.args);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd.spawn()?;
    let pid_override = systemd_unit_main_pid(&unit).await;
    Ok(SpawnedAcpChild {
        child,
        pid_override,
        unit: Some(unit),
    })
}

#[cfg(target_os = "linux")]
fn spawn_acp_direct(
    agent: &AcpAgentConfig,
    workdir: &Path,
    env: &HashMap<String, String>,
) -> Result<Child> {
    let mut cmd = Command::new(&agent.command);
    cmd.args(&agent.args);
    cmd.current_dir(workdir);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().map_err(Into::into)
}

#[cfg(not(target_os = "linux"))]
async fn spawn_acp_child(
    agent: &AcpAgentConfig,
    workdir: &Path,
    env: &HashMap<String, String>,
) -> Result<SpawnedAcpChild> {
    let mut cmd = Command::new(&agent.command);
    cmd.args(&agent.args);
    cmd.current_dir(workdir);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }

    #[cfg(target_os = "macos")]
    if let Some(max_bytes) = acp_memory_max_bytes_macos() {
        unsafe {
            cmd.pre_exec(move || {
                let limit = libc::rlimit {
                    rlim_cur: max_bytes as libc::rlim_t,
                    rlim_max: max_bytes as libc::rlim_t,
                };
                libc::setrlimit(libc::RLIMIT_AS, &limit);
                Ok(())
            });
        }
    }

    let child = cmd.spawn()?;
    Ok(SpawnedAcpChild {
        child,
        pid_override: None,
    })
}

#[cfg(target_os = "macos")]
fn acp_memory_max_bytes_macos() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = std::ffi::CString::new("hw.memsize").ok()?;
    let res = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if res != 0 || value == 0 {
        return None;
    }
    let mut max_bytes = ((value as f64) * ACP_MEMORY_MAX_FRACTION).round() as u64;
    let min_bytes = ACP_MEMORY_MIN_MB * 1024 * 1024;
    max_bytes = max_bytes.max(min_bytes).min(value);
    Some(max_bytes)
}

#[cfg(target_os = "linux")]
fn sanitize_unit_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(target_os = "linux")]
fn acp_scope_unit(provider_id: &str, suffix: &str) -> String {
    let name = sanitize_unit_component(provider_id);
    format!("ctx-acp-{}-{}", name, suffix)
}

#[cfg(target_os = "linux")]
fn acp_memory_max_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb_str = rest.split_whitespace().next()?;
            let kb: u64 = kb_str.parse().ok()?;
            let total_mb = kb / 1024;
            if total_mb == 0 {
                return None;
            }
            let mut max_mb = ((total_mb as f64) * ACP_MEMORY_MAX_FRACTION).round() as u64;
            max_mb = max_mb.max(ACP_MEMORY_MIN_MB).min(total_mb);
            return Some(max_mb);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn acp_memory_max_bytes_windows() -> Option<u64> {
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 || status.ullTotalPhys == 0 {
        return None;
    }
    let total = status.ullTotalPhys;
    let mut max_bytes = ((total as f64) * ACP_MEMORY_MAX_FRACTION).round() as u64;
    let min_bytes = ACP_MEMORY_MIN_MB * 1024 * 1024;
    max_bytes = max_bytes.max(min_bytes).min(total);
    Some(max_bytes)
}

#[cfg(target_os = "windows")]
fn attach_acp_job(
    provider_id: &str,
    pid: u32,
) -> Result<Option<std::os::windows::io::OwnedHandle>> {
    let Some(max_bytes) = acp_memory_max_bytes_windows() else {
        return Ok(None);
    };

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job == 0 {
        anyhow::bail!("CreateJobObjectW failed");
    }

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    info.ProcessMemoryLimit = max_bytes as usize;
    let set_ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set_ok == 0 {
        unsafe { CloseHandle(job) };
        anyhow::bail!("SetInformationJobObject failed");
    }

    let process = unsafe {
        OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if process == 0 {
        unsafe { CloseHandle(job) };
        anyhow::bail!("OpenProcess failed for pid {pid}");
    }

    let assign_ok = unsafe { AssignProcessToJobObject(job, process) };
    unsafe { CloseHandle(process) };
    if assign_ok == 0 {
        unsafe { CloseHandle(job) };
        anyhow::bail!("AssignProcessToJobObject failed");
    }

    let owned = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(job as *mut _) };
    tracing::debug!(
        provider_id = %provider_id,
        pid,
        max_bytes,
        "attached ACP process to job object"
    );
    Ok(Some(owned))
}

#[cfg(target_os = "linux")]
async fn run_command(cmd: &mut Command, label: &str) -> Result<()> {
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output())
        .await
        .context("command timed out")?
        .with_context(|| format!("running {label}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::bail!(
        "{label} failed ({}): {}{}",
        output.status,
        stderr,
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" ({stdout})")
        }
    );
}

#[cfg(target_os = "linux")]
async fn systemd_run_supports_pid() -> bool {
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--help");
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output()).await;
    let Ok(Ok(output)) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    help.contains("--pid")
}

#[cfg(target_os = "linux")]
async fn systemd_run_available() -> bool {
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--version");
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output()).await;
    let Ok(Ok(output)) = output else {
        return false;
    };
    output.status.success()
}

#[cfg(target_os = "linux")]
async fn systemd_unit_main_pid(unit: &str) -> Option<u32> {
    if let Some(pid) = systemd_unit_main_pid_once(unit).await {
        return Some(pid);
    }
    if unit.contains('.') {
        return None;
    }
    let service = format!("{unit}.service");
    if let Some(pid) = systemd_unit_main_pid_once(&service).await {
        return Some(pid);
    }
    let scope = format!("{unit}.scope");
    systemd_unit_main_pid_once(&scope).await
}

#[cfg(target_os = "linux")]
async fn systemd_unit_main_pid_once(unit: &str) -> Option<u32> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user")
        .arg("show")
        .arg(unit)
        .arg("-p")
        .arg("MainPID")
        .arg("--value");
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output()).await.ok()?;
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let parsed = value.trim().parse::<u32>().ok()?;
    if parsed == 0 {
        None
    } else {
        Some(parsed)
    }
}

#[cfg(target_os = "linux")]
async fn wait_for_systemd_main_pid(unit: &str) -> Option<u32> {
    for _ in 0..10 {
        if let Some(pid) = systemd_unit_main_pid(unit).await {
            return Some(pid);
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    None
}

#[cfg(target_os = "linux")]
async fn attach_acp_scope(provider_id: &str, pid: u32) -> Result<()> {
    if !systemd_run_supports_pid().await {
        anyhow::bail!("systemd-run lacks --pid; ACP scope isolation unavailable");
    }

    let unit = acp_scope_unit(provider_id, &pid.to_string());
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--user").arg("--scope").arg("--unit").arg(unit);
    if let Some(max_mb) = acp_memory_max_mb() {
        cmd.arg("--property").arg(format!("MemoryMax={}M", max_mb));
    }
    cmd.arg("--pid").arg(pid.to_string());
    run_command(&mut cmd, "systemd-run").await
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
                        log_acp_line(&process, "stdout", &line);
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
                    let tool_call = params.get("toolCall").cloned().unwrap_or(json!({}));
                    let tool_call_id = tool_call
                        .get("toolCallId")
                        .or_else(|| tool_call.get("tool_call_id"))
                        .or_else(|| params.get("toolCallId"))
                        .or_else(|| params.get("tool_call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let options = params
                        .get("options")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    if session_id.is_empty() || tool_call_id.is_empty() {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32602, "message": "missing sessionId or toolCallId"}});
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                        continue;
                    }

                    let routed = process
                        .router
                        .send(
                            &session_id,
                            AcpSessionNotification::RequestPermission {
                                req_id,
                                tool_call_id,
                                tool_call,
                                options,
                            },
                        )
                        .await;
                    if !routed {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "request_permission received outside of an active session prompt"}});
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
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
                let status_note = describe_exit_status(&process).await;
                log_acp_line(
                    &process,
                    "meta",
                    &format!("agent stdout closed (exit status: {status_note})"),
                );
                let message = shutdown_message(&process, "agent stdout closed");
                fail_pending(&process, &message).await;
                process.router.broadcast_shutdown(message).await;
                break;
            }
            Err(e) => {
                let status_note = describe_exit_status(&process).await;
                log_acp_line(
                    &process,
                    "meta",
                    &format!("agent stdout read error (exit status: {status_note}): {e}"),
                );
                let message = shutdown_message(&process, &format!("agent stdout read error: {e}"));
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
                log_acp_line(&process, "stderr", &line);
                push_capped(&process.stderr_lines, line, 500).await;
            }
            Ok(None) => break,
            Err(e) => {
                let message = format!("stderr read error: {e}");
                log_acp_line(&process, "stderr", &message);
                push_capped(&process.stderr_lines, message, 500).await;
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
                "CTX_SESSION_ID" | "CTX_PROVIDER_SESSION_REF" | "CTX_SYSTEM_PROMPT_APPEND"
            )
        })
        .collect()
}

fn acp_log_path(env: &HashMap<String, String>, provider_id: &str) -> Option<PathBuf> {
    let data_root = env.get("CTX_DATA_ROOT")?;
    let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
    Some(
        Path::new(data_root)
            .join("logs")
            .join("providers")
            .join(format!("acp-{}-{}.log", provider_id, timestamp)),
    )
}

async fn spawn_acp_log_writer(path: PathBuf) -> Option<mpsc::UnboundedSender<String>> {
    if let Some(parent) = path.parent() {
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return None;
        }
    }

    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .ok()?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut file = file;
        while let Some(line) = rx.recv().await {
            let redacted = redact_sensitive(&line);
            if file.write_all(redacted.as_bytes()).await.is_err() {
                break;
            }
            if !redacted.ends_with('\n') && file.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = file.flush().await;
        }
    });

    Some(tx)
}

fn redact_sensitive(input: &str) -> String {
    fn redact_after_marker(mut s: String, marker: &str) -> String {
        let redacted = "[REDACTED]";
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find(marker) {
            let marker_start = search_from + rel;
            let start = marker_start + marker.len();
            if start >= s.len() {
                break;
            }
            if s[start..].starts_with(redacted) {
                search_from = start + redacted.len();
                continue;
            }

            let mut end = s.len();
            for (i, ch) in s[start..].char_indices() {
                if ch.is_whitespace() || ch == '"' || ch == '\'' || ch == '&' {
                    end = start + i;
                    break;
                }
            }

            s.replace_range(start..end, redacted);
            search_from = start + redacted.len();
        }
        s
    }

    let mut out = input.to_string();
    out = redact_after_marker(out, "Bearer ");
    out = redact_after_marker(out, "bearer ");
    out = redact_after_marker(out, "Authorization: Bearer ");
    out = redact_after_marker(out, "authorization: Bearer ");
    out = redact_after_marker(out, "token=");
    out = redact_after_marker(out, "TOKEN=");
    out = redact_after_marker(out, "CTX_AUTH_TOKEN=");
    out = redact_after_marker(out, "ctxAuthToken\":\"");
    out = redact_after_marker(out, "ctx_auth_token\":\"");
    out
}

fn log_acp_line(process: &AcpProcess, prefix: &str, line: &str) {
    if let Some(tx) = process.log_tx.as_ref() {
        let _ = tx.send(format!("[{prefix}] {line}"));
    }
}

fn shutdown_message(process: &AcpProcess, base: &str) -> String {
    match process.log_path.as_ref() {
        Some(path) => format!("{base} (see {})", path.display()),
        None => base.to_string(),
    }
}

async fn describe_exit_status(process: &AcpProcess) -> String {
    let mut child = process.child.lock().await;
    match child.try_wait() {
        Ok(Some(status)) => format_exit_status(&status),
        Ok(None) => "still running".to_string(),
        Err(err) => format!("unavailable: {err}"),
    }
}

fn format_exit_status(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }
    #[cfg(unix)]
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    "unknown".to_string()
}

fn normalize_session_model_id(model_id: Option<&str>) -> Option<String> {
    let trimmed = model_id?.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_session_mode_id(mode_id: Option<&str>) -> Option<String> {
    let trimmed = mode_id?.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_permission_options(options: &[serde_json::Value]) -> Vec<PermissionOption> {
    options
        .iter()
        .filter_map(|opt| {
            let option_id = opt
                .get("optionId")
                .or_else(|| opt.get("option_id"))
                .or_else(|| opt.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if option_id.is_empty() {
                return None;
            }
            let label = opt
                .get("name")
                .or_else(|| opt.get("label"))
                .or_else(|| opt.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(option_id)
                .trim();
            let kind = opt
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let description = opt
                .get("description")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());
            Some(PermissionOption {
                option_id: option_id.to_string(),
                label: label.to_string(),
                kind,
                description,
            })
        })
        .collect()
}

fn build_permission_question(tool_call: &serde_json::Value) -> String {
    let title = tool_call
        .get("title")
        .or_else(|| tool_call.get("name"))
        .or_else(|| tool_call.get("toolName"))
        .and_then(|v| v.as_str())
        .unwrap_or("tool call")
        .trim();
    let kind = tool_call
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if kind.is_empty() {
        format!("Allow tool call: {}?", title)
    } else {
        format!("Allow tool call: {} ({})?", title, kind)
    }
}

fn select_permission_option_id(
    options: &[PermissionOption],
    selected_label: Option<&str>,
    preferred_kinds: &[&str],
) -> Option<String> {
    if let Some(label) = selected_label {
        if let Some(opt) = options.iter().find(|o| o.label == label) {
            return Some(opt.option_id.clone());
        }
    }
    for kind in preferred_kinds {
        if let Some(opt) = options.iter().find(|o| o.kind == *kind) {
            return Some(opt.option_id.clone());
        }
    }
    options.first().map(|opt| opt.option_id.clone())
}

pub(crate) fn build_request_permission_response(
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

pub(crate) fn normalize_session_update(
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
                .or_else(|| update.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .or_else(|| {
                    update
                        .pointer("/rawInput/call_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            update
                                .pointer("/raw_input/call_id")
                                .and_then(|v| v.as_str())
                        })
                        .map(|v| v.to_string())
                });
            let mut payload = Map::new();
            if let Some(tool_call_id) = tool_call_id {
                payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            }
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
                .or_else(|| update.get("tool_call_id"))
                .and_then(|v| v.as_str())
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
                .map(|v| v.to_string())
                .or_else(|| {
                    update
                        .pointer("/rawInput/call_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            update
                                .pointer("/raw_input/call_id")
                                .and_then(|v| v.as_str())
                        })
                        .map(|v| v.to_string())
                });
            let status = update.get("status").and_then(|v| v.as_str()).unwrap_or("");
            let mut payload = Map::new();
            if let Some(ref tool_call_id) = tool_call_id {
                payload.insert("tool_call_id".to_string(), json!(tool_call_id));
            }
            payload.insert("acp_update".to_string(), update.clone());
            add_update_meta_fields(&mut payload, &context_window, &usage);
            let mut out = vec![NormalizedEvent {
                event_type: SessionEventType::ToolCallUpdate,
                payload_json: Value::Object(payload),
            }];
            if matches!(status, "completed" | "failed") {
                let mut payload = Map::new();
                if let Some(ref tool_call_id) = tool_call_id {
                    payload.insert("tool_call_id".to_string(), json!(tool_call_id));
                }
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

/// Offline transcript replay helpers used by contract tests.
///
/// These helpers intentionally avoid spawning any real provider binaries; they replay captured (or
/// synthetic) ACP `session/update` notifications plus the final `session/prompt` response through
/// the same normalization logic used at runtime.
pub mod transcript {
    use std::path::Path;

    use anyhow::{Context, Result};
    use serde_json::{json, Value};

    use ctx_core::models::SessionEventType;

    use super::StreamState;
    use super::{
        is_auth_required_error, jsonrpc_id_u64, normalize_session_update, NormalizedEvent,
    };

    pub const TRANSCRIPT_VERSION: u32 = 1;

    #[derive(Debug, Clone, serde::Deserialize)]
    pub struct AcpTranscript {
        pub version: u32,
        pub provider: String,
        pub case: String,
        #[serde(default)]
        pub description: Option<String>,
        pub events: Vec<AcpTranscriptEvent>,
    }

    #[derive(Debug, Clone, serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum AcpTranscriptEvent {
        SessionUpdate {
            msg: Value,
        },
        PromptResponse {
            msg: Value,
        },
        #[serde(rename = "note")]
        Note {
            text: String,
        },
    }

    pub fn load_transcript(path: impl AsRef<Path>) -> Result<AcpTranscript> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading transcript {}", path.display()))?;
        let transcript: AcpTranscript =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(transcript)
    }

    fn extract_session_id(msg: &Value) -> Option<String> {
        msg.get("params")
            .and_then(|p| p.get("sessionId").or_else(|| p.get("session_id")))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn extract_stop_reason(prompt_response: &Value) -> String {
        prompt_response
            .get("result")
            .and_then(|v| v.get("stopReason"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    }

    pub fn replay_transcript(transcript: &AcpTranscript) -> Result<Vec<NormalizedEvent>> {
        anyhow::ensure!(
            transcript.version == TRANSCRIPT_VERSION,
            "unsupported transcript version {}; expected {}",
            transcript.version,
            TRANSCRIPT_VERSION
        );

        let mut state = StreamState::default();
        let mut out: Vec<NormalizedEvent> = Vec::new();
        let mut prompt_response: Option<Value> = None;
        let mut session_id: Option<String> = None;

        for ev in &transcript.events {
            match ev {
                AcpTranscriptEvent::SessionUpdate { msg } => {
                    if session_id.is_none() {
                        session_id = extract_session_id(msg);
                    }
                    out.extend(normalize_session_update(msg, &mut state));
                }
                AcpTranscriptEvent::PromptResponse { msg } => {
                    if prompt_response.is_some() {
                        anyhow::bail!("transcript contains more than one prompt_response");
                    }
                    prompt_response = Some(msg.clone());
                }
                AcpTranscriptEvent::Note { .. } => {}
            }
        }

        let prompt_response = prompt_response.context("transcript missing prompt_response")?;

        if let Some(err) = prompt_response.get("error") {
            if is_auth_required_error(err) {
                out.push(NormalizedEvent {
                    event_type: SessionEventType::AuthRequired,
                    payload_json: json!({
                        "kind": "auth_required",
                        "provider": transcript.provider,
                        "message": "Provider requires authentication to continue.",
                        "acp_error": err,
                    }),
                });
            }
            out.push(NormalizedEvent {
                event_type: SessionEventType::Error,
                payload_json: json!({
                    "provider": transcript.provider,
                    "acp_error": err,
                }),
            });
        }

        if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
            state.saw_assistant_complete = true;
            out.push(NormalizedEvent {
                event_type: SessionEventType::AssistantComplete,
                payload_json: json!({ "full_content": state.assistant_buf }),
            });
        }

        // Best-effort extraction: if the response has an `id`, preserve it for debugging.
        let response_id = prompt_response.get("id").and_then(jsonrpc_id_u64);
        out.push(NormalizedEvent {
            event_type: SessionEventType::Done,
            payload_json: json!({
                "provider": transcript.provider,
                "acp_session_id": session_id,
                "status": if prompt_response.get("error").is_some() { "error" } else { "success" },
                "stop_reason": extract_stop_reason(&prompt_response),
                "acp_response_id": response_id,
                "transcript_case": transcript.case,
            }),
        });

        Ok(out)
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
