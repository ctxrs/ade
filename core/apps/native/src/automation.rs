use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use gpui::{App, Context, Window, WindowHandle};
use image::{ColorType, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::timeout as tokio_timeout;

use crate::app::ShellView;

#[derive(Clone, Debug, Default)]
pub struct AutomationConfig {
    pub addr: Option<SocketAddr>,
    pub screenshot_dir: Option<PathBuf>,
    pub fixture: Option<String>,
}

impl AutomationConfig {
    fn should_start(&self) -> bool {
        self.addr.is_some() || self.screenshot_dir.is_some() || self.fixture.is_some()
    }

    fn effective_addr(&self) -> Option<SocketAddr> {
        if let Some(addr) = self.addr {
            return Some(addr);
        }
        if self.should_start() {
            return Some(SocketAddr::from(([127, 0, 0, 1], 0)));
        }
        None
    }
}

pub fn start(app: &mut App, window: WindowHandle<ShellView>, config: AutomationConfig) {
    let Some(addr) = config.effective_addr() else {
        return;
    };
    let (ready_tx, ready_rx) = watch::channel(false);
    let (command_tx, command_rx) = mpsc::channel(32);
    let state = Arc::new(AutomationState {
        ready_tx,
        ready_rx,
        command_tx,
        config: config.clone(),
    });

    app.spawn(move |cx: &mut gpui::AsyncApp| {
        let cx = cx.clone();
        async move {
            run_command_loop(cx, window, command_rx).await;
        }
    })
    .detach();

    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            eprintln!("ctx-native: automation using existing tokio runtime");
            handle.spawn(run_http_server(addr, Arc::clone(&state)));
        }
        Err(err) => {
            eprintln!(
                "ctx-native: automation runtime unavailable ({err}); starting dedicated runtime thread"
            );
            let state = Arc::clone(&state);
            std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(run_http_server(addr, state));
                    }
                    Err(err) => {
                        eprintln!("ctx-native: automation runtime build failed: {err}");
                    }
                }
            });
        }
    }
    let _ = state.ready_tx.send(true);
}

struct AutomationState {
    ready_tx: watch::Sender<bool>,
    ready_rx: watch::Receiver<bool>,
    command_tx: mpsc::Sender<AutomationRequest>,
    config: AutomationConfig,
}

async fn run_http_server(addr: SocketAddr, state: Arc<AutomationState>) {
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("ctx-native: automation bind failed: {err}");
            return;
        }
    };
    match listener.local_addr() {
        Ok(local_addr) => {
            eprintln!("ctx-native: automation listening on {local_addr}");
        }
        Err(err) => {
            eprintln!("ctx-native: automation local addr failed: {err}");
        }
    };

    let app = Router::new()
        .route("/ready", get(ready_handler))
        .route("/focus", post(focus_handler))
        .route("/screenshot", post(screenshot_handler))
        .route("/exit", post(exit_handler))
        .with_state(state);

    if let Err(err) = axum::serve(listener, app).await {
        eprintln!("ctx-native: automation server error: {err}");
    }
}

#[derive(Deserialize)]
struct ReadyQuery {
    timeout_ms: Option<u64>,
}

#[derive(Serialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    error: Option<String>,
}

fn ok<T: Serialize>(result: T) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            result: Some(result),
            error: None,
        }),
    )
}

fn err<T: Serialize>(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<ApiResponse<T>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            result: None,
            error: Some(message.into()),
        }),
    )
}

async fn ready_handler(
    State(state): State<Arc<AutomationState>>,
    Query(query): Query<ReadyQuery>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let timeout_ms = query.timeout_ms.unwrap_or(30_000);
    let mut ready_rx = state.ready_rx.clone();
    if *ready_rx.borrow() {
        return ok(json!({ "ready": true }));
    }

    let ready = tokio_timeout(Duration::from_millis(timeout_ms), ready_rx.changed())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|_| *ready_rx.borrow())
        .unwrap_or(false);

    if ready {
        ok(json!({ "ready": true }))
    } else {
        err(StatusCode::REQUEST_TIMEOUT, "timeout waiting for native app readiness")
    }
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum FocusTarget {
    Main,
    Composer,
    ComposerAttachments,
    SessionsPane,
    DiffPane,
    ArtifactsPane,
    TerminalPanel,
}

impl FocusTarget {
    fn as_str(&self) -> &'static str {
        match self {
            FocusTarget::Main => "main",
            FocusTarget::Composer => "composer",
            FocusTarget::ComposerAttachments => "composer_attachments",
            FocusTarget::SessionsPane => "sessions_pane",
            FocusTarget::DiffPane => "diff_pane",
            FocusTarget::ArtifactsPane => "artifacts_pane",
            FocusTarget::TerminalPanel => "terminal_panel",
        }
    }
}

#[derive(Deserialize)]
struct FocusRequest {
    target: FocusTarget,
}

