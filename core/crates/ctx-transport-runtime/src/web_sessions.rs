use std::collections::{HashMap, HashSet};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use uuid::Uuid;

const DEFAULT_WIDTH: u32 = 1280;
const DEFAULT_HEIGHT: u32 = 720;
const DEFAULT_FPS: u32 = 30;
const DEFAULT_IDLE_SECS: u64 = 30 * 60;
const REAPER_INTERVAL_SECS: u64 = 60;

const WORKER_PACKAGE_JSON: &str = include_str!("../../../packages/web-session-worker/package.json");
const WORKER_SCRIPT: &str = include_str!("../../../packages/web-session-worker/bin/worker.mjs");

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

pub fn render_web_session_view(session: &WebSessionInfo, signal_path: &str) -> String {
    fn escape_html(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\"', "&quot;")
            .replace('\'', "&#39;")
    }

    const TEMPLATE: &str = r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <title>Web Session</title>
    <style>
      :root { color-scheme: dark; }
      body { margin: 0; background: #0b0b0b; color: #ddd; font-family: system-ui, sans-serif; }
      header { padding: 8px 12px; background: #111; font-size: 14px; display: flex; gap: 12px; align-items: center; }
      #status { font-size: 12px; opacity: 0.7; }
      #wrap { width: 100vw; height: calc(100vh - 36px); display: flex; align-items: center; justify-content: center; background: #000; }
      video { width: 100%; height: 100%; object-fit: contain; background: #000; cursor: default; }
    </style>
  </head>
  <body>
    <header>
      <div>Web Session: %%URL%%</div>
      <div id="status">connecting…</div>
    </header>
    <div id="wrap">
      <video id="view" autoplay playsinline muted></video>
    </div>
    <script>
      const status = document.getElementById('status');
      const video = document.getElementById('view');
      const VIEW_W = %%WIDTH%%;
      const VIEW_H = %%HEIGHT%%;
      let focused = false;
      let lastPoint = { x: VIEW_W / 2, y: VIEW_H / 2 };

      const wsUrl = (location.protocol === 'https:' ? 'wss://' : 'ws://') + location.host + '%%SIGNAL_PATH%%';
      const ws = new WebSocket(wsUrl);
      const pc = new RTCPeerConnection({ iceServers: [{urls: 'stun:stun.l.google.com:19302'}] });

      pc.ontrack = (ev) => {
        const stream = ev.streams && ev.streams[0] ? ev.streams[0] : new MediaStream([ev.track]);
        if (video.srcObject !== stream) {
          video.srcObject = stream;
          video.play().catch(() => {});
        }
      };

      pc.onicecandidate = (ev) => {
        if (ev.candidate) ws.send(JSON.stringify({ type: 'candidate', candidate: ev.candidate }));
      };

      ws.addEventListener('open', async () => {
        status.textContent = 'signaling';
        pc.addTransceiver('video', { direction: 'recvonly' });
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        ws.send(JSON.stringify({ type: 'offer', sdp: offer.sdp }));
      });

      ws.addEventListener('message', async (ev) => {
        const msg = JSON.parse(ev.data);
        if (msg.type === 'answer') {
          await pc.setRemoteDescription({ type: 'answer', sdp: msg.sdp });
          status.textContent = 'connected';
        } else if (msg.type === 'candidate') {
          if (msg.candidate) await pc.addIceCandidate(msg.candidate);
        } else if (msg.type === 'cursor') {
          updateCursor(msg.cursor);
        }
      });

      function mods(ev) {
        let m = 0;
        if (ev.altKey) m |= 1;
        if (ev.ctrlKey) m |= 2;
        if (ev.metaKey) m |= 4;
        if (ev.shiftKey) m |= 8;
        return m;
      }

      function mapCoords(ev) {
        const rect = video.getBoundingClientRect();
        const actualW = video.videoWidth || VIEW_W;
        const actualH = video.videoHeight || VIEW_H;
        const videoAspect = actualW / actualH;
        const rectAspect = rect.width / rect.height;
        let displayW = rect.width;
        let displayH = rect.height;
        let offsetX = 0;
        let offsetY = 0;
        if (rectAspect > videoAspect) {
          displayH = rect.height;
          displayW = rect.height * videoAspect;
          offsetX = (rect.width - displayW) / 2;
        } else {
          displayW = rect.width;
          displayH = rect.width / videoAspect;
          offsetY = (rect.height - displayH) / 2;
        }
        const x = (ev.clientX - rect.left - offsetX) * actualW / displayW;
        const y = (ev.clientY - rect.top - offsetY) * actualH / displayH;
        const mapped = { x: Math.max(0, Math.min(actualW, x)), y: Math.max(0, Math.min(actualH, y)) };
        lastPoint = mapped;
        return mapped;
      }

      function buttonName(button) {
        if (button === 1) return 'middle';
        if (button === 2) return 'right';
        return 'left';
      }

      function send(msg) {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify(msg));
        }
      }

      let lastCursor = 'default';
      let cursorTimer = null;
      function startCursorProbe() {
        if (cursorTimer) return;
        cursorTimer = setInterval(() => {
          if (!focused) return;
          send({ type: 'cursor_probe', x: lastPoint.x, y: lastPoint.y });
        }, 120);
      }

      function updateCursor(cursor) {
        if (!cursor || cursor === lastCursor) return;
        lastCursor = cursor;
        video.style.cursor = cursor;
      }

      video.addEventListener('mousedown', (ev) => {
        ev.preventDefault();
        focused = true;
        const { x, y } = mapCoords(ev);
        send({ type: 'mouse', event: 'down', x, y, button: buttonName(ev.button), buttons: ev.buttons, clickCount: ev.detail, modifiers: mods(ev) });
        send({ type: 'cursor_probe', x, y });
        startCursorProbe();
      });
      video.addEventListener('mouseup', (ev) => {
        ev.preventDefault();
        const { x, y } = mapCoords(ev);
        send({ type: 'mouse', event: 'up', x, y, button: buttonName(ev.button), buttons: ev.buttons, clickCount: ev.detail, modifiers: mods(ev) });
      });
      video.addEventListener('mousemove', (ev) => {
        const { x, y } = mapCoords(ev);
        send({ type: 'mouse', event: 'move', x, y, buttons: ev.buttons, modifiers: mods(ev) });
        send({ type: 'cursor_probe', x, y });
      });
      video.addEventListener('wheel', (ev) => {
        ev.preventDefault();
        const { x, y } = mapCoords(ev);
        send({ type: 'mouse', event: 'wheel', x, y, deltaX: ev.deltaX, deltaY: ev.deltaY, modifiers: mods(ev) });
      }, { passive: false });
      video.addEventListener('contextmenu', (ev) => ev.preventDefault());

      window.addEventListener('keydown', (ev) => {
        if (!focused) return;
        ev.preventDefault();
        const modifiers = mods(ev);
        const text = (modifiers === 0 && ev.key && ev.key.length === 1) ? ev.key : '';
        const raw = modifiers !== 0 || !text;
        send({ type: 'key', event: 'down', key: ev.key, code: ev.code, keyCode: ev.keyCode, text, modifiers, raw });
      });
      window.addEventListener('keyup', (ev) => {
        if (!focused) return;
        ev.preventDefault();
        const modifiers = mods(ev);
        send({ type: 'key', event: 'up', key: ev.key, code: ev.code, keyCode: ev.keyCode, modifiers });
      });
    </script>
  </body>
</html>
"#;

    TEMPLATE
        .replace("%%URL%%", &escape_html(&session.url))
        .replace("%%WIDTH%%", &session.viewport.width.to_string())
        .replace("%%HEIGHT%%", &session.viewport.height.to_string())
        .replace("%%SIGNAL_PATH%%", signal_path)
}

pub struct WebSessionHandle {
    info: WebSessionInfo,
    stream_token: String,
    runtime: Arc<Mutex<WebSessionRuntime>>,
    run_lock: Arc<Mutex<()>>,
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

    pub fn matches_stream_token(&self, token: &str) -> bool {
        self.stream_token == token
    }

    pub fn signal_path(&self) -> String {
        build_signal_path(&self.info.id, &self.stream_token)
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
        let stream_token = Uuid::new_v4().to_string();
        let viewport = req.viewport.clone().unwrap_or(WebSessionViewport {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        });
        let fps = req.fps.unwrap_or(DEFAULT_FPS);
        let display = self.next_display().await?;
        let worker_port = allocate_port()?;

        let stream_path = build_stream_path(&id, &stream_token);
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
            stream_token,
            runtime: Arc::new(Mutex::new(runtime)),
            run_lock: Arc::new(Mutex::new(())),
        });

        self.spawn_worker(&handle, &req, worker_port, &display)
            .await?;
        self.await_worker_ready(worker_port).await?;

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

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().context("spawning web session worker")?;
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

    async fn await_worker_ready(&self, port: u16) -> Result<()> {
        let url = format!("http://127.0.0.1:{port}/health");
        for _ in 0..40 {
            let resp = self.client.get(&url).send().await;
            if let Ok(resp) = resp {
                if resp.status().is_success() {
                    return Ok(());
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

pub async fn ensure_worker_bundle(
    data_root: &Path,
    node: &NodeRuntimeSpec,
) -> Result<WorkerBundle> {
    if let Ok(worker_path) = std::env::var("CTX_WEB_SESSION_WORKER") {
        let node_modules_path = std::env::var("CTX_WEB_SESSION_NODE_PATH")
            .context("CTX_WEB_SESSION_NODE_PATH required with CTX_WEB_SESSION_WORKER")?;
        let worker_path = PathBuf::from(worker_path);
        let node_modules_path = PathBuf::from(node_modules_path);
        if !worker_path.exists() {
            anyhow::bail!("web session worker not found at {}", worker_path.display());
        }
        if !node_modules_path.exists() {
            anyhow::bail!(
                "web session node_modules not found at {}",
                node_modules_path.display()
            );
        }
        return Ok(WorkerBundle {
            worker_path,
            node_modules_path,
        });
    }

    let version = worker_version()?;
    let root = data_root
        .join("tools")
        .join("web-session-worker")
        .join(&version);
    let bin_dir = root.join("bin");
    tokio::fs::create_dir_all(&bin_dir).await?;
    tokio::fs::write(root.join("package.json"), WORKER_PACKAGE_JSON).await?;
    tokio::fs::write(bin_dir.join("worker.mjs"), WORKER_SCRIPT).await?;

    let node_modules = root.join("node_modules");
    let deps_ready = node_modules.join("playwright").exists() && node_modules.join("wrtc").exists();
    if !deps_ready {
        let _guard = worker_install_lock().lock().await;
        let deps_ready =
            node_modules.join("playwright").exists() && node_modules.join("wrtc").exists();
        if !deps_ready {
            install_worker_deps(node, &root).await?;
        }
    }

    Ok(WorkerBundle {
        worker_path: bin_dir.join("worker.mjs"),
        node_modules_path: node_modules,
    })
}

async fn build_run_payload(
    handle: &WebSessionHandle,
    req: WebSessionRunRequest,
) -> Result<serde_json::Value> {
    let mut payload = serde_json::Map::new();
    if let Some(code) = req.code {
        payload.insert("code".to_string(), serde_json::Value::String(code));
    }
    if let Some(script_path) = req.script_path {
        let resolved = resolve_script_path(handle, &script_path).await?;
        payload.insert(
            "script_path".to_string(),
            serde_json::Value::String(resolved.to_string_lossy().to_string()),
        );
    }
    if let Some(timeout_ms) = req.timeout_ms {
        payload.insert(
            "timeout_ms".to_string(),
            serde_json::Value::Number(timeout_ms.into()),
        );
    }
    Ok(serde_json::Value::Object(payload))
}

async fn resolve_script_path(handle: &WebSessionHandle, script_path: &str) -> Result<PathBuf> {
    let candidate = PathBuf::from(script_path);
    let work_dir = handle.work_dir().await;
    if candidate.is_absolute() {
        if work_dir.is_none() {
            return Ok(candidate);
        }
        anyhow::bail!("script_path must be relative to work_dir");
    }
    let work_dir = work_dir.context("script_path requires work_dir")?;
    let joined = work_dir.join(candidate);
    let canonical = joined
        .canonicalize()
        .with_context(|| format!("failed to resolve script_path {script_path}"))?;
    if !canonical.starts_with(&work_dir) {
        anyhow::bail!("script_path must be inside work_dir");
    }
    Ok(canonical)
}

fn build_stream_path(id: &str, stream_token: &str) -> String {
    format!("/sessions/web/{id}/view?token={stream_token}")
}

fn build_signal_path(id: &str, stream_token: &str) -> String {
    format!("/sessions/web/{id}/signal?token={stream_token}")
}

fn allocate_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding port")?;
    let port = listener.local_addr()?.port();
    Ok(port)
}

fn worker_install_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn worker_version() -> Result<String> {
    static VERSION: OnceLock<String> = OnceLock::new();
    if let Some(version) = VERSION.get() {
        return Ok(version.clone());
    }
    let value: serde_json::Value =
        serde_json::from_str(WORKER_PACKAGE_JSON).context("parsing worker package.json")?;
    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .context("worker package.json missing version")?
        .to_string();
    let _ = VERSION.set(version.clone());
    Ok(version)
}

async fn install_worker_deps(node: &NodeRuntimeSpec, root: &Path) -> Result<()> {
    let mut cmd = if let Ok(pnpm) = which::which("pnpm") {
        let mut cmd = Command::new(pnpm);
        cmd.arg("install")
            .arg("--prod")
            .arg("--ignore-scripts")
            .arg("--reporter")
            .arg("silent")
            .current_dir(root);
        cmd
    } else {
        let mut cmd = Command::new(&node.node_bin);
        cmd.arg(&node.npm_cli_js)
            .arg("install")
            .arg("--omit=dev")
            .arg("--no-audit")
            .arg("--no-fund")
            .arg("--ignore-scripts")
            .current_dir(root)
            .env("npm_config_update_notifier", "false")
            .env("npm_config_ignore_scripts", "true");
        cmd
    };

    let output = cmd.output().await.context("running package install")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("package install failed: {}", stderr.trim());
    }
    Ok(())
}

async fn log_stream<R: tokio::io::AsyncRead + Unpin>(mut reader: R, label: &str) {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]);
                for line in chunk.split('\n') {
                    if !line.trim().is_empty() {
                        tracing::info!("[{label}] {line}");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_session_info() -> WebSessionInfo {
        let now = Utc::now();
        WebSessionInfo {
            id: "sess-1".to_string(),
            kind: "web".to_string(),
            session_id: None,
            worktree_id: None,
            status: WebSessionStatus::Running,
            created_at: now,
            updated_at: now,
            last_activity: now,
            url: "https://example.com".to_string(),
            viewport: WebSessionViewport {
                width: 1280,
                height: 720,
            },
            fps: 30,
            viewers: 0,
            stream_path: build_stream_path("sess-1", "stream-token"),
            stream_url: None,
        }
    }

    fn test_handle(work_dir: Option<PathBuf>) -> WebSessionHandle {
        let now = Utc::now();
        WebSessionHandle {
            info: test_session_info(),
            stream_token: "stream-token".to_string(),
            runtime: Arc::new(Mutex::new(WebSessionRuntime {
                status: WebSessionStatus::Running,
                updated_at: now,
                last_activity: now,
                viewers: 0,
                worker_port: 4321,
                child: None,
                work_dir,
            })),
            run_lock: Arc::new(Mutex::new(())),
        }
    }

    #[test]
    fn web_session_paths_embed_stream_token() {
        assert_eq!(
            build_stream_path("sess-1", "stream-token"),
            "/sessions/web/sess-1/view?token=stream-token"
        );
        assert_eq!(
            build_signal_path("sess-1", "stream-token"),
            "/sessions/web/sess-1/signal?token=stream-token"
        );
    }

    #[test]
    fn rendered_view_uses_tokenized_signal_path() {
        let html = render_web_session_view(
            &test_session_info(),
            "/sessions/web/sess-1/signal?token=stream-token",
        );
        assert!(html.contains("/sessions/web/sess-1/signal?token=stream-token"));
    }

    #[tokio::test]
    async fn closing_missing_session_returns_not_found_error() {
        let manager = WebSessionManager::new();
        let err = manager.close("missing-session").await.unwrap_err();
        assert!(format!("{err:#}").contains("session not found"));
    }

    #[tokio::test]
    async fn resolve_script_path_rejects_absolute_paths() {
        let dir = std::env::temp_dir().join(format!("ctx-web-session-test-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let handle = test_handle(Some(dir.clone()));
        let absolute = dir.join("script.js");
        tokio::fs::write(&absolute, "console.log('hi');")
            .await
            .unwrap();

        let err = resolve_script_path(&handle, absolute.to_str().unwrap())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("relative to work_dir"));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn resolve_script_path_allows_absolute_paths_without_work_dir() {
        let absolute =
            std::env::temp_dir().join(format!("ctx-web-session-test-{}.js", Uuid::new_v4()));
        let handle = test_handle(None);

        let resolved = resolve_script_path(&handle, absolute.to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(resolved, absolute);
    }

    #[tokio::test]
    async fn resolve_script_path_accepts_relative_paths_inside_work_dir() {
        let dir = std::env::temp_dir().join(format!("ctx-web-session-test-{}", Uuid::new_v4()));
        let nested = dir.join("scripts");
        tokio::fs::create_dir_all(&nested).await.unwrap();
        let script = nested.join("script.js");
        tokio::fs::write(&script, "console.log('hi');")
            .await
            .unwrap();
        let handle = test_handle(Some(dir.clone()));

        let resolved = resolve_script_path(&handle, "scripts/script.js")
            .await
            .unwrap();
        assert_eq!(resolved, script.canonicalize().unwrap());
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
