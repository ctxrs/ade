use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use gpui::{
    App, Bounds, Context, DevicePixels, Pixels, ScreenCaptureFrame, Window, WindowHandle, point,
    size,
};
use image::{ColorType, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::timeout;

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

    app.spawn(move |cx| async move {
        run_command_loop(cx, window, command_rx).await;
    })
    .detach();

    let handle = match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("ctx-native: automation runtime unavailable: {err}");
            return;
        }
    };
    handle.spawn(run_http_server(addr, Arc::clone(&state)));
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

    let ready = timeout(Duration::from_millis(timeout_ms), ready_rx.changed())
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
    Screenshot { path: PathBuf },
    Exit,
}

async fn run_command_loop(
    cx: &mut gpui::AsyncApp,
    window: WindowHandle<ShellView>,
    mut command_rx: mpsc::Receiver<AutomationRequest>,
) {
    while let Some(request) = command_rx.recv().await {
        let command = request.command;
        let response = match command {
            AutomationCommand::Focus { target } => window
                .update(cx, |view, window, cx| {
                    apply_focus_target(view, window, cx, target);
                    Ok(json!({ "target": target.as_str() }))
                })
                .map_err(|err| err.to_string())
                .and_then(|result| result),
            AutomationCommand::Screenshot { path } => capture_window(cx, &window, path)
                .await
                .map(|path| json!({ "path": path.to_string_lossy() })),
            AutomationCommand::Exit => window
                .update(cx, |_, _, cx| {
                    cx.quit();
                    Ok(json!({ "quitting": true }))
                })
                .map_err(|err| err.to_string())
                .and_then(|result| result),
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
            view.composer_focus.focus(window);
        }
        FocusTarget::ComposerAttachments => {
            view.composer_attachment_focus.focus(window);
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

#[derive(Clone, Copy)]
struct CaptureInfo {
    window_bounds: Bounds<Pixels>,
    display_bounds: Bounds<Pixels>,
    scale_factor: f32,
    display_id: Option<u64>,
}

async fn capture_window(
    cx: &mut gpui::AsyncApp,
    window: &WindowHandle<ShellView>,
    path: PathBuf,
) -> Result<PathBuf, String> {
    let supported = cx
        .update(|app| app.is_screen_capture_supported())
        .map_err(|err| err.to_string())?;
    if !supported {
        return Err("GPUI screen capture is not available on this platform".to_string());
    }

    let info = window
        .update(cx, |_, window, cx| {
            let window_bounds = window.bounds();
            let scale_factor = window.scale_factor();
            let display = window.display().or_else(|| cx.primary_display());
            let display_bounds = display
                .as_ref()
                .map(|display| display.bounds())
                .unwrap_or(window_bounds);
            let display_id = display.map(|display| u64::from(u32::from(display.id())));
            CaptureInfo {
                window_bounds,
                display_bounds,
                scale_factor,
                display_id,
            }
        })
        .map_err(|err| err.to_string())?;

    let sources_rx = cx
        .update(|app| app.screen_capture_sources())
        .map_err(|err| err.to_string())?;
    let sources = timeout(Duration::from_secs(5), sources_rx)
        .await
        .map_err(|_| "timeout waiting for screen capture sources".to_string())?
        .map_err(|err| err.to_string())?;
    if sources.is_empty() {
        return Err("no screen capture sources available".to_string());
    }

    let mut selected = None;
    if let Some(display_id) = info.display_id {
        for source in &sources {
            if let Ok(metadata) = source.metadata() {
                if metadata.id == display_id {
                    selected = Some(source.clone());
                    break;
                }
            }
        }
    }
    if selected.is_none() {
        for source in &sources {
            if let Ok(metadata) = source.metadata() {
                if metadata.is_main.unwrap_or(false) {
                    selected = Some(source.clone());
                    break;
                }
            }
        }
    }
    let source = selected.unwrap_or_else(|| sources[0].clone());

    let (frame_tx, frame_rx) = oneshot::channel();
    let frame_tx = Arc::new(Mutex::new(Some(frame_tx)));
    let stream_rx = source.stream(
        cx.foreground_executor(),
        Box::new(move |frame| {
            if let Ok(mut slot) = frame_tx.lock() {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(frame);
                }
            }
        }),
    );
    let _stream = timeout(Duration::from_secs(5), stream_rx)
        .await
        .map_err(|_| "timeout starting screen capture stream".to_string())?
        .map_err(|err| err.to_string())?;
    let frame = timeout(Duration::from_secs(5), frame_rx)
        .await
        .map_err(|_| "timeout waiting for screen capture frame".to_string())?
        .map_err(|_| "screen capture stream closed".to_string())?;

    let rgba_frame = frame_to_rgba(frame)?;
    let crop = compute_crop(info, rgba_frame.width, rgba_frame.height)?;
    let cropped = crop_rgba(&rgba_frame, crop)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create screenshot dir: {err}"))?;
    }

    image::save_buffer_with_format(
        &path,
        &cropped.data,
        cropped.width,
        cropped.height,
        ColorType::Rgba8,
        ImageFormat::Png,
    )
    .map_err(|err| format!("failed to write screenshot: {err}"))?;

    Ok(path)
}

#[derive(Clone, Copy)]
struct CropRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Clone)]
struct RgbaFrame {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

fn compute_crop(info: CaptureInfo, frame_width: u32, frame_height: u32) -> Result<CropRect, String> {
    let window_device = bounds_to_device(info.window_bounds, info.scale_factor);
    let display_device = bounds_to_device(info.display_bounds, info.scale_factor);

    let x = window_device.origin.x.0 - display_device.origin.x.0;
    let y = window_device.origin.y.0 - display_device.origin.y.0;
    let width = window_device.size.width.0;
    let height = window_device.size.height.0;

    let frame_width = frame_width as i32;
    let frame_height = frame_height as i32;
    if frame_width <= 0 || frame_height <= 0 {
        return Err("screen capture returned an empty frame".to_string());
    }

    let mut crop_x = x.max(0);
    let mut crop_y = y.max(0);
    let mut crop_w = width;
    let mut crop_h = height;
    if crop_x >= frame_width || crop_y >= frame_height {
        return Err("window bounds are outside the capture frame".to_string());
    }
    if crop_x + crop_w > frame_width {
        crop_w = frame_width - crop_x;
    }
    if crop_y + crop_h > frame_height {
        crop_h = frame_height - crop_y;
    }
    if crop_w <= 0 || crop_h <= 0 {
        return Err("window bounds produce an empty capture region".to_string());
    }

    Ok(CropRect {
        x: crop_x,
        y: crop_y,
        width: crop_w,
        height: crop_h,
    })
}

fn bounds_to_device(bounds: Bounds<Pixels>, scale_factor: f32) -> Bounds<DevicePixels> {
    let scaled = bounds.scale(scale_factor);
    Bounds::new(
        point(DevicePixels::from(scaled.origin.x), DevicePixels::from(scaled.origin.y)),
        size(
            DevicePixels::from(scaled.size.width),
            DevicePixels::from(scaled.size.height),
        ),
    )
}

fn crop_rgba(frame: &RgbaFrame, crop: CropRect) -> Result<RgbaFrame, String> {
    let width = crop.width as u32;
    let height = crop.height as u32;
    let frame_width = frame.width as i32;
    if width == 0 || height == 0 {
        return Err("crop size is empty".to_string());
    }
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..crop.height {
        let src_row = crop.y + row;
        let src_start = ((src_row * frame_width + crop.x) * 4) as usize;
        let src_end = src_start + (crop.width * 4) as usize;
        data.extend_from_slice(&frame.data[src_start..src_end]);
    }
    Ok(RgbaFrame { width, height, data })
}

#[cfg(target_os = "macos")]
fn frame_to_rgba(frame: ScreenCaptureFrame) -> Result<RgbaFrame, String> {
    use core_foundation::base::TCFType;
    use core_video::{pixel_buffer::CVPixelBuffer, r#return::kCVReturnSuccess};

    let pixel_buffer = unsafe {
        CVPixelBuffer::wrap_under_get_rule(frame.0.as_concrete_TypeRef() as _)
    };
    unsafe {
        if pixel_buffer.lock_base_address(0) != kCVReturnSuccess {
            return Err("failed to lock screen capture buffer".to_string());
        }
        let width = pixel_buffer.get_width() as u32;
        let height = pixel_buffer.get_height() as u32;
        let bytes_per_row = pixel_buffer.get_bytes_per_row() as usize;
        let base = pixel_buffer.get_base_address();
        if base.is_null() {
            let _ = pixel_buffer.unlock_base_address(0);
            return Err("screen capture buffer is null".to_string());
        }
        let raw_len = bytes_per_row * height as usize;
        let raw = std::slice::from_raw_parts(base as *const u8, raw_len);
        let mut data = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height as usize {
            let start = row * bytes_per_row;
            let end = start + (width as usize * 4);
            for px in raw[start..end].chunks_exact(4) {
                data.push(px[2]);
                data.push(px[1]);
                data.push(px[0]);
                data.push(px[3]);
            }
        }
        if pixel_buffer.unlock_base_address(0) != kCVReturnSuccess {
            return Err("failed to unlock screen capture buffer".to_string());
        }
        Ok(RgbaFrame { width, height, data })
    }
}

#[cfg(not(target_os = "macos"))]
fn frame_to_rgba(frame: ScreenCaptureFrame) -> Result<RgbaFrame, String> {
    use scap::frame::Frame;

    match frame.0 {
        Frame::BGRA(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_bgra(&frame.data),
        }),
        Frame::BGRx(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_bgrx(&frame.data),
        }),
        Frame::XBGR(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_xbgr(&frame.data),
        }),
        Frame::RGBx(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_rgbx(&frame.data),
        }),
        Frame::RGB(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_rgb(&frame.data),
        }),
        Frame::BGR0(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_bgr0(&frame.data, frame.width, frame.height),
        }),
        Frame::YUVFrame(frame) => Ok(RgbaFrame {
            width: frame.width as u32,
            height: frame.height as u32,
            data: convert_nv12(&frame),
        }),
    }
}