async fn focus_handler(
    State(state): State<Arc<AutomationState>>,
    Json(request): Json<FocusRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let response = dispatch_command(
        &state,
        AutomationCommand::Focus {
            target: request.target,
        },
    )
    .await;
    match response {
        Ok(result) => ok(result),
        Err(message) => err(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

#[derive(Deserialize)]
struct ScreenshotRequest {
    name: Option<String>,
    path: Option<PathBuf>,
}

async fn screenshot_handler(
    State(state): State<Arc<AutomationState>>,
    Json(request): Json<ScreenshotRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let path = match resolve_screenshot_path(request, &state.config) {
        Ok(path) => path,
        Err(message) => return err(StatusCode::BAD_REQUEST, message),
    };

    let response =
        dispatch_command(&state, AutomationCommand::Screenshot { path }).await;
    match response {
        Ok(result) => ok(result),
        Err(message) => err(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

async fn exit_handler(
    State(state): State<Arc<AutomationState>>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    eprintln!("ctx-native: exit request");
    let response = dispatch_command(&state, AutomationCommand::Exit).await;
    match response {
        Ok(result) => ok(result),
        Err(message) => err(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

fn resolve_screenshot_path(
    request: ScreenshotRequest,
    config: &AutomationConfig,
) -> Result<PathBuf, String> {
    if let Some(path) = request.path {
        if path.is_relative() {
            if let Some(dir) = &config.screenshot_dir {
                return Ok(dir.join(path));
            }
            return Err(
                "relative screenshot path requires --screenshot-dir".to_string(),
            );
        }
        return Ok(path);
    }

    let name = request
        .name
        .ok_or_else(|| "screenshot requires name or path".to_string())?;
    if name.is_empty() {
        return Err("screenshot name must not be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("screenshot name must not include path separators".to_string());
    }

    let dir = config
        .screenshot_dir
        .as_ref()
        .ok_or_else(|| "screenshot name requires --screenshot-dir".to_string())?;
    let file_name = if name.ends_with(".png") {
        name
    } else {
        format!("{name}.png")
    };
    Ok(dir.join(file_name))
}

async fn dispatch_command(
    state: &AutomationState,
    command: AutomationCommand,
) -> Result<Value, String> {
    let (respond_to, response_rx) = oneshot::channel();
    state
        .command_tx
        .send(AutomationRequest {
            command,
            respond_to,
        })
        .await
        .map_err(|_| "automation command channel closed".to_string())?;

    tokio_timeout(Duration::from_secs(10), response_rx)
        .await
        .map_err(|_| "automation command timed out".to_string())?
        .map_err(|_| "automation command dropped".to_string())?
}

struct AutomationRequest {
    command: AutomationCommand,
    respond_to: oneshot::Sender<Result<Value, String>>,
}

enum AutomationCommand {
    Focus { target: FocusTarget },
    Screenshot { path: PathBuf },
    Exit,
}

async fn run_command_loop(
    cx: gpui::AsyncApp,
    window: WindowHandle<ShellView>,
    mut command_rx: mpsc::Receiver<AutomationRequest>,
) {
    while let Some(request) = command_rx.recv().await {
        let command = request.command;
        let response = match command {
            AutomationCommand::Focus { target } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |view, window, cx| {
                        apply_focus_target(view, window, cx, target);
                        Ok(json!({ "target": target.as_str() }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::Screenshot { path } => capture_window(&cx, &window, path)
                .await
                .map(|path| json!({ "path": path.to_string_lossy() })),
            AutomationCommand::Exit => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |_, _, cx| {
                        cx.quit();
                        Ok(json!({ "quitting": true }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
        };

        let _ = request.respond_to.send(response);
    }
}

fn apply_focus_target(
    view: &mut ShellView,
    window: &mut Window,
    cx: &mut Context<ShellView>,
    target: FocusTarget,
) {
    view.show_sessions_pane = false;
    view.show_diff_pane = false;
    view.show_artifacts_pane = false;
    view.show_terminal_panel = false;

    match target {
        FocusTarget::Main => {}
        FocusTarget::Composer => {
            view.composer_focus.focus(window, cx);
        }
        FocusTarget::ComposerAttachments => {
            view.composer_attachment_focus.focus(window, cx);
        }
        FocusTarget::SessionsPane => {
            view.show_sessions_pane = true;
        }
        FocusTarget::DiffPane => {
            view.show_diff_pane = true;
        }
        FocusTarget::ArtifactsPane => {
            view.show_artifacts_pane = true;
        }
        FocusTarget::TerminalPanel => {
            view.show_terminal_panel = true;
        }
    }

    cx.notify();
}

async fn capture_window(
    cx: &gpui::AsyncApp,
    window: &WindowHandle<ShellView>,
    path: PathBuf,
) -> Result<PathBuf, String> {
    let image = {
        let mut cx_window = cx.clone();
        window
            .update(&mut cx_window, |_, window, _cx| window.render_to_image())
            .map_err(|err| err.to_string())?
            .map_err(|err| err.to_string())?
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create screenshot dir: {err}"))?;
    }

    image::save_buffer_with_format(
        &path,
        image.as_raw(),
        image.width(),
        image.height(),
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .map_err(|err| format!("failed to write screenshot: {err}"))?;

    Ok(path)
}
