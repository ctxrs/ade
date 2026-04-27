#[path = "terminals_gateway.rs"]
mod terminals_gateway;
#[path = "terminals_handle.rs"]
mod terminals_handle;
#[path = "terminals_manager.rs"]
mod terminals_manager;

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
use uuid::Uuid;

use ctx_core::env::DAEMON_AUTH_ENV_VARS;
use ctx_core::ids::{SessionId, TaskId, TerminalId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, TerminalStatus};
use terminals_gateway::connect_terminal_gateway;
use terminals_handle::build_stream_path;
use terminals_manager::push_output;
pub use terminals_manager::TerminalManagerStats;

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
pub const DEFAULT_OUTPUT_TAIL_BYTES: usize = 20 * 1024;

fn scrub_daemon_auth_env(cmd: &mut CommandBuilder) {
    for key in DAEMON_AUTH_ENV_VARS {
        cmd.env_remove(key);
    }
}
const TERMINAL_PING_INTERVAL: Duration = Duration::from_secs(25);
const TERMINAL_STREAM_TOKEN_TTL_SECS: i64 = 30;
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
    stream_tokens: Arc<Mutex<HashMap<String, chrono::DateTime<chrono::Utc>>>>,
    container_backed: bool,
    runtime: Arc<Mutex<TerminalRuntime>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    status_tx: broadcast::Sender<TerminalStatusEvent>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    backend: TerminalBackend,
}

#[derive(Default)]
pub struct TerminalManager {
    sessions: tokio::sync::Mutex<HashMap<TerminalId, Arc<TerminalSessionHandle>>>,
}

impl TerminalManager {
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

        let mut cmd = if let Some(native_container) = &req.native_container {
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

        scrub_daemon_auth_env(&mut cmd);
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
            stream_path: build_stream_path(id),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let session = Arc::new(TerminalSessionHandle {
            info,
            stream_tokens: Arc::new(Mutex::new(HashMap::new())),
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
            stream_path: build_stream_path(id),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let session = Arc::new(TerminalSessionHandle {
            info,
            stream_tokens: Arc::new(Mutex::new(HashMap::new())),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = &self.previous {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn scrub_daemon_auth_env_removes_sensitive_tokens_from_pty_commands() {
        let _auth = ScopedEnvVar::set("CTX_AUTH_TOKEN", "daemon-token");
        let _mcp = ScopedEnvVar::set("CTX_MCP_TOKEN", "mcp-token");
        let mut cmd = CommandBuilder::new("/bin/sh");

        scrub_daemon_auth_env(&mut cmd);

        for key in DAEMON_AUTH_ENV_VARS {
            assert_eq!(cmd.get_env(key), None, "expected {key} to be removed");
        }
    }
}
