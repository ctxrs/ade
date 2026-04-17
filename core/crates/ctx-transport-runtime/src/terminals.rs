use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, RootCertStore, SignatureScheme};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, connect_async_tls_with_config, Connector};

use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, TerminalStatus};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub const DEFAULT_OUTPUT_TAIL_BYTES: usize = 20 * 1024;
const TERMINAL_PING_INTERVAL: Duration = Duration::from_secs(25);
const TERMINAL_RECONNECT_BASE_MS: u64 = 500;
const TERMINAL_RECONNECT_MAX_MS: u64 = 10_000;
const TERMINAL_REAPER_INTERVAL: Duration = Duration::from_secs(60);

fn lock_or_recover<'a, T>(mutex: &'a Mutex<T>, name: &str) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

fn terminal_idle_timeout() -> Option<Duration> {
    let raw = std::env::var("CTX_TERMINAL_IDLE_TIMEOUT_SECS").ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let secs = trimmed.parse::<u64>().ok()?;
    if secs == 0 {
        return None;
    }
    Some(Duration::from_secs(secs))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalClientMessage {
    Resize { cols: u16, rows: u16 },
    Input { data: String },
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TerminalServerMessage {
    Status {
        status: TerminalStatus,
        exit_code: Option<i32>,
    },
    Pong,
}

#[derive(Debug, Clone)]
pub struct TerminalCreateRequest {
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub cwd: PathBuf,
    pub shell: String,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub env: HashMap<String, String>,
    pub native_container: Option<NativeContainerTerminalSpec>,
    pub shared_vm_container: Option<SharedVmContainerTerminalSpec>,
}

#[derive(Debug, Clone)]
pub struct NativeContainerTerminalSpec {
    pub cli_bin: PathBuf,
    pub cli_env: HashMap<String, String>,
    pub container_name: String,
    pub workdir: String,
    pub user: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SharedVmContainerTerminalSpec {
    pub helper_path: PathBuf,
    pub data_root: PathBuf,
    pub workspace_id: WorkspaceId,
    pub workdir: String,
    pub user: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TerminalStatusEvent {
    pub status: TerminalStatus,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct RemoteTerminalRequest {
    pub terminal_id: TerminalId,
    pub gateway_url: String,
    pub worker_id: String,
    pub token: Option<String>,
    pub gateway_ca_pem: Option<String>,
}

#[derive(Debug, Clone)]
struct TerminalRuntime {
    status: TerminalStatus,
    exit_code: Option<i32>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_activity: chrono::DateTime<chrono::Utc>,
    connected_clients: usize,
}

enum RemoteTerminalOutgoing {
    Binary(Vec<u8>),
    Text(String),
    Close,
}

enum TerminalBackend {
    Local {
        input_tx: mpsc::UnboundedSender<Vec<u8>>,
        master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
        child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    },
    Remote {
        outbound_tx: mpsc::UnboundedSender<RemoteTerminalOutgoing>,
    },
}

pub struct TerminalSessionHandle {
    info: TerminalSession,
    container_backed: bool,
    runtime: Arc<Mutex<TerminalRuntime>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    status_tx: broadcast::Sender<TerminalStatusEvent>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    backend: TerminalBackend,
}

#[derive(Debug, Clone, Serialize)]
pub struct TerminalManagerStats {
    pub session_count: usize,
    pub output_buffer_bytes: usize,
    pub max_output_buffer_bytes: usize,
    pub connected_clients: usize,
}

impl TerminalSessionHandle {
    pub fn snapshot(&self) -> TerminalSession {
        let runtime = lock_or_recover(self.runtime.as_ref(), "terminal runtime");
        TerminalSession {
            status: runtime.status.clone(),
            exit_code: runtime.exit_code,
            updated_at: runtime.updated_at,
            ..self.info.clone()
        }
    }

    fn touch_activity(&self) {
        let mut runtime = lock_or_recover(self.runtime.as_ref(), "terminal runtime");
        runtime.last_activity = Utc::now();
        runtime.updated_at = runtime.last_activity;
    }

    pub fn mark_client_connected(&self) {
        let mut runtime = lock_or_recover(self.runtime.as_ref(), "terminal runtime");
        runtime.connected_clients = runtime.connected_clients.saturating_add(1);
        runtime.last_activity = Utc::now();
        runtime.updated_at = runtime.last_activity;
    }

    pub fn mark_client_disconnected(&self) {
        let mut runtime = lock_or_recover(self.runtime.as_ref(), "terminal runtime");
        runtime.connected_clients = runtime.connected_clients.saturating_sub(1);
        runtime.updated_at = Utc::now();
    }

    pub fn output_receiver(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    pub fn status_receiver(&self) -> broadcast::Receiver<TerminalStatusEvent> {
        self.status_tx.subscribe()
    }

    pub fn output_snapshot(&self) -> Vec<u8> {
        let buffer = lock_or_recover(self.output_buffer.as_ref(), "terminal buffer");
        buffer.iter().copied().collect()
    }

    pub fn output_snapshot_tail(&self, tail: usize) -> Vec<u8> {
        let buffer = lock_or_recover(self.output_buffer.as_ref(), "terminal buffer");
        let len = buffer.len();
        if len == 0 || tail == 0 {
            return Vec::new();
        }
        let tail = tail.min(len);
        buffer.iter().skip(len - tail).copied().collect()
    }

    pub fn send_input(&self, data: Vec<u8>) {
        self.touch_activity();
        match &self.backend {
            TerminalBackend::Local { input_tx, .. } => {
                let _ = input_tx.send(data);
            }
            TerminalBackend::Remote { outbound_tx } => {
                let _ = outbound_tx.send(RemoteTerminalOutgoing::Binary(data));
            }
        }
    }

    #[doc(hidden)]
    pub fn test_handle_with_output(output: &[u8]) -> Arc<Self> {
        let now = Utc::now();
        let (output_tx, _) = broadcast::channel(16);
        let (status_tx, _) = broadcast::channel(16);
        let (_outbound_tx, outbound_rx) = mpsc::unbounded_channel();
        let mut output_buffer = VecDeque::with_capacity(output.len());
        output_buffer.extend(output.iter().copied());
        Arc::new(Self {
            info: TerminalSession {
                id: TerminalId::new(),
                workspace_id: WorkspaceId::new(),
                task_id: None,
                session_id: None,
                worktree_id: None,
                cwd: "/tmp".to_string(),
                shell: "/bin/sh".to_string(),
                title: "test-terminal".to_string(),
                status: TerminalStatus::Running,
                exit_code: None,
                created_at: now,
                updated_at: now,
            },
            container_backed: false,
            runtime: Arc::new(Mutex::new(TerminalRuntime {
                status: TerminalStatus::Running,
                exit_code: None,
                updated_at: now,
                last_activity: now,
                connected_clients: 0,
            })),
            output_tx,
            status_tx,
            output_buffer: Arc::new(Mutex::new(output_buffer)),
            backend: TerminalBackend::Remote {
                outbound_tx: {
                    drop(outbound_rx);
                    _outbound_tx
                },
            },
        })
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.touch_activity();
        match &self.backend {
            TerminalBackend::Local { master, .. } => {
                let master = lock_or_recover(master.as_ref(), "terminal master");
                master
                    .resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .context("resize pty")?;
            }
            TerminalBackend::Remote { outbound_tx } => {
                let payload = serde_json::to_string(&TerminalClientMessage::Resize { cols, rows })
                    .unwrap_or_else(|_| "{\"type\":\"resize\"}".to_string());
                let _ = outbound_tx.send(RemoteTerminalOutgoing::Text(payload));
            }
        }
        Ok(())
    }

    pub fn kill(&self) -> Result<()> {
        match &self.backend {
            TerminalBackend::Local { child, .. } => {
                let mut child = lock_or_recover(child.as_ref(), "terminal child");
                child.kill().context("kill terminal")?;
            }
            TerminalBackend::Remote { outbound_tx } => {
                let _ = outbound_tx.send(RemoteTerminalOutgoing::Close);
            }
        }
        Ok(())
    }

    pub fn mark_exited(&self, exit_code: Option<i32>) {
        let mut runtime = lock_or_recover(self.runtime.as_ref(), "terminal runtime");
        runtime.status = TerminalStatus::Exited;
        runtime.exit_code = exit_code;
        runtime.updated_at = Utc::now();
        runtime.last_activity = runtime.updated_at;
        let _ = self.status_tx.send(TerminalStatusEvent {
            status: TerminalStatus::Exited,
            exit_code,
        });
    }

    fn is_running(&self) -> bool {
        let runtime = lock_or_recover(self.runtime.as_ref(), "terminal runtime");
        matches!(runtime.status, TerminalStatus::Running)
    }
}

#[derive(Default)]
pub struct TerminalManager {
    sessions: tokio::sync::Mutex<HashMap<TerminalId, Arc<TerminalSessionHandle>>>,
}

impl TerminalManager {
    pub async fn list(&self, workspace_id: WorkspaceId) -> Vec<TerminalSession> {
        let sessions = self.sessions.lock().await;
        sessions
            .values()
            .filter(|sess| sess.info.workspace_id == workspace_id)
            .map(|sess| sess.snapshot())
            .collect()
    }

    pub async fn stats(&self) -> TerminalManagerStats {
        let sessions = self.sessions.lock().await;
        let mut output_buffer_bytes = 0;
        let mut max_output_buffer_bytes = 0;
        let mut connected_clients = 0;
        for handle in sessions.values() {
            let buffer_len = {
                let buffer = lock_or_recover(handle.output_buffer.as_ref(), "terminal buffer");
                buffer.len()
            };
            output_buffer_bytes += buffer_len;
            if buffer_len > max_output_buffer_bytes {
                max_output_buffer_bytes = buffer_len;
            }
            let runtime = lock_or_recover(handle.runtime.as_ref(), "terminal runtime");
            connected_clients += runtime.connected_clients;
        }
        TerminalManagerStats {
            session_count: sessions.len(),
            output_buffer_bytes,
            max_output_buffer_bytes,
            connected_clients,
        }
    }

    pub async fn start_reaper(self: Arc<Self>) {
        let Some(idle_for) = terminal_idle_timeout() else {
            return;
        };
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(TERMINAL_REAPER_INTERVAL);
            loop {
                interval.tick().await;
                if let Err(err) = self.reap_idle(idle_for).await {
                    tracing::warn!("terminal reap failed: {err:#}");
                }
            }
        });
    }

    pub async fn has_running(&self) -> bool {
        let sessions = self.sessions.lock().await;
        sessions.values().any(|sess| sess.is_running())
    }

    pub async fn has_running_container_backed(&self) -> bool {
        let sessions = self.sessions.lock().await;
        sessions
            .values()
            .any(|sess| sess.container_backed && sess.is_running())
    }

    pub async fn get(&self, id: TerminalId) -> Option<Arc<TerminalSessionHandle>> {
        let sessions = self.sessions.lock().await;
        sessions.get(&id).cloned()
    }

    pub async fn create(&self, req: TerminalCreateRequest) -> Result<Arc<TerminalSessionHandle>> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: req.rows.unwrap_or(DEFAULT_ROWS),
                cols: req.cols.unwrap_or(DEFAULT_COLS),
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("open pty")?;

        let cmd = if let Some(native_container) = &req.native_container {
            let mut cmd = CommandBuilder::new(native_container.cli_bin.clone());
            for (key, value) in &native_container.cli_env {
                cmd.env(key, value);
            }

            // Container env.
            cmd.arg("exec");
            cmd.arg("-i");
            cmd.arg("-t");
            cmd.arg("--workdir");
            cmd.arg(native_container.workdir.clone());
            cmd.arg("--env");
            cmd.arg("TERM=xterm-256color");
            if let Some(user) = native_container.user.as_ref() {
                cmd.arg("--user");
                cmd.arg(user.clone());
            }
            for (key, value) in &req.env {
                cmd.arg("--env");
                cmd.arg(format!("{key}={value}"));
            }
            cmd.arg(native_container.container_name.clone());
            cmd.arg(req.shell.clone());
            cmd
        } else if let Some(shared_vm_container) = &req.shared_vm_container {
            let mut cmd = CommandBuilder::new(shared_vm_container.helper_path.clone());
            cmd.arg("shared-vm-exec");
            cmd.arg("--data-root");
            cmd.arg(shared_vm_container.data_root.clone());
            cmd.arg("--cwd");
            cmd.arg("/");
            cmd.arg("--command");
            cmd.arg(ctx_sandbox_container_runtime::SHARED_VM_SANDBOX_CLI_GUEST_BIN);
            cmd.arg("--user");
            cmd.arg("root");
            if let Ok(sandbox_env) = ctx_sandbox_container_runtime::sandbox_cli_env_for_mode(
                &shared_vm_container.data_root,
                &ctx_sandbox_container_runtime::SandboxCommandMode::SharedVm {
                    helper_path: shared_vm_container.helper_path.clone(),
                },
            ) {
                let mut env_pairs = sandbox_env.into_iter().collect::<Vec<_>>();
                env_pairs.sort_by(|(left, _), (right, _)| left.cmp(right));
                for (key, value) in env_pairs {
                    cmd.arg("--env");
                    cmd.arg(format!("{key}={value}"));
                }
            }
            cmd.arg("--pty");
            cmd.arg("--");
            cmd.arg("exec");
            cmd.arg("-i");
            cmd.arg("-t");
            cmd.arg("--workdir");
            cmd.arg(shared_vm_container.workdir.clone());
            cmd.arg("--env");
            cmd.arg("TERM=xterm-256color");
            if let Some(user) = shared_vm_container.user.as_ref() {
                cmd.arg("--user");
                cmd.arg(user.clone());
            }
            for (key, value) in &req.env {
                cmd.arg("--env");
                cmd.arg(format!("{key}={value}"));
            }
            cmd.arg(format!(
                "ctx-harness-{}",
                shared_vm_container.workspace_id.0
            ));
            cmd.arg(req.shell.clone());
            cmd
        } else {
            let mut cmd = CommandBuilder::new(req.shell.clone());
            cmd.cwd(req.cwd.clone());
            cmd.env("TERM", "xterm-256color");
            for (key, value) in &req.env {
                cmd.env(key, value);
            }
            cmd
        };

        let child = pair.slave.spawn_command(cmd).context("spawn terminal")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let mut writer = pair.master.take_writer().context("take pty writer")?;

        let (output_tx, _) = broadcast::channel(1024);
        let (status_tx, _) = broadcast::channel(16);
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let output_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));

        let now = Utc::now();
        let runtime = Arc::new(Mutex::new(TerminalRuntime {
            status: TerminalStatus::Running,
            exit_code: None,
            updated_at: now,
            last_activity: now,
            connected_clients: 0,
        }));

        let output_buffer_clone = output_buffer.clone();
        let output_tx_clone = output_tx.clone();
        let runtime_output = runtime.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = &buf[..n];
                        push_output(
                            &output_buffer_clone,
                            &output_tx_clone,
                            &runtime_output,
                            bytes,
                        );
                    }
                    Err(_) => break,
                }
            }
        });

        std::thread::spawn(move || {
            while let Some(data) = input_rx.blocking_recv() {
                if writer.write_all(&data).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
        });

        let runtime_clone = runtime.clone();
        let status_tx_clone = status_tx.clone();
        let child_arc = Arc::new(Mutex::new(child));
        let child_arc_clone = child_arc.clone();

        std::thread::spawn(move || loop {
            let exit: Option<portable_pty::ExitStatus> = {
                let mut child = lock_or_recover(child_arc_clone.as_ref(), "terminal child");
                child.try_wait().ok().flatten()
            };
            if let Some(status) = exit {
                let exit_code = i32::try_from(status.exit_code()).ok();
                let mut runtime = lock_or_recover(runtime_clone.as_ref(), "terminal runtime");
                runtime.status = TerminalStatus::Exited;
                runtime.exit_code = exit_code;
                runtime.updated_at = Utc::now();
                runtime.last_activity = runtime.updated_at;
                let _ = status_tx_clone.send(TerminalStatusEvent {
                    status: TerminalStatus::Exited,
                    exit_code,
                });
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        });

        let id = TerminalId::new();
        let title = PathBuf::from(&req.shell)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("terminal")
            .to_string();

        let info = TerminalSession {
            id,
            workspace_id: req.workspace_id,
            task_id: req.task_id,
            session_id: req.session_id,
            worktree_id: req.worktree_id,
            cwd: req.cwd.to_string_lossy().to_string(),
            shell: req.shell,
            title,
            status: TerminalStatus::Running,
            exit_code: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let session = Arc::new(TerminalSessionHandle {
            info,
            container_backed: req.native_container.is_some() || req.shared_vm_container.is_some(),
            runtime,
            output_tx,
            status_tx,
            output_buffer,
            backend: TerminalBackend::Local {
                input_tx,
                master: Arc::new(Mutex::new(pair.master)),
                child: child_arc,
            },
        });

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id, session.clone());
        Ok(session)
    }

    pub async fn create_remote(
        &self,
        req: TerminalCreateRequest,
        remote: RemoteTerminalRequest,
    ) -> Result<Arc<TerminalSessionHandle>> {
        let id = remote.terminal_id;
        let title = PathBuf::from(&req.shell)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("terminal")
            .to_string();

        let (output_tx, _) = broadcast::channel(1024);
        let (status_tx, _) = broadcast::channel(16);
        let output_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<RemoteTerminalOutgoing>();

        let now = Utc::now();
        let runtime = Arc::new(Mutex::new(TerminalRuntime {
            status: TerminalStatus::Running,
            exit_code: None,
            updated_at: now,
            last_activity: now,
            connected_clients: 0,
        }));

        let info = TerminalSession {
            id,
            workspace_id: req.workspace_id,
            task_id: req.task_id,
            session_id: req.session_id,
            worktree_id: req.worktree_id,
            cwd: req.cwd.to_string_lossy().to_string(),
            shell: req.shell,
            title,
            status: TerminalStatus::Running,
            exit_code: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let session = Arc::new(TerminalSessionHandle {
            info,
            container_backed: false,
            runtime: runtime.clone(),
            output_tx: output_tx.clone(),
            status_tx: status_tx.clone(),
            output_buffer: output_buffer.clone(),
            backend: TerminalBackend::Remote {
                outbound_tx: outbound_tx.clone(),
            },
        });

        let remote_clone = remote.clone();
        let output_buffer_clone = output_buffer.clone();
        let output_tx_clone = output_tx.clone();
        let status_tx_clone = status_tx.clone();
        let runtime_clone = runtime.clone();

        tokio::spawn(async move {
            let mut outbound_rx = outbound_rx;
            let mut backoff = Duration::from_millis(TERMINAL_RECONNECT_BASE_MS);
            loop {
                let ws_stream = match connect_terminal_gateway(&remote_clone).await {
                    Ok(stream) => {
                        backoff = Duration::from_millis(TERMINAL_RECONNECT_BASE_MS);
                        stream
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "terminal gateway connection failed");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff + backoff)
                            .min(Duration::from_millis(TERMINAL_RECONNECT_MAX_MS));
                        continue;
                    }
                };

                let (mut ws_write, mut ws_read) = ws_stream.split();
                let mut ping = tokio::time::interval(TERMINAL_PING_INTERVAL);
                ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

                loop {
                    tokio::select! {
                        outbound = outbound_rx.recv() => {
                            let Some(outbound) = outbound else {
                                return;
                            };
                            let send_result = match outbound {
                                RemoteTerminalOutgoing::Binary(data) => {
                                    ws_write.send(Message::Binary(data.into())).await
                                }
                                RemoteTerminalOutgoing::Text(text) => {
                                    ws_write.send(Message::Text(text.into())).await
                                }
                                RemoteTerminalOutgoing::Close => ws_write.send(Message::Close(None)).await,
                            };
                            if send_result.is_err() {
                                break;
                            }
                        }
                        msg = ws_read.next() => {
                            let msg = match msg {
                                Some(Ok(msg)) => msg,
                                Some(Err(_)) | None => break,
                            };
                            match msg {
                                Message::Binary(data) => {
                                    push_output(&output_buffer_clone, &output_tx_clone, &runtime_clone, &data);
                                }
                                Message::Text(text) => {
                                    if let Ok(parsed) =
                                        serde_json::from_str::<TerminalServerMessage>(text.as_str())
                                    {
                                        match parsed {
                                            TerminalServerMessage::Status { status, exit_code } => {
                                                let mut runtime =
                                                    lock_or_recover(runtime_clone.as_ref(), "terminal runtime");
                                                runtime.status = status.clone();
                                                runtime.exit_code = exit_code;
                                                runtime.updated_at = Utc::now();
                                                runtime.last_activity = runtime.updated_at;
                                                let _ = status_tx_clone
                                                    .send(TerminalStatusEvent { status, exit_code });
                                            }
                                            TerminalServerMessage::Pong => {}
                                        }
                                    } else {
                                        push_output(&output_buffer_clone, &output_tx_clone, &runtime_clone, text.as_bytes());
                                    }
                                }
                                Message::Ping(payload) => {
                                    let _ = ws_write.send(Message::Pong(payload)).await;
                                }
                                Message::Pong(_) => {}
                                Message::Close(_) => break,
                                Message::Frame(_) => {}
                            }
                        }
                        _ = ping.tick() => {
                            if ws_write.send(Message::Ping(Vec::new().into())).await.is_err() {
                                break;
                            }
                        }
                    }
                }

                tokio::time::sleep(backoff).await;
                backoff = (backoff + backoff).min(Duration::from_millis(TERMINAL_RECONNECT_MAX_MS));
            }
        });

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id, session.clone());
        Ok(session)
    }

    pub async fn remove(&self, id: TerminalId) -> Option<Arc<TerminalSessionHandle>> {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(&id)
    }

    async fn reap_idle(&self, idle_for: Duration) -> Result<()> {
        let handles = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, handle)| (*id, handle.clone()))
                .collect::<Vec<_>>()
        };
        let mut to_close = Vec::new();
        for (id, handle) in handles {
            let runtime = lock_or_recover(handle.runtime.as_ref(), "terminal runtime");
            if !matches!(runtime.status, TerminalStatus::Running) {
                continue;
            }
            if runtime.connected_clients > 0 {
                continue;
            }
            let idle = Utc::now() - runtime.last_activity;
            if idle.to_std().unwrap_or_default() > idle_for {
                to_close.push(id);
            }
        }
        for id in to_close {
            if let Some(handle) = self.remove(id).await {
                let _ = handle.kill();
                handle.mark_exited(None);
            }
        }
        Ok(())
    }
}