#[cfg(not(target_os = "macos"))]
fn convert_bgra(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        out.push(px[2]);
        out.push(px[1]);
        out.push(px[0]);
        out.push(px[3]);
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn convert_bgrx(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        out.push(px[2]);
        out.push(px[1]);
        out.push(px[0]);
        out.push(255);
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn convert_xbgr(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        out.push(px[3]);
        out.push(px[2]);
        out.push(px[1]);
        out.push(255);
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn convert_rgbx(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for px in data.chunks_exact(4) {
        out.push(px[0]);
        out.push(px[1]);
        out.push(px[2]);
        out.push(255);
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn convert_rgb(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity((data.len() / 3) * 4);
    for px in data.chunks_exact(3) {
        out.push(px[0]);
        out.push(px[1]);
        out.push(px[2]);
        out.push(255);
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn convert_bgr0(data: &[u8], width: i32, height: i32) -> Vec<u8> {
    let expected = (width * height * 4) as usize;
    if data.len() == expected {
        return convert_bgrx(data);
    }
    let mut out = Vec::with_capacity((data.len() / 3) * 4);
    for px in data.chunks_exact(3) {
        out.push(px[2]);
        out.push(px[1]);
        out.push(px[0]);
        out.push(255);
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn convert_nv12(frame: &scap::frame::YUVFrame) -> Vec<u8> {
    let width = frame.width.max(0) as usize;
    let height = frame.height.max(0) as usize;
    let mut out = vec![0; width * height * 4];
    let y_stride = frame.luminance_stride.max(0) as usize;
    let uv_stride = frame.chrominance_stride.max(0) as usize;
    for y in 0..height {
        let y_row = y * y_stride;
        let uv_row = (y / 2) * uv_stride;
        for x in 0..width {
            let y_val = frame.luminance_bytes[y_row + x] as i32;
            let uv_index = uv_row + (x / 2) * 2;
            let u_val = frame.chrominance_bytes[uv_index] as i32;
            let v_val = frame.chrominance_bytes[uv_index + 1] as i32;
            let (r, g, b) = yuv_to_rgb(y_val, u_val, v_val);
            let idx = (y * width + x) * 4;
            out[idx] = r;
            out[idx + 1] = g;
            out[idx + 2] = b;
            out[idx + 3] = 255;
        }
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn yuv_to_rgb(y: i32, u: i32, v: i32) -> (u8, u8, u8) {
    let c = y - 16;
    let d = u - 128;
    let e = v - 128;
    let r = (298 * c + 409 * e + 128) >> 8;
    let g = (298 * c - 100 * d - 208 * e + 128) >> 8;
    let b = (298 * c + 516 * d + 128) >> 8;
    (clamp_rgb(r), clamp_rgb(g), clamp_rgb(b))
}

#[cfg(not(target_os = "macos"))]
fn clamp_rgb(value: i32) -> u8 {
    if value < 0 {
        0
    } else if value > 255 {
        255
    } else {
        value as u8
    }
}
