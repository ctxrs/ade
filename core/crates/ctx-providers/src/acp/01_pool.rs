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