fn push_output(
    output_buffer: &Arc<Mutex<VecDeque<u8>>>,
    output_tx: &broadcast::Sender<Vec<u8>>,
    runtime: &Arc<Mutex<TerminalRuntime>>,
    bytes: &[u8],
) {
    {
        let mut buffer = lock_or_recover(output_buffer.as_ref(), "terminal output buffer");
        for b in bytes {
            buffer.push_back(*b);
        }
        while buffer.len() > MAX_OUTPUT_BYTES {
            buffer.pop_front();
        }
    }
    {
        let mut runtime = lock_or_recover(runtime.as_ref(), "terminal runtime");
        runtime.last_activity = Utc::now();
        runtime.updated_at = runtime.last_activity;
    }
    let _ = output_tx.send(bytes.to_vec());
}

#[derive(Debug)]
struct GatewayCertVerifier {
    inner: Arc<WebPkiServerVerifier>,
    server_name: ServerName<'static>,
    pinned_der: Option<Vec<u8>>,
}

impl ServerCertVerifier for GatewayCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if let Some(pinned) = &self.pinned_der {
            if end_entity.as_ref() == pinned.as_slice() {
                return Ok(ServerCertVerified::assertion());
            }
        }
        self.inner.verify_server_cert(
            end_entity,
            intermediates,
            &self.server_name,
            ocsp_response,
            now,
        )
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn gateway_ws_connector(pem: &str) -> Result<Connector> {
    let mut roots = RootCertStore::empty();
    let mut pinned_der: Option<Vec<u8>> = None;
    for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
        let cert = cert.context("parsing gateway CA")?;
        if pinned_der.is_none() {
            pinned_der = Some(cert.as_ref().to_vec());
        }
        roots.add(cert).context("adding gateway CA")?;
    }
    let verifier = WebPkiServerVerifier::builder(Arc::new(roots.clone()))
        .build()
        .context("building gateway verifier")?;
    let server_name =
        ServerName::try_from("ctx-gateway").context("building gateway server name")?;
    let verifier = GatewayCertVerifier {
        inner: verifier,
        server_name,
        pinned_der,
    };
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(verifier));
    Ok(Connector::Rustls(Arc::new(config)))
}

