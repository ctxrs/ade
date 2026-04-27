use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_FPS: u32 = 30;
const DEFAULT_IDLE_SECS: u64 = 30 * 60;
const REAPER_INTERVAL_SECS: u64 = 60;
const WEB_SESSION_STREAM_TOKEN_TTL_SECS: i64 = 30;
pub const WEB_SESSION_WORKER_AUTH_HEADER: &str = "x-ctx-worker-auth";

mod runtime_support;
mod view;
mod worker_bundle;

#[cfg(test)]
mod tests;

use runtime_support::{
    allocate_port, build_run_payload, build_signal_connect_path, build_stream_connect_path,
    build_stream_path, log_stream,
};
pub use view::render_web_session_view;
pub use worker_bundle::ensure_worker_bundle;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSessionStatus {
    Running,
    Closed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionViewport {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionInfo {
    pub id: String,
    pub kind: String,
    pub session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub status: WebSessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub url: String,
    pub viewport: WebSessionViewport,
    pub fps: u32,
    pub viewers: u32,
    pub stream_path: String,
    pub stream_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionCreateRequest {
    pub url: String,
    pub viewport: Option<WebSessionViewport>,
    pub fps: Option<u32>,
    pub work_dir: Option<PathBuf>,
    pub session_id: Option<String>,
    pub worktree_id: Option<String>,
    pub node_bin: PathBuf,
    pub worker_path: PathBuf,
    pub node_modules_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionRunRequest {
    pub code: Option<String>,
    pub script_path: Option<String>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSessionRunResponse {
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NodeRuntimeSpec {
    pub node_bin: PathBuf,
    pub npm_cli_js: PathBuf,
}

pub struct WorkerBundle {
    pub worker_path: PathBuf,
    pub node_modules_path: PathBuf,
}

pub struct WebSessionHandle {
    info: WebSessionInfo,
    stream_tokens: Arc<Mutex<HashMap<String, WebSessionStreamToken>>>,
    worker_auth_secret: String,
    runtime: Arc<Mutex<WebSessionRuntime>>,
    run_lock: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WebSessionStreamTokenKind {
    View,
    Signal,
}

#[derive(Debug, Clone)]
struct WebSessionStreamToken {
    kind: WebSessionStreamTokenKind,
    expires_at: DateTime<Utc>,
}

struct WebSessionRuntime {
    status: WebSessionStatus,
    updated_at: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    viewers: u32,
    worker_port: u16,
    child: Option<Child>,
    work_dir: Option<PathBuf>,
}

impl WebSessionHandle {
    pub async fn snapshot(&self) -> WebSessionInfo {
        let runtime = self.runtime.lock().await;
        WebSessionInfo {
            status: runtime.status.clone(),
            updated_at: runtime.updated_at,
            last_activity: runtime.last_activity,
            viewers: runtime.viewers,
            ..self.info.clone()
        }
    }

    pub async fn touch(&self) {
        let mut runtime = self.runtime.lock().await;
        runtime.last_activity = Utc::now();
        runtime.updated_at = runtime.last_activity;
    }

    pub async fn set_viewers(&self, viewers: u32) {
        let mut runtime = self.runtime.lock().await;
        runtime.viewers = viewers;
        runtime.updated_at = Utc::now();
    }

    pub async fn worker_port(&self) -> u16 {
        let runtime = self.runtime.lock().await;
        runtime.worker_port
    }

    pub async fn work_dir(&self) -> Option<PathBuf> {
        let runtime = self.runtime.lock().await;
        runtime.work_dir.clone()
    }

    async fn issue_stream_connect_path(
        &self,
        kind: WebSessionStreamTokenKind,
    ) -> (String, DateTime<Utc>) {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::seconds(WEB_SESSION_STREAM_TOKEN_TTL_SECS);
        let mut tokens = self.stream_tokens.lock().await;
        tokens.retain(|_, access| access.expires_at > now);
        let token = Uuid::new_v4().to_string();
        tokens.insert(token.clone(), WebSessionStreamToken { kind, expires_at });
        let path = match kind {
            WebSessionStreamTokenKind::View => build_stream_connect_path(&self.info.id, &token),
            WebSessionStreamTokenKind::Signal => build_signal_connect_path(&self.info.id, &token),
        };
        (path, expires_at)
    }

    pub async fn issue_view_connect_path(&self) -> (String, DateTime<Utc>) {
        self.issue_stream_connect_path(WebSessionStreamTokenKind::View)
            .await
    }

    pub async fn issue_signal_connect_path(&self) -> (String, DateTime<Utc>) {
        self.issue_stream_connect_path(WebSessionStreamTokenKind::Signal)
            .await
    }

    async fn consume_stream_token(&self, token: &str, kind: WebSessionStreamTokenKind) -> bool {
        let now = Utc::now();
        let mut tokens = self.stream_tokens.lock().await;
        tokens.retain(|_, access| access.expires_at > now);
        let Some(access) = tokens.get(token).cloned() else {
            return false;
        };
        if access.expires_at <= now || access.kind != kind {
            return false;
        }
        tokens.remove(token);
        true
    }

    pub async fn consume_view_token(&self, token: &str) -> bool {
        self.consume_stream_token(token, WebSessionStreamTokenKind::View)
            .await
    }

    pub async fn consume_signal_token(&self, token: &str) -> bool {
        self.consume_stream_token(token, WebSessionStreamTokenKind::Signal)
            .await
    }

    pub fn worker_auth_secret(&self) -> &str {
        &self.worker_auth_secret
    }

    pub async fn close(&self) -> Result<()> {
        let mut runtime = self.runtime.lock().await;
        if let Some(mut child) = runtime.child.take() {
            let _ = child.kill().await;
        }
        runtime.status = WebSessionStatus::Closed;
        runtime.updated_at = Utc::now();
        Ok(())
    }
}

pub struct WebSessionManager {
    sessions: Mutex<HashMap<String, Arc<WebSessionHandle>>>,
    client: Client,
    next_display: Mutex<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WebSessionManagerStats {
    pub session_count: usize,
    pub running: usize,
    pub closed: usize,
    pub error: usize,
    pub total_viewers: u32,
    pub active_children: usize,
}

impl WebSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            client: Client::new(),
            next_display: Mutex::new(90),
        }
    }

    pub async fn stats(&self) -> WebSessionManagerStats {
        let handles = {
            let sessions = self.sessions.lock().await;
            sessions.values().cloned().collect::<Vec<_>>()
        };
        let mut running = 0;
        let mut closed = 0;
        let mut error = 0;
        let mut total_viewers = 0;
        let mut active_children = 0;
        for handle in handles.iter() {
            let runtime = handle.runtime.lock().await;
            match runtime.status {
                WebSessionStatus::Running => running += 1,
                WebSessionStatus::Closed => closed += 1,
                WebSessionStatus::Error => error += 1,
            }
            total_viewers += runtime.viewers;
            if runtime.child.is_some() {
                active_children += 1;
            }
        }
        WebSessionManagerStats {
            session_count: handles.len(),
            running,
            closed,
            error,
            total_viewers,
            active_children,
        }
    }

    pub async fn list(&self) -> Vec<WebSessionInfo> {
        let handles = {
            let sessions = self.sessions.lock().await;
            sessions.values().cloned().collect::<Vec<_>>()
        };
        let mut out = Vec::with_capacity(handles.len());
        for session in handles {
            out.push(session.snapshot().await);
        }
        out
    }

    pub async fn get(&self, id: &str) -> Option<Arc<WebSessionHandle>> {
        let sessions = self.sessions.lock().await;
        sessions.get(id).cloned()
    }

    pub async fn create(&self, req: WebSessionCreateRequest) -> Result<Arc<WebSessionHandle>> {
        let id = Uuid::new_v4().to_string();
        let worker_auth_secret = Uuid::new_v4().to_string();
        let viewport = req.viewport.clone().unwrap_or(WebSessionViewport {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        });
        let fps = req.fps.unwrap_or(DEFAULT_FPS);
        let display = self.next_display().await?;
        let worker_port = allocate_port()?;

        let stream_path = build_stream_path(&id);
        let created_at = Utc::now();

        let info = WebSessionInfo {
            id: id.clone(),
            kind: "web".to_string(),
            session_id: req.session_id.clone(),
            worktree_id: req.worktree_id.clone(),
            status: WebSessionStatus::Running,
            created_at,
            updated_at: created_at,
            last_activity: created_at,
            url: req.url.clone(),
            viewport: viewport.clone(),
            fps,
            viewers: 0,
            stream_path,
            stream_url: None,
        };

        let runtime = WebSessionRuntime {
            status: WebSessionStatus::Running,
            updated_at: created_at,
            last_activity: created_at,
            viewers: 0,
            worker_port,
            child: None,
            work_dir: req.work_dir.clone(),
        };

        let handle = Arc::new(WebSessionHandle {
            info,
            stream_tokens: Arc::new(Mutex::new(HashMap::new())),
            worker_auth_secret,
            runtime: Arc::new(Mutex::new(runtime)),
            run_lock: Arc::new(Mutex::new(())),
        });

        self.spawn_worker(&handle, &req, worker_port, &display)
            .await?;
        self.await_worker_ready(worker_port, handle.worker_auth_secret())
            .await?;

        let mut sessions = self.sessions.lock().await;
        sessions.insert(id.clone(), handle.clone());
        Ok(handle)
    }

    pub async fn run(&self, id: &str, req: WebSessionRunRequest) -> Result<WebSessionRunResponse> {
        let handle = self.get(id).await.context("session not found")?;
        let _guard = handle.run_lock.lock().await;
        handle.touch().await;

        let payload = build_run_payload(&handle, req).await?;
        let port = handle.worker_port().await;
        let url = format!("http://127.0.0.1:{port}/run");

        let resp = self
            .client
            .post(&url)
            .header(
                WEB_SESSION_WORKER_AUTH_HEADER,
                handle.worker_auth_secret().to_string(),
            )
            .json(&payload)
            .send()
            .await
            .context("sending run request")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Ok(WebSessionRunResponse {
                ok: false,
                result: None,
                error: Some(format!("worker error: {body}")),
            });
        }

        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(WebSessionRunResponse {
            ok: value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            result: value.get("result").cloned(),
            error: value
                .get("error")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    pub async fn eval(&self, id: &str, req: WebSessionRunRequest) -> Result<WebSessionRunResponse> {
        let handle = self.get(id).await.context("session not found")?;
        let _guard = handle.run_lock.lock().await;
        handle.touch().await;

        let payload = build_run_payload(&handle, req).await?;
        let port = handle.worker_port().await;
        let url = format!("http://127.0.0.1:{port}/eval");

        let resp = self
            .client
            .post(&url)
            .header(
                WEB_SESSION_WORKER_AUTH_HEADER,
                handle.worker_auth_secret().to_string(),
            )
            .json(&payload)
            .send()
            .await
            .context("sending eval request")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Ok(WebSessionRunResponse {
                ok: false,
                result: None,
                error: Some(format!("worker error: {body}")),
            });
        }

        let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        Ok(WebSessionRunResponse {
            ok: value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            result: value.get("result").cloned(),
            error: value
                .get("error")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        })
    }

    pub async fn close(&self, id: &str) -> Result<()> {
        let handle = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(id)
        };
        let handle = handle.context("session not found")?;
        handle.close().await?;
        Ok(())
    }

    pub async fn close_for_task(
        &self,
        session_ids: &HashSet<String>,
        worktree_ids: &HashSet<String>,
    ) -> Result<usize> {
        let handles = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, handle)| (id.clone(), handle.clone()))
                .collect::<Vec<_>>()
        };
        let mut to_close = Vec::new();
        for (id, handle) in handles {
            let info = &handle.info;
            let matches_session = info
                .session_id
                .as_ref()
                .map(|sid| session_ids.contains(sid))
                .unwrap_or(false);
            let matches_worktree = info
                .worktree_id
                .as_ref()
                .map(|wid| worktree_ids.contains(wid))
                .unwrap_or(false);
            if matches_session || matches_worktree {
                to_close.push(id);
            }
        }

        let mut closed = 0;
        for id in to_close {
            self.close(&id).await?;
            closed += 1;
        }
        Ok(closed)
    }

    pub async fn bump_viewers(&self, id: &str, delta: i32) -> Result<u32> {
        let handle = self.get(id).await.context("session not found")?;
        handle.touch().await;
        let mut runtime = handle.runtime.lock().await;
        let next = (runtime.viewers as i32 + delta).max(0) as u32;
        runtime.viewers = next;
        runtime.updated_at = Utc::now();
        Ok(next)
    }

    pub async fn start_reaper(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(REAPER_INTERVAL_SECS));
            loop {
                interval.tick().await;
                if let Err(err) = self.reap_idle(Duration::from_secs(DEFAULT_IDLE_SECS)).await {
                    tracing::warn!("web session reap failed: {err:#}");
                }
            }
        });
    }

    async fn reap_idle(&self, idle_for: Duration) -> Result<()> {
        let mut to_close = Vec::new();
        let handles = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, handle)| (id.clone(), handle.clone()))
                .collect::<Vec<_>>()
        };
        for (id, handle) in handles {
            let snapshot = handle.snapshot().await;
            if snapshot.status != WebSessionStatus::Running {
                continue;
            }
            let idle = Utc::now() - snapshot.last_activity;
            if idle.to_std().unwrap_or_default() > idle_for && snapshot.viewers == 0 {
                to_close.push(id);
            }
        }

        for id in to_close {
            let _ = self.close(&id).await;
        }
        Ok(())
    }

    async fn next_display(&self) -> Result<String> {
        let mut guard = self.next_display.lock().await;
        for _ in 0..1000 {
            let candidate = *guard;
            *guard += 1;
            let lock_path = format!("/tmp/.X{candidate}-lock");
            if !Path::new(&lock_path).exists() {
                return Ok(format!(":{candidate}"));
            }
        }
        anyhow::bail!("failed to allocate X display");
    }

    async fn spawn_worker(
        &self,
        handle: &Arc<WebSessionHandle>,
        req: &WebSessionCreateRequest,
        port: u16,
        display: &str,
    ) -> Result<()> {
        let xvfb_path = which::which("Xvfb").context("Xvfb not found in PATH")?;
        let ffmpeg_path = which::which("ffmpeg").context("ffmpeg not found in PATH")?;
        let node_path = req.node_bin.clone();
        let worker_path = req.worker_path.clone();
        let node_modules_path = req.node_modules_path.clone();

        let mut cmd = Command::new(node_path);
        cmd.arg(worker_path);
        cmd.env("PORT", port.to_string());
        cmd.env("TARGET_URL", req.url.clone());
        cmd.env(
            "WIDTH",
            req.viewport
                .as_ref()
                .map(|v| v.width)
                .unwrap_or(DEFAULT_WIDTH)
                .to_string(),
        );
        cmd.env(
            "HEIGHT",
            req.viewport
                .as_ref()
                .map(|v| v.height)
                .unwrap_or(DEFAULT_HEIGHT)
                .to_string(),
        );
        cmd.env("FPS", req.fps.unwrap_or(DEFAULT_FPS).to_string());
        cmd.env("DISPLAY", display);
        cmd.env("NODE_PATH", node_modules_path);
        cmd.env("MAP_META_TO_CTRL", "1");
        cmd.env("FFMPEG_PATH", ffmpeg_path.to_string_lossy().to_string());
        cmd.env("XVFB_PATH", xvfb_path.to_string_lossy().to_string());
        if let Some(work_dir) = &req.work_dir {
            cmd.env("WORK_DIR", work_dir.to_string_lossy().to_string());
            cmd.current_dir(work_dir);
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("spawning web session worker")?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(format!("{}\n", handle.worker_auth_secret()).as_bytes())
                .await
                .context("writing web session worker auth secret")?;
        }
        if let Some(stdout) = child.stdout.take() {
            tokio::spawn(log_stream(stdout, "web-session"));
        }
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(log_stream(stderr, "web-session"));
        }

        let mut runtime = handle.runtime.lock().await;
        runtime.child = Some(child);
        Ok(())
    }

    async fn await_worker_ready(&self, port: u16, worker_auth_secret: &str) -> Result<()> {
        let url = format!("http://127.0.0.1:{port}/health");
        for _ in 0..40 {
            let resp = self
                .client
                .get(&url)
                .header(WEB_SESSION_WORKER_AUTH_HEADER, worker_auth_secret)
                .send()
                .await;
            if let Ok(resp) = resp {
                if resp.status().is_success() {
                    let unauthenticated = self.client.get(&url).send().await;
                    match unauthenticated {
                        Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
                            return Ok(());
                        }
                        Ok(resp) => {
                            anyhow::bail!(
                                "worker health endpoint must reject unauthenticated loopback access (got {})",
                                resp.status()
                            );
                        }
                        Err(err) => {
                            anyhow::bail!("worker health endpoint auth verification failed: {err}");
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        anyhow::bail!("worker did not become ready");
    }
}

impl Default for WebSessionManager {
    fn default() -> Self {
        Self::new()
    }
}
