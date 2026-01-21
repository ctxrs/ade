use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::future::{Either, select};
use futures_util::{FutureExt, pin_mut};
use gpui::{
    App, AppContext, ClickEvent, Context, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PlatformInput, ScrollDelta, ScrollStrategy, ScrollWheelEvent,
    TouchPhase, Window, WindowHandle, px, size, point,
};
use gpui_component::{Root, input::set_force_cursor_visible};
use image::{ColorType, ImageFormat};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, json, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::timeout;
use uuid::Uuid;

use crate::app::{ComposerMenuId, ShellView};
use crate::app::state::RightPaneMode;
use crate::app::state::ShellRoute;
use crate::automation_tree;
use ctx_core::ids::TaskId;

#[derive(Clone, Debug, Default)]
pub struct AutomationConfig {
    pub addr: Option<SocketAddr>,
    pub screenshot_dir: Option<PathBuf>,
    #[allow(dead_code)]
    pub fixture: Option<String>,
}

impl AutomationConfig {
    fn effective_addr(&self) -> Option<SocketAddr> {
        self.addr
    }
}

pub fn start(app: &mut App, window: WindowHandle<Root>, config: AutomationConfig) {
    let Some(addr) = config.effective_addr() else {
        return;
    };
    set_force_cursor_visible(true);
    let (ready_tx, ready_rx) = watch::channel(false);
    let (command_tx, command_rx) = mpsc::channel(32);
    let (render_tx, render_rx) = watch::channel(0u64);
    let (draw_tx, draw_rx) = watch::channel(0u64);
    let (input_render_tx, input_render_rx) = watch::channel(0u64);
    let state = Arc::new(AutomationState {
        ready_tx,
        ready_rx,
        command_tx,
        config: config.clone(),
        render_tx,
        render_rx,
        draw_tx,
        draw_rx,
        input_render_tx,
        input_render_rx,
        render_counter: AtomicU64::new(0),
        last_input_frame: AtomicU64::new(0),
        input_epoch: AtomicU64::new(0),
        rendered_input_epoch: AtomicU64::new(0),
        pending_input_paint_epoch: AtomicU64::new(0),
        draw_epoch: AtomicU64::new(0),
        pending_draw_epoch: AtomicU64::new(0),
    });

    let command_state = Arc::clone(&state);
    app.spawn(move |cx: &mut gpui::AsyncApp| {
        let cx = cx.clone();
        async move {
            run_command_loop(cx, window, command_rx, command_state).await;
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
    let frame_state = Arc::clone(&state);
    let _ = window.update(app, |_, window, _cx| {
        start_frame_tracking(window, frame_state);
        window.refresh();
        ()
    });
    let _ = state.ready_tx.send(true);
    start_render_pump(app, window, Arc::clone(&state));
}

struct AutomationState {
    ready_tx: watch::Sender<bool>,
    ready_rx: watch::Receiver<bool>,
    command_tx: mpsc::Sender<AutomationRequest>,
    config: AutomationConfig,
    render_tx: watch::Sender<u64>,
    render_rx: watch::Receiver<u64>,
    draw_tx: watch::Sender<u64>,
    draw_rx: watch::Receiver<u64>,
    input_render_tx: watch::Sender<u64>,
    input_render_rx: watch::Receiver<u64>,
    render_counter: AtomicU64,
    last_input_frame: AtomicU64,
    input_epoch: AtomicU64,
    rendered_input_epoch: AtomicU64,
    pending_input_paint_epoch: AtomicU64,
    draw_epoch: AtomicU64,
    pending_draw_epoch: AtomicU64,
}

fn start_frame_tracking(window: &mut Window, state: Arc<AutomationState>) {
    window.on_next_frame(move |window, _cx| {
        record_render(&state);
        start_frame_tracking(window, state);
    });
}

fn record_render(state: &AutomationState) -> u64 {
    let next = state.render_counter.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = state.render_tx.send(next);
    if next == 1 {
        let _ = state.ready_tx.send(true);
    }
    next
}

fn record_draw(state: &AutomationState) -> u64 {
    let next = state.draw_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = state.draw_tx.send(next);
    next
}

fn record_input_paint_epoch(root: &Root, cx: &App, state: &AutomationState) {
    let Some(paint_epoch) = active_composer_paint_epoch(root, cx) else {
        return;
    };
    let rendered_epoch = state.rendered_input_epoch.load(Ordering::SeqCst);
    if paint_epoch > rendered_epoch {
        state
            .rendered_input_epoch
            .store(paint_epoch, Ordering::SeqCst);
        let _ = state.input_render_tx.send(paint_epoch);
    }
}

fn active_composer_paint_epoch(root: &Root, cx: &App) -> Option<u64> {
    let shell_view = root.view().clone().downcast::<ShellView>().ok()?;
    Some(
        shell_view
            .read(cx)
            .active_composer_input()
            .read(cx)
            .paint_epoch(),
    )
}

async fn render_frame(
    cx: &gpui::AsyncApp,
    window: &WindowHandle<Root>,
    state: &Arc<AutomationState>,
) -> Result<Value, String> {
    let (render_tx, render_rx) = oneshot::channel();
    let mut cx_window = cx.clone();
    window
        .update(&mut cx_window, |_, window, cx| {
            window.refresh();
            let state = Arc::clone(state);
            window.defer(cx, move |window, cx| {
                let result = (|| {
                    let clear = window.draw(cx);
                    clear.clear();
                    record_draw(&state);
                    if let Some(root) = window.root::<Root>().and_then(|root| root) {
                        let root_ref = root.read(cx);
                        record_input_paint_epoch(&*root_ref, cx, &state);
                    }
                    Ok::<_, String>(())
                })();
                let _ = render_tx.send(result);
            });
        })
        .map_err(|err| err.to_string())?;

    let timer = cx.background_executor().timer(Duration::from_millis(2_000));
    let render_rx = render_rx.fuse();
    let timer = timer.fuse();
    pin_mut!(render_rx, timer);

    match select(render_rx, timer).await {
        Either::Left((result, _timer)) => {
            result
                .map_err(|_| "render frame canceled".to_string())?
                .map_err(|err| err.to_string())?;
        }
        Either::Right((_result, _render_rx)) => {
            return Err("timeout waiting for render frame".to_string());
        }
    }
    Ok(json!({ "rendered": true }))
}

fn start_render_pump(app: &mut App, window: WindowHandle<Root>, state: Arc<AutomationState>) {
    app.spawn(move |cx: &mut gpui::AsyncApp| {
        let mut cx = cx.clone();
        async move {
            let mut last_render = state.render_counter.load(Ordering::SeqCst);
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(33))
                    .await;
                let current = state.render_counter.load(Ordering::SeqCst);
                if current == last_render {
                    if window
                        .update(&mut cx, |_, window, _cx| {
                            window.refresh();
                        })
                        .is_err()
                    {
                        break;
                    }
                } else {
                    last_render = current;
                }
            }
        }
    })
    .detach();
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
	    .route("/route", post(route_handler))
	    .route("/select_workspace", post(select_workspace_handler))
	    .route("/screenshot", post(screenshot_handler))
	    .route("/wait", post(wait_handler))
	    .route("/exit", post(exit_handler))
	    .route("/ws", get(ws_handler))
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
    match wait_ready(&state, query.timeout_ms.unwrap_or(30_000)).await {
        Ok(true) => ok(json!({ "ready": true })),
        Ok(false) => err(StatusCode::REQUEST_TIMEOUT, "timeout waiting for native app readiness"),
        Err(message) => err(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum FocusTarget {
    NewTask,
    Main,
    ArchivedTasks,
    ArchivedTasksClose,
    TaskSearch,
    SettingsSearch,
    Composer,
    ComposerAttachments,
    ComposerProviderMenu,
    ComposerModelMenu,
    DismissMenus,
    SessionsPane,
    DiffPane,
    ArtifactsPane,
    TerminalPanel,
}

impl FocusTarget {
    fn as_str(&self) -> &'static str {
        match self {
            FocusTarget::NewTask => "new_task",
            FocusTarget::Main => "main",
            FocusTarget::ArchivedTasks => "archived_tasks",
            FocusTarget::ArchivedTasksClose => "archived_tasks_close",
            FocusTarget::TaskSearch => "task_search",
            FocusTarget::SettingsSearch => "settings_search",
            FocusTarget::Composer => "composer",
            FocusTarget::ComposerAttachments => "composer_attachments",
            FocusTarget::ComposerProviderMenu => "composer_provider_menu",
            FocusTarget::ComposerModelMenu => "composer_model_menu",
            FocusTarget::DismissMenus => "dismiss_menus",
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

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum RouteTarget {
    Workbench,
    Workspaces,
    Settings,
    Providers,
    Diagnostics,
    AppSettings,
}

impl RouteTarget {
    fn as_str(&self) -> &'static str {
        match self {
            RouteTarget::Workbench => "workbench",
            RouteTarget::Workspaces => "workspaces",
            RouteTarget::Settings => "settings",
            RouteTarget::Providers => "providers",
            RouteTarget::Diagnostics => "diagnostics",
            RouteTarget::AppSettings => "app_settings",
        }
    }

    fn to_shell_route(&self) -> ShellRoute {
        match self {
            RouteTarget::Workbench => ShellRoute::Workbench,
            RouteTarget::Workspaces => ShellRoute::Workspaces,
            RouteTarget::Settings => ShellRoute::Settings,
            RouteTarget::Providers => ShellRoute::Providers,
            RouteTarget::Diagnostics => ShellRoute::Diagnostics,
            RouteTarget::AppSettings => ShellRoute::AppSettings,
        }
    }
}

#[derive(Deserialize)]
struct RouteRequest {
    route: RouteTarget,
}

async fn route_handler(
    State(state): State<Arc<AutomationState>>,
    Json(request): Json<RouteRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let route = request.route.to_shell_route();
    let response = dispatch_command(&state, AutomationCommand::SetRoute { route }).await;
    match response {
        Ok(_result) => ok(json!({ "route": request.route.as_str() })),
        Err(message) => err(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

#[derive(Deserialize)]
struct SelectWorkspaceRequest {
    index: usize,
}

async fn select_workspace_handler(
    State(state): State<Arc<AutomationState>>,
    Json(request): Json<SelectWorkspaceRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let response =
        dispatch_command(&state, AutomationCommand::SelectWorkspace { index: request.index }).await;
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

const SCREENSHOT_INPUT_WAIT_MS: u64 = 2_000;

async fn screenshot_handler(
    State(state): State<Arc<AutomationState>>,
    Json(request): Json<ScreenshotRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let path = match resolve_screenshot_path(request, &state.config) {
        Ok(path) => path,
        Err(message) => return err(StatusCode::BAD_REQUEST, message),
    };

    if let Err(rpc_err) = wait_for_input_rendered_if_needed(&state, SCREENSHOT_INPUT_WAIT_MS).await
    {
        return err(StatusCode::INTERNAL_SERVER_ERROR, rpc_err.message);
    }

    let response =
        dispatch_command(&state, AutomationCommand::Screenshot { path }).await;
    match response {
        Ok(result) => ok(result),
        Err(message) => err(StatusCode::INTERNAL_SERVER_ERROR, message),
    }
}

async fn wait_handler(
    State(state): State<Arc<AutomationState>>,
    Json(request): Json<WaitIdleParams>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    match wait_for_idle(&state, request).await {
        Ok(result) => ok(result),
        Err(rpc_err) => err(StatusCode::INTERNAL_SERVER_ERROR, rpc_err.message),
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

    timeout(Duration::from_secs(10), response_rx)
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
    RenderFrame,
    Screenshot { path: PathBuf },
    Resize { width: f32, height: f32 },
    Click { x: f32, y: f32, button: MouseButton },
    Scroll {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
        precise: bool,
    },
    ComposerStatus,
    ThreadStatus,
    ThreadDebug,
    SelectSession { index: usize },
    SelectTask { task_id: TaskId },
    SelectWorkspace { index: usize },
    Type { text: String },
    KeyPress { keystroke: Keystroke },
    SetRoute { route: ShellRoute },
    Exit,
}

async fn run_command_loop(
    cx: gpui::AsyncApp,
    window: WindowHandle<Root>,
    mut command_rx: mpsc::Receiver<AutomationRequest>,
    state: Arc<AutomationState>,
) {
    let any_window = gpui::AnyWindowHandle::from(window);
    while let Some(request) = command_rx.recv().await {
        let command = request.command;
        let response = match command {
            AutomationCommand::Focus { target } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, window, cx| {
                        with_shell_view(root, cx, |view, cx| {
                            apply_focus_target(view, window, cx, target);
                        })?;
                        if matches!(
                            target,
                            FocusTarget::NewTask
                                | FocusTarget::Composer
                                | FocusTarget::ComposerAttachments
                                | FocusTarget::ComposerProviderMenu
                                | FocusTarget::ComposerModelMenu
                                | FocusTarget::TaskSearch
                        ) {
                            mark_input_paint_needed(&state);
                        }
                        mark_input(&state);
                        Ok(json!({ "target": target.as_str() }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::SetRoute { route } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, window, cx| {
                        with_shell_view(root, cx, |view, cx| {
                            view.set_route(route, cx);
                        })?;
                        window.refresh();
                        if matches!(route, ShellRoute::Workbench) {
                            mark_input_paint_needed(&state);
                        }
                        mark_input(&state);
                        Ok(json!({ "route": format!("{:?}", route) }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }

            AutomationCommand::RenderFrame => {
                render_frame(&cx, &window, &state).await
            }
            AutomationCommand::ComposerStatus => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, _window, cx| {
                        let status = with_shell_view(root, cx, |view, cx| {
                            let input = view.active_composer_input().read(cx);
                            let text = input.value().to_string();
                            let paint_epoch = input.paint_epoch();
                            json!({
                                "focused": view.composer_has_focus,
                                "new_task_mode": view.new_task_mode,
                                "text": text,
                                "paint_epoch": paint_epoch,
                            })
                        })?;
                        Ok(status)
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::ThreadStatus => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, _window, cx| {
                        let status = with_shell_view(root, cx, |view, _cx| {
                            view.thread_render_status()
                        })?;
                        Ok(status)
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::ThreadDebug => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, _window, cx| {
                        let status = with_shell_view(root, cx, |view, _cx| {
                            view.thread_render_debug()
                        })?;
                        Ok(status)
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::Screenshot { path } => {
                capture_window(&cx, &window, &state, path)
                .await
                .map(|path| json!({ "path": path.to_string_lossy() }))
            }
            AutomationCommand::Resize { width, height } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |_, window, _cx| {
                        if width <= 0.0 || height <= 0.0 {
                            return Err("window size must be positive".to_string());
                        }
                        window.resize(size(px(width), px(height)));
                        mark_input(&state);
                        Ok(json!({ "width": width, "height": height }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::Click { x, y, button } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |_, window, cx| {
                        dispatch_click(window, cx, x, y, button);
                        window.refresh();
                        mark_input(&state);
                        Ok(json!({ "x": x, "y": y }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::Scroll {
                x,
                y,
                delta_x,
                delta_y,
                precise,
            } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |_, window, cx| {
                        dispatch_scroll(window, cx, x, y, delta_x, delta_y, precise);
                        window.refresh();
                        mark_input(&state);
                        Ok(json!({
                            "x": x,
                            "y": y,
                            "delta_x": delta_x,
                            "delta_y": delta_y,
                            "precise": precise,
                        }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::SelectSession { index } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, window, cx| {
                        with_shell_view(root, cx, |view, cx| {
                            view.select_session(index, window, cx);
                        })?;
                        mark_input(&state);
                        Ok(json!({ "index": index }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::SelectTask { task_id } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, window, cx| {
                        with_shell_view(root, cx, |view, cx| {
                            view.focus_task(task_id, window, cx);
                        })?;
                        mark_input(&state);
                        Ok(json!({ "task_id": task_id.0.to_string() }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::SelectWorkspace { index } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, window, cx| {
                        with_shell_view(root, cx, |view, cx| {
                            view.select_workspace(index, cx);
                            view.set_route(ShellRoute::Workbench, cx);
                        })?;
                        window.refresh();
                        mark_input_paint_needed(&state);
                        mark_input(&state);
                        Ok(json!({ "index": index }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::Type { text } => {
                let mut cx = cx.clone();
                window
                    .update(&mut cx, |root, window, cx| {
                        with_shell_view(root, cx, |view, cx| {
                            let input = view.active_composer_input();
                            input.update(cx, |state, cx| {
                                let next = format!("{}{}", state.value(), text);
                                state.set_value(next, window, cx);
                            });
                        })?;
                        window.refresh();
                        mark_input_paint_needed(&state);
                        mark_input(&state);
                        Ok(json!({ "text": text, "chars": text.chars().count() }))
                    })
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
            }
            AutomationCommand::KeyPress { keystroke } => {
                let mut cx = cx.clone();
                cx.update_window(any_window, |_, window, cx| {
                    window.dispatch_keystroke(keystroke.clone(), cx);
                    window.refresh();
                    mark_input_paint_needed(&state);
                    mark_input(&state);
                    json!({ "key": keystroke.to_string() })
                })
                .map_err(|err| err.to_string())
            }
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

fn mark_input(state: &AutomationState) {
    let current = state.render_counter.load(Ordering::SeqCst);
    state.last_input_frame.store(current, Ordering::SeqCst);
    state.input_epoch.fetch_add(1, Ordering::SeqCst);
    mark_draw_needed(state);
}

fn mark_draw_needed(state: &AutomationState) {
    let current_draw = state.draw_epoch.load(Ordering::SeqCst);
    state
        .pending_draw_epoch
        .store(current_draw.saturating_add(1), Ordering::SeqCst);
}

fn mark_input_paint_needed(state: &AutomationState) {
    let current_paint = state.rendered_input_epoch.load(Ordering::SeqCst);
    state
        .pending_input_paint_epoch
        .store(current_paint.saturating_add(1), Ordering::SeqCst);
}

fn dispatch_click(window: &mut Window, cx: &mut App, x: f32, y: f32, button: MouseButton) {
    let position = point(px(x), px(y));
    let modifiers = Modifiers::none();
    window.dispatch_platform_input(
        PlatformInput::MouseMove(MouseMoveEvent {
            position,
            modifiers,
            pressed_button: None,
        }),
        cx,
    );
    window.dispatch_platform_input(
        PlatformInput::MouseDown(MouseDownEvent {
            button,
            position,
            modifiers,
            click_count: 1,
            first_mouse: true,
        }),
        cx,
    );
    window.dispatch_platform_input(
        PlatformInput::MouseUp(MouseUpEvent {
            button,
            position,
            modifiers,
            click_count: 1,
        }),
        cx,
    );
}

fn dispatch_scroll(
    window: &mut Window,
    cx: &mut App,
    x: f32,
    y: f32,
    delta_x: f32,
    delta_y: f32,
    precise: bool,
) {
    let position = point(px(x), px(y));
    let modifiers = Modifiers::none();
    let delta = if precise {
        ScrollDelta::Pixels(point(px(delta_x), px(delta_y)))
    } else {
        ScrollDelta::Lines(point(delta_x, delta_y))
    };
    window.dispatch_platform_input(
        PlatformInput::ScrollWheel(ScrollWheelEvent {
            position,
            delta,
            modifiers,
            touch_phase: TouchPhase::Moved,
        }),
        cx,
    );
}

fn apply_focus_target(
    view: &mut ShellView,
    window: &mut Window,
    cx: &mut Context<ShellView>,
    target: FocusTarget,
) {
    view.right_pane = None;
    view.show_terminal_panel = false;

    match target {
        FocusTarget::NewTask => {
            view.focus_new_task(&ClickEvent::default(), window, cx);
        }
        FocusTarget::Main => {}
        FocusTarget::ArchivedTasks => {
            view.set_archived_collapsed(false, cx);
            view.ensure_archived_loaded(cx);
            view.task_list_scroll_handle
                .scroll_to_item(0, ScrollStrategy::Top);
        }
        FocusTarget::ArchivedTasksClose => {
            view.set_archived_collapsed(true, cx);
        }
        FocusTarget::TaskSearch => {
            view.composer_has_focus = false;
            view.task_search_input
                .update(cx, |state, cx| state.focus(window, cx));
        }
        FocusTarget::SettingsSearch => {
            view.set_route(ShellRoute::Settings, cx);
            cx.update_entity(&view.settings_state, |state, cx| {
                state.focus_search(window, cx);
            });
        }
        FocusTarget::Composer => {
            view.focus_composer(&ClickEvent::default(), window, cx);
        }
        FocusTarget::ComposerAttachments => {
            view.focus_composer(&ClickEvent::default(), window, cx);
        }
        FocusTarget::ComposerProviderMenu => {
            if view.composer_open_menu != Some(ComposerMenuId::Harness) {
                view.toggle_menu(ComposerMenuId::Harness, window, cx);
            }
            view.focus_composer(&ClickEvent::default(), window, cx);
        }
        FocusTarget::ComposerModelMenu => {
            if view.composer_open_menu != Some(ComposerMenuId::Model) {
                view.toggle_menu(ComposerMenuId::Model, window, cx);
            }
            view.focus_composer(&ClickEvent::default(), window, cx);
        }
        FocusTarget::DismissMenus => {
            view.close_menu(cx);
        }
        FocusTarget::SessionsPane => {
            view.right_pane = Some(RightPaneMode::Sessions);
        }
        FocusTarget::DiffPane => {
            view.right_pane = Some(RightPaneMode::Diff);
        }
        FocusTarget::ArtifactsPane => {
            view.right_pane = Some(RightPaneMode::Artifacts);
        }
        FocusTarget::TerminalPanel => {
            view.show_terminal_panel = true;
        }
    }

    cx.notify();
}

fn with_shell_view<R, C: AppContext>(
    root: &mut Root,
    cx: &mut C,
    f: impl FnOnce(&mut ShellView, &mut Context<ShellView>) -> R,
) -> Result<R, String> {
    let shell_view = root
        .view()
        .clone()
        .downcast::<ShellView>()
        .map_err(|_| "automation expected ShellView root".to_string())?;
    Ok(shell_view.update(cx, f))
}


async fn capture_window(
    cx: &gpui::AsyncApp,
    window: &WindowHandle<Root>,
    state: &Arc<AutomationState>,
    path: PathBuf,
) -> Result<PathBuf, String> {
    let (image_tx, image_rx) = oneshot::channel();
    let mut cx_window = cx.clone();
    window
        .update(&mut cx_window, |_, window, cx| {
            window.refresh();
            let state = Arc::clone(state);
            window.defer(cx, move |window, cx| {
                let result = (|| {
                    let clear = window.draw(cx);
                    clear.clear();
                    record_draw(&state);
                    let image = window.render_to_image().map_err(|err| err.to_string());
                    if let Some(root) = window.root::<Root>().and_then(|root| root) {
                        let root_ref = root.read(cx);
                        record_input_paint_epoch(&*root_ref, cx, &state);
                    }
                    image
                })();
                let _ = image_tx.send(result);
            });
        })
        .map_err(|err| err.to_string())?;

    let timer = cx.background_executor().timer(Duration::from_millis(2_000));
    let image_rx = image_rx.fuse();
    let timer = timer.fuse();
    pin_mut!(image_rx, timer);

    let image = match select(image_rx, timer).await {
        Either::Left((result, _timer)) => result
            .map_err(|_| "screenshot capture canceled".to_string())?
            .map_err(|err| err.to_string())?,
        Either::Right((_result, _image_rx)) => {
            return Err("timeout waiting for screenshot capture".to_string());
        }
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

async fn wait_ready(
    state: &AutomationState,
    timeout_ms: u64,
) -> Result<bool, String> {
    let mut ready_rx = state.ready_rx.clone();
    if *ready_rx.borrow() {
        return Ok(true);
    }

    let ready = timeout(Duration::from_millis(timeout_ms), ready_rx.changed())
        .await
        .ok()
        .and_then(Result::ok)
        .map(|_| *ready_rx.borrow())
        .unwrap_or(false);

    Ok(ready)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AutomationState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AutomationState>) {
    while let Some(message) = socket.recv().await {
        let message = match message {
            Ok(message) => message,
            Err(_) => break,
        };

        match message {
            Message::Text(text) => {
                if let Some(response) = handle_rpc_text(&state, &text).await {
                    if let Ok(payload) = serde_json::to_string(&response) {
                        if socket.send(Message::Text(payload)).await.is_err() {
                            break;
                        }
                    }
                }
            }
            Message::Binary(_) => {}
            Message::Ping(payload) => {
                let _ = socket.send(Message::Pong(payload)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

#[derive(Debug, Deserialize)]
struct RpcRequest {
    jsonrpc: Option<String>,
    method: Option<String>,
    params: Option<Value>,
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(-32600, message)
    }

    fn method_not_found(message: impl Into<String>) -> Self {
        Self::new(-32601, message)
    }

    fn parse_error(message: impl Into<String>) -> Self {
        Self::new(-32700, message)
    }

    fn server_error(message: impl Into<String>) -> Self {
        Self::new(-32000, message)
    }
}

async fn handle_rpc_text(state: &AutomationState, text: &str) -> Option<Value> {
    let payload: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            return Some(rpc_error_value(Value::Null, RpcError::parse_error(format!(
                "parse error: {err}"
            ))));
        }
    };

    match payload {
        Value::Array(items) => handle_rpc_batch(state, items).await,
        Value::Object(_) => handle_rpc_single(state, payload).await,
        _ => Some(rpc_error_value(
            Value::Null,
            RpcError::invalid_request("request must be an object or array"),
        )),
    }
}

async fn handle_rpc_batch(state: &AutomationState, items: Vec<Value>) -> Option<Value> {
    if items.is_empty() {
        return Some(rpc_error_value(
            Value::Null,
            RpcError::invalid_request("batch request must not be empty"),
        ));
    }

    let mut responses = Vec::new();
    for item in items {
        if let Some(response) = handle_rpc_single(state, item).await {
            responses.push(response);
        }
    }

    if responses.is_empty() {
        None
    } else {
        Some(Value::Array(responses))
    }
}

async fn handle_rpc_single(state: &AutomationState, value: Value) -> Option<Value> {
    let request: RpcRequest = match serde_json::from_value(value) {
        Ok(request) => request,
        Err(err) => {
            return Some(rpc_error_value(
                Value::Null,
                RpcError::invalid_request(format!("invalid request: {err}")),
            ));
        }
    };

    let id = match normalize_rpc_id(request.id) {
        Ok(id) => id,
        Err(err) => return Some(rpc_error_value(Value::Null, err)),
    };

    if request.jsonrpc.as_deref() != Some("2.0") {
        return Some(rpc_error_value(
            id.unwrap_or(Value::Null),
            RpcError::invalid_request("jsonrpc must be \"2.0\""),
        ));
    }

    let method = match request.method {
        Some(method) => method,
        None => {
            return Some(rpc_error_value(
                id.unwrap_or(Value::Null),
                RpcError::invalid_request("missing method"),
            ));
        }
    };

    let result = handle_rpc_method(state, method.as_str(), request.params).await;
    match result {
        Ok(result) => id.map(|id| rpc_result_value(id, result)),
        Err(err) => id.map(|id| rpc_error_value(id, err)),
    }
}

fn normalize_rpc_id(id: Option<Value>) -> Result<Option<Value>, RpcError> {
    let Some(id) = id else {
        return Ok(None);
    };
    match id {
        Value::Null | Value::String(_) | Value::Number(_) => Ok(Some(id)),
        _ => Err(RpcError::invalid_request("id must be string, number, or null")),
    }
}

fn rpc_result_value(id: Value, result: Value) -> Value {
    let fallback_id = id.clone();
    let fallback_result = result.clone();
    match serde_json::to_value(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }) {
        Ok(value) => value,
        Err(_) => json!({ "jsonrpc": "2.0", "id": fallback_id, "result": fallback_result }),
    }
}

fn rpc_error_value(id: Value, error: RpcError) -> Value {
    let fallback_id = id.clone();
    match serde_json::to_value(RpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(error),
    }) {
        Ok(value) => value,
        Err(_) => json!({ "jsonrpc": "2.0", "id": fallback_id, "error": { "code": -32000, "message": "serialization failure" } }),
    }
}

async fn handle_rpc_method(
    state: &AutomationState,
    method: &str,
    params: Option<Value>,
) -> Result<Value, RpcError> {
    match method {
        "automation.composer.status" => {
            dispatch_command(state, AutomationCommand::ComposerStatus)
                .await
                .map_err(RpcError::server_error)
        }
        "automation.thread.status" => {
            dispatch_command(state, AutomationCommand::ThreadStatus)
                .await
                .map_err(RpcError::server_error)
        }
        "automation.thread.debug" => {
            dispatch_command(state, AutomationCommand::ThreadDebug)
                .await
                .map_err(RpcError::server_error)
        }
        "automation.ready" => {
            let params: ReadyQuery = parse_params(params)?;
            let ready = wait_ready(state, params.timeout_ms.unwrap_or(30_000))
                .await
                .map_err(RpcError::server_error)?;
            if ready {
                Ok(json!({ "ready": true }))
            } else {
                Err(RpcError::server_error("timeout waiting for native app readiness"))
            }
        }
        "automation.keyboard.press" => {
            let params: KeyPressParams = parse_params(params)?;
            let normalized = normalize_keystroke_input(&params.key);
            let keystroke = Keystroke::parse(&normalized)
                .map_err(|err| RpcError::invalid_params(err.to_string()))?;
            dispatch_command(state, AutomationCommand::KeyPress { keystroke })
                .await
                .map_err(RpcError::server_error)
        }
        "automation.keyboard.type" => {
            let params: KeyTypeParams = parse_params(params)?;
            dispatch_command(state, AutomationCommand::Type { text: params.text })
                .await
                .map_err(RpcError::server_error)
        }
        "automation.locator.visible" => {
            let params: LocatorParams = parse_params(params)?;
            let node = resolve_selector_node(&params.selector)
                .ok_or_else(|| RpcError::server_error("locator not found"))?;
            Ok(json!({ "visible": node.visible }))
        }
        "automation.locator.text" => {
            let params: LocatorParams = parse_params(params)?;
            let node = resolve_selector_node(&params.selector)
                .ok_or_else(|| RpcError::server_error("locator not found"))?;
            Ok(json!({ "text": node.name.unwrap_or_default() }))
        }
        "automation.locator.click" => {
            let params: LocatorParams = parse_params(params)?;
            if let Some(node) = resolve_selector_node(&params.selector) {
                if node.visible && node.bounds.width > 0.0 && node.bounds.height > 0.0 {
                    let x = node.bounds.x + node.bounds.width / 2.0;
                    let y = node.bounds.y + node.bounds.height / 2.0;
                    return dispatch_command(
                        state,
                        AutomationCommand::Click {
                            x,
                            y,
                            button: MouseButton::Left,
                        },
                    )
                    .await
                    .map_err(RpcError::server_error);
                }
            }
            if let Some(target) = focus_target_for_selector(&params.selector) {
                return dispatch_command(state, AutomationCommand::Focus { target })
                    .await
                    .map_err(RpcError::server_error);
            }
            Err(RpcError::server_error("locator click not supported"))
        }
        "automation.locator.type" => {
            let params: LocatorTypeParams = parse_params(params)?;
            let target = match params.selector.kind.as_str() {
                "id" if params.selector.value == "composer-input" || params.selector.value == "composer-send" => {
                    FocusTarget::Composer
                }
                _ => {
                    return Err(RpcError::server_error(
                        "locator type is only supported for #composer-input",
                    ));
                }
            };
            dispatch_command(state, AutomationCommand::Focus { target })
                .await
                .map_err(RpcError::server_error)?;
            dispatch_command(state, AutomationCommand::Type { text: params.text })
                .await
                .map_err(RpcError::server_error)
        }
        "automation.tree.snapshot" => {
            serde_json::to_value(automation_tree::snapshot())
                .map_err(|err| RpcError::server_error(err.to_string()))
        }
        "ctx.ready" => {
            let params: ReadyQuery = parse_params(params)?;
            let ready = wait_ready(state, params.timeout_ms.unwrap_or(30_000))
                .await
                .map_err(RpcError::server_error)?;
            if ready {
                Ok(json!({ "ready": true }))
            } else {
                Err(RpcError::server_error("timeout waiting for native app readiness"))
            }
        }
        "ctx.screenshot" => {
            let params: ScreenshotRequest = parse_params(params)?;
            let path = resolve_screenshot_path(params, &state.config)
                .map_err(RpcError::invalid_params)?;
            wait_for_input_rendered_if_needed(state, SCREENSHOT_INPUT_WAIT_MS).await?;
            let result = dispatch_command(state, AutomationCommand::Screenshot { path })
                .await
                .map_err(RpcError::server_error)?;
            Ok(result)
        }
        "ctx.window.resize" => {
            let params: ResizeParams = parse_params(params)?;
            if params.width <= 0.0 || params.height <= 0.0 {
                return Err(RpcError::invalid_params(
                    "width and height must be positive",
                ));
            }
            dispatch_command(
                state,
                AutomationCommand::Resize {
                    width: params.width,
                    height: params.height,
                },
            )
            .await
            .map_err(RpcError::server_error)
        }
        "ctx.input.click" => {
            let params: ClickParams = parse_params(params)?;
            let button = parse_mouse_button(params.button)?;
            dispatch_command(
                state,
                AutomationCommand::Click {
                    x: params.x,
                    y: params.y,
                    button,
                },
            )
            .await
            .map_err(RpcError::server_error)
        }
        "ctx.input.scroll" => {
            let params: ScrollParams = parse_params(params)?;
            dispatch_command(
                state,
                AutomationCommand::Scroll {
                    x: params.x,
                    y: params.y,
                    delta_x: params.delta_x,
                    delta_y: params.delta_y,
                    precise: params.precise,
                },
            )
            .await
            .map_err(RpcError::server_error)
        }
        "ctx.sessions.select" => {
            let params: SelectSessionParams = parse_params(params)?;
            dispatch_command(state, AutomationCommand::SelectSession { index: params.index })
                .await
                .map_err(RpcError::server_error)
        }
        "ctx.tasks.select" => {
            let params: SelectTaskParams = parse_params(params)?;
            let task_id = Uuid::parse_str(params.task_id.trim())
                .map(TaskId)
                .map_err(|_| RpcError::invalid_params("task_id must be a UUID"))?;
            dispatch_command(state, AutomationCommand::SelectTask { task_id })
                .await
                .map_err(RpcError::server_error)
        }
        "ctx.input.type" => {
            let params: TypeParams = parse_params(params)?;
            dispatch_command(
                state,
                AutomationCommand::Type { text: params.text },
            )
            .await
            .map_err(RpcError::server_error)
        }
        "ctx.key.press" => {
            let params: KeyPressParams = parse_params(params)?;
            let normalized = normalize_keystroke_input(&params.key);
            let keystroke = Keystroke::parse(&normalized)
                .map_err(|err| RpcError::invalid_params(err.to_string()))?;
            dispatch_command(state, AutomationCommand::KeyPress { keystroke })
                .await
                .map_err(RpcError::server_error)
        }
        "ctx.wait.idle" => {
            let params: WaitIdleParams = parse_params(params)?;
            wait_for_idle(state, params).await
        }
        _ => Err(RpcError::method_not_found(format!(
            "unknown method {method}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct Selector {
    kind: String,
    value: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocatorParams {
    selector: Selector,
}

#[derive(Debug, Deserialize)]
struct LocatorTypeParams {
    selector: Selector,
    text: String,
}

fn resolve_selector_node(selector: &Selector) -> Option<automation_tree::AutomationNode> {
    match selector.kind.as_str() {
        "id" => automation_tree::registry().get_by_id(&selector.value),
        "role" => select_first_match(automation_tree::registry().get_by_role(
            &selector.value,
            selector.name.as_deref(),
        )),
        "text" => select_first_match(automation_tree::registry().get_by_text(&selector.value)),
        _ => None,
    }
}

fn select_first_match(
    nodes: Vec<automation_tree::AutomationNode>,
) -> Option<automation_tree::AutomationNode> {
    let mut first = None;
    for node in nodes {
        if first.is_none() {
            first = Some(node.clone());
        }
        if node.visible {
            return Some(node);
        }
    }
    first
}

fn focus_target_for_selector(selector: &Selector) -> Option<FocusTarget> {
    if selector.kind != "id" {
        return None;
    }
    match selector.value.as_str() {
        "app-shell" => Some(FocusTarget::Main),
        "sidebar-new-task" => Some(FocusTarget::NewTask),
        "task-search-input" => Some(FocusTarget::TaskSearch),
        "settings-search-input" => Some(FocusTarget::SettingsSearch),
        "composer-input" | "composer-send" => Some(FocusTarget::Composer),
        "sessions-list" => Some(FocusTarget::SessionsPane),
        "diff-pane" => Some(FocusTarget::DiffPane),
        "artifacts-pane" => Some(FocusTarget::ArtifactsPane),
        "terminal-panel" => Some(FocusTarget::TerminalPanel),
        _ => None,
    }
}

fn parse_params<T: DeserializeOwned>(params: Option<Value>) -> Result<T, RpcError> {
    let value = match params {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(value) => value,
    };
    serde_json::from_value(value).map_err(|err| RpcError::invalid_params(err.to_string()))
}

#[derive(Debug, Deserialize)]
struct ResizeParams {
    width: f32,
    height: f32,
}

#[derive(Debug, Deserialize)]
struct ClickParams {
    x: f32,
    y: f32,
    button: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScrollParams {
    x: f32,
    y: f32,
    delta_x: f32,
    delta_y: f32,
    #[serde(default)]
    precise: bool,
}

#[derive(Debug, Deserialize)]
struct SelectSessionParams {
    index: usize,
}

#[derive(Debug, Deserialize)]
struct SelectTaskParams {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct TypeParams {
    text: String,
}

#[derive(Debug, Deserialize)]
struct KeyPressParams {
    key: String,
}

#[derive(Debug, Deserialize)]
struct KeyTypeParams {
    text: String,
}

fn parse_mouse_button(button: Option<String>) -> Result<MouseButton, RpcError> {
    let normalized = button
        .as_deref()
        .unwrap_or("left")
        .trim()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "" | "left" => Ok(MouseButton::Left),
        "right" => Ok(MouseButton::Right),
        "middle" => Ok(MouseButton::Middle),
        "back" => Ok(MouseButton::Navigate(gpui::NavigationDirection::Back)),
        "forward" => Ok(MouseButton::Navigate(gpui::NavigationDirection::Forward)),
        _ => Err(RpcError::invalid_params("unsupported mouse button")),
    }
}

#[derive(Debug, Deserialize)]
struct WaitIdleParams {
    timeout_ms: Option<u64>,
    frames: Option<u64>,
    settle_ms: Option<u64>,
}

async fn wait_for_input_rendered_if_needed(
    state: &AutomationState,
    timeout_ms: u64,
) -> Result<u64, RpcError> {
    let target_epoch = state.pending_input_paint_epoch.load(Ordering::SeqCst);
    let rendered_epoch = state.rendered_input_epoch.load(Ordering::SeqCst);
    if target_epoch == 0 || target_epoch <= rendered_epoch {
        return Ok(rendered_epoch);
    }
    wait_for_input_rendered(state, target_epoch, timeout_ms).await
}

async fn wait_for_input_rendered(
    state: &AutomationState,
    target_epoch: u64,
    timeout_ms: u64,
) -> Result<u64, RpcError> {
    if state.rendered_input_epoch.load(Ordering::SeqCst) >= target_epoch {
        return Ok(state.rendered_input_epoch.load(Ordering::SeqCst));
    }

    dispatch_command(state, AutomationCommand::RenderFrame)
        .await
        .map_err(RpcError::server_error)?;

    if state.rendered_input_epoch.load(Ordering::SeqCst) >= target_epoch {
        return Ok(state.rendered_input_epoch.load(Ordering::SeqCst));
    }

    let mut input_render_rx = state.input_render_rx.clone();
    let wait_result = timeout(Duration::from_millis(timeout_ms), async {
        loop {
            input_render_rx
                .changed()
                .await
                .map_err(|_| RpcError::server_error("input render counter closed"))?;
            if *input_render_rx.borrow() >= target_epoch {
                break;
            }
        }
        Ok::<_, RpcError>(*input_render_rx.borrow())
    })
    .await;

    let epoch = match wait_result {
        Ok(Ok(epoch)) => epoch,
        Ok(Err(err)) => return Err(err),
        Err(_) => return Err(RpcError::server_error("timeout waiting for input render")),
    };

    dispatch_command(state, AutomationCommand::RenderFrame)
        .await
        .map_err(RpcError::server_error)?;

    Ok(epoch)
}

async fn wait_for_draw_if_needed(
    state: &AutomationState,
    timeout_ms: u64,
) -> Result<u64, RpcError> {
    let target_epoch = state.pending_draw_epoch.load(Ordering::SeqCst);
    let drawn_epoch = state.draw_epoch.load(Ordering::SeqCst);
    if target_epoch == 0 || target_epoch <= drawn_epoch {
        return Ok(drawn_epoch);
    }
    wait_for_draw(state, target_epoch, timeout_ms).await
}

async fn wait_for_draw(
    state: &AutomationState,
    target_epoch: u64,
    timeout_ms: u64,
) -> Result<u64, RpcError> {
    if state.draw_epoch.load(Ordering::SeqCst) >= target_epoch {
        return Ok(state.draw_epoch.load(Ordering::SeqCst));
    }

    dispatch_command(state, AutomationCommand::RenderFrame)
        .await
        .map_err(RpcError::server_error)?;

    if state.draw_epoch.load(Ordering::SeqCst) >= target_epoch {
        return Ok(state.draw_epoch.load(Ordering::SeqCst));
    }

    let mut draw_rx = state.draw_rx.clone();
    let wait_result = timeout(Duration::from_millis(timeout_ms), async {
        loop {
            draw_rx
                .changed()
                .await
                .map_err(|_| RpcError::server_error("draw counter closed"))?;
            if *draw_rx.borrow() >= target_epoch {
                break;
            }
        }
        Ok::<_, RpcError>(*draw_rx.borrow())
    })
    .await;

    match wait_result {
        Ok(Ok(epoch)) => Ok(epoch),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(RpcError::server_error("timeout waiting for draw")),
    }
}

async fn wait_for_idle(state: &AutomationState, params: WaitIdleParams) -> Result<Value, RpcError> {
    let frames = params.frames.map(|frames| frames.max(1));
    let timeout_ms = params.timeout_ms.unwrap_or(3_000);
    let settle_ms = params.settle_ms.unwrap_or(0);
    let render_rx = state.render_rx.clone();
    let mut draw_rx = state.draw_rx.clone();

    let wait_result = timeout(Duration::from_millis(timeout_ms), async {
        if let Some(frames) = frames {
            let target = state
                .last_input_frame
                .load(Ordering::SeqCst)
                .saturating_add(frames);
            loop {
                if *render_rx.borrow() >= target {
                    break;
                }
                dispatch_command(state, AutomationCommand::RenderFrame)
                    .await
                    .map_err(RpcError::server_error)?;
            }
        } else {
            wait_for_input_rendered_if_needed(state, timeout_ms).await?;
            wait_for_draw_if_needed(state, timeout_ms).await?;
        }

        let mut render_count = *render_rx.borrow();
        let mut draw_count = *draw_rx.borrow();
        if settle_ms > 0 {
            loop {
                let sleep = tokio::time::sleep(Duration::from_millis(settle_ms));
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => break,
                    changed = draw_rx.changed() => {
                        changed.map_err(|_| RpcError::server_error("draw counter closed"))?;
                    }
                }
            }
            render_count = *render_rx.borrow();
            draw_count = *draw_rx.borrow();
        }
        Ok::<_, RpcError>((render_count, draw_count))
    })
    .await;

    match wait_result {
        Ok(Ok((render_count, draw_count))) => Ok(json!({
            "frames": frames.unwrap_or(0),
            "render_count": render_count,
            "draw_count": draw_count,
            "settle_ms": settle_ms,
        })),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(RpcError::server_error("timeout waiting for idle render")),
    }
}

fn normalize_keystroke_input(input: &str) -> String {
    input
        .trim()
        .replace('+', "-")
        .split_whitespace()
        .collect::<String>()
}
