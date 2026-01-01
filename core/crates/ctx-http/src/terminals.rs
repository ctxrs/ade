use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tokio::sync::{broadcast, mpsc};

use ctx_core::ids::{SessionId, TaskId, TerminalId, TrackId, WorkspaceId, WorktreeId};
use ctx_core::models::{TerminalSession, TerminalStatus};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct TerminalCreateRequest {
    pub workspace_id: WorkspaceId,
    pub task_id: Option<TaskId>,
    pub track_id: Option<TrackId>,
    pub session_id: Option<SessionId>,
    pub worktree_id: Option<WorktreeId>,
    pub cwd: PathBuf,
    pub shell: String,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct TerminalStatusEvent {
    pub status: TerminalStatus,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone)]
struct TerminalRuntime {
    status: TerminalStatus,
    exit_code: Option<i32>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

pub struct TerminalSessionHandle {
    info: TerminalSession,
    runtime: Arc<Mutex<TerminalRuntime>>,
    output_tx: broadcast::Sender<Vec<u8>>,
    status_tx: broadcast::Sender<TerminalStatusEvent>,
    output_buffer: Arc<Mutex<VecDeque<u8>>>,
    input_tx: mpsc::UnboundedSender<Vec<u8>>,
    master: Arc<Mutex<Box<dyn portable_pty::MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl TerminalSessionHandle {
    pub fn snapshot(&self) -> TerminalSession {
        let runtime = self.runtime.lock().expect("terminal runtime lock");
        TerminalSession {
            status: runtime.status.clone(),
            exit_code: runtime.exit_code,
            updated_at: runtime.updated_at,
            ..self.info.clone()
        }
    }

    pub fn output_receiver(&self) -> broadcast::Receiver<Vec<u8>> {
        self.output_tx.subscribe()
    }

    pub fn status_receiver(&self) -> broadcast::Receiver<TerminalStatusEvent> {
        self.status_tx.subscribe()
    }

    pub fn output_snapshot(&self) -> Vec<u8> {
        let buffer = self.output_buffer.lock().expect("terminal buffer lock");
        buffer.iter().copied().collect()
    }

    pub fn send_input(&self, data: Vec<u8>) {
        let _ = self.input_tx.send(data);
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let master = self.master.lock().expect("terminal master lock");
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("resize pty")?;
        Ok(())
    }

    pub fn kill(&self) -> Result<()> {
        let mut child = self.child.lock().expect("terminal child lock");
        child.kill().context("kill terminal")?;
        Ok(())
    }

    pub fn mark_exited(&self, exit_code: Option<i32>) {
        let mut runtime = self.runtime.lock().expect("terminal runtime lock");
        runtime.status = TerminalStatus::Exited;
        runtime.exit_code = exit_code;
        runtime.updated_at = Utc::now();
        let _ = self.status_tx.send(TerminalStatusEvent {
            status: TerminalStatus::Exited,
            exit_code,
        });
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

    pub async fn has_running(&self) -> bool {
        let sessions = self.sessions.lock().await;
        sessions
            .values()
            .any(|sess| matches!(sess.snapshot().status, TerminalStatus::Running))
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

        let mut cmd = CommandBuilder::new(req.shell.clone());
        cmd.cwd(req.cwd.clone());
        cmd.env("TERM", "xterm-256color");

        let child = pair.slave.spawn_command(cmd).context("spawn terminal")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
        let mut writer = pair.master.take_writer().context("take pty writer")?;

        let (output_tx, _) = broadcast::channel(1024);
        let (status_tx, _) = broadcast::channel(16);
        let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let output_buffer = Arc::new(Mutex::new(VecDeque::with_capacity(8192)));

        let output_buffer_clone = output_buffer.clone();
        let output_tx_clone = output_tx.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = &buf[..n];
                        {
                            let mut buffer = output_buffer_clone
                                .lock()
                                .expect("terminal output buffer lock");
                            for b in bytes {
                                buffer.push_back(*b);
                            }
                            while buffer.len() > MAX_OUTPUT_BYTES {
                                buffer.pop_front();
                            }
                        }
                        let _ = output_tx_clone.send(bytes.to_vec());
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

        let runtime = Arc::new(Mutex::new(TerminalRuntime {
            status: TerminalStatus::Running,
            exit_code: None,
            updated_at: Utc::now(),
        }));
        let runtime_clone = runtime.clone();
        let status_tx_clone = status_tx.clone();
        let child_arc = Arc::new(Mutex::new(child));
        let child_arc_clone = child_arc.clone();

        std::thread::spawn(move || loop {
            let exit: Option<portable_pty::ExitStatus> = {
                let mut child = child_arc_clone.lock().expect("terminal child lock");
                child.try_wait().ok().flatten()
            };
            if let Some(status) = exit {
                let exit_code = i32::try_from(status.exit_code()).ok();
                let mut runtime = runtime_clone.lock().expect("terminal runtime lock");
                runtime.status = TerminalStatus::Exited;
                runtime.exit_code = exit_code;
                runtime.updated_at = Utc::now();
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
            track_id: req.track_id,
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
            runtime,
            output_tx,
            status_tx,
            output_buffer,
            input_tx,
            master: Arc::new(Mutex::new(pair.master)),
            child: child_arc,
        });

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id, session.clone());
        Ok(session)
    }

    pub async fn remove(&self, id: TerminalId) -> Option<Arc<TerminalSessionHandle>> {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(&id)
    }
}