async fn connect_terminal_gateway(
    remote: &RemoteTerminalRequest,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    let mut base = remote.gateway_url.trim_end_matches('/').to_string();
    if base.starts_with("https://") {
        base = base.replacen("https://", "wss://", 1);
    } else if base.starts_with("http://") {
        base = base.replacen("http://", "ws://", 1);
    } else if !base.starts_with("ws://") && !base.starts_with("wss://") {
        base = format!("ws://{base}");
    }
    let url = format!(
        "{base}/workers/{}/terminals/{}/daemon",
        remote.worker_id, remote.terminal_id.0
    );
    let mut req = url
        .as_str()
        .into_client_request()
        .context("building terminal relay request")?;
    if let Some(token) = remote.token.as_deref() {
        req.headers_mut().insert(
            "x-ctx-gateway-token",
            token.parse().context("parsing gateway token")?,
        );
    }
    let (ws_stream, _) = if let Some(pem) = remote.gateway_ca_pem.as_deref() {
        let connector = gateway_ws_connector(pem)?;
        connect_async_tls_with_config(req, None, false, Some(connector))
            .await
            .with_context(|| format!("connecting to gateway terminal relay at {url}"))?
    } else {
        connect_async(req)
            .await
            .with_context(|| format!("connecting to gateway terminal relay at {url}"))?
    };
    Ok(ws_stream)
}
