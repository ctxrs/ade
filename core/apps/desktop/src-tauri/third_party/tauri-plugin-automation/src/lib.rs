use std::{
    collections::HashMap,
    ffi::c_void,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use tauri::{LogicalPosition, LogicalSize, Manager, Runtime, State};

type MessageHandler = unsafe extern "C" fn(*mut c_void);

type MessageHandlerFn = dyn Fn(&mut Message) + Send + Sync;
static MESSAGE_HANDLER: OnceLock<Box<MessageHandlerFn>> = OnceLock::new();

#[tauri::command]
async fn resolve<R: Runtime>(
    _app: tauri::AppHandle<R>,
    automation: State<'_, Automation>,
    id: String,
    result: Option<serde_json::Value>,
) -> Result<(), ()> {
    let mut pending = automation
        .pending_scripts
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(sender) = pending.remove(&id) {
        let _ = sender.send(result.unwrap_or_default());
    }
    Ok(())
}

#[allow(dead_code)]
#[derive(Debug)]
enum MessageKind {
    EvalScript {
        id: String,
        label: Option<String>,
        script: String,
    },
    GetWindowHandle,
    GetWindowHandles,
    CloseWindow {
        label: String,
    },
    GetWindowRect {
        label: Option<String>,
    },
    SetWindowRect {
        label: Option<String>,
        x: Option<i32>,
        y: Option<i32>,
        width: Option<i32>,
        height: Option<i32>,
    },
    FullscreenWindow {
        label: Option<String>,
    },
    MinimizeWindow {
        label: Option<String>,
    },
    MaximizeWindow {
        label: Option<String>,
    },
}

struct Message {
    kind: MessageKind,
    response_tx: Option<tokio::sync::oneshot::Sender<serde_json::Value>>,
}

struct Automation {
    pending_scripts: Mutex<HashMap<String, tokio::sync::oneshot::Sender<serde_json::Value>>>,
}

fn take_response(
    message: &mut Message,
) -> Option<tokio::sync::oneshot::Sender<serde_json::Value>> {
    message.response_tx.take()
}

fn try_send_response(
    sender: tokio::sync::oneshot::Sender<serde_json::Value>,
    value: serde_json::Value,
) {
    let _ = sender.send(value);
}

pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let (webview_created_tx, webview_created_rx) = tokio::sync::broadcast::channel(16);

    tauri::plugin::Builder::new("automation")
        .invoke_handler(tauri::generate_handler![resolve])
        .js_init_script(include_str!("init.js").to_string())
        .on_webview_ready(move |webview| {
            webview_created_tx
                .send(webview.get_webview_window(webview.label()).expect(&format!(
                    "Failed to get webview window for label {}",
                    webview.label()
                )))
                // This could fail if there's no task on the receiving end,
                //  we can ignore safely.
                .unwrap_or_default();
        })
        .setup(|app, _api| {
            app.manage(Automation {
                pending_scripts: Mutex::new(HashMap::new()),
            });

            app.add_capability(
                tauri::ipc::CapabilityBuilder::new("automation")
                    .local(true)
                    .window("*")
                    .remote("http://*".into())
                    .remote("https://*".into())
                    .permission("automation:default"),
            )?;

            unsafe {
                if let Some(lib_path) =
                    std::env::var_os("AUTOMATION_LIBRARY_PATH").map(PathBuf::from)
                {
                    let lib = libloading::Library::new(lib_path).expect("Could not load library");
                    let start: libloading::Symbol<unsafe extern "C" fn(MessageHandler)> = lib
                        .get(b"tauri_plugin_automation_start")
                        .expect("Failed to get the automation start function from automation lib");

                    let app_ = app.clone();
                    MESSAGE_HANDLER
                        .set(Box::new(move |message| match &message.kind {
                            MessageKind::EvalScript { id, label, script } => {
                                let id = id.clone();
                                let label = label.clone();
                                let script = script.clone();
                                let automation = app_.state::<Automation>();
                                let mut pending = automation
                                    .pending_scripts
                                    .lock()
                                    .unwrap_or_else(|poison| poison.into_inner());
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                pending.insert(id.clone(), response_tx);

                                with_window(
                                    &app_,
                                    label.as_deref(),
                                    &webview_created_rx,
                                    move |window| {
                                        let _ = window.eval(&script);
                                    },
                                );
                            }
                            MessageKind::GetWindowHandle => {
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                let handle = if app_.get_webview_window("main").is_some() {
                                    "main".to_string().into()
                                } else {
                                    app_.webview_windows()
                                        .into_values()
                                        .next()
                                        .map(|w| w.label().into())
                                        .unwrap_or_default()
                                };
                                try_send_response(response_tx, handle);
                            }
                            MessageKind::GetWindowHandles => {
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                let handles = app_
                                    .webview_windows()
                                    .into_values()
                                    .map(|w| w.label().to_string())
                                    .collect();
                                try_send_response(response_tx, handles);
                            }
                            MessageKind::CloseWindow { label } => {
                                let label = label.clone();
                                let window = app_.get_webview_window(&label);
                                if let Some(window) = &window {
                                    window.close().expect("Failed to close the window");
                                }
                                if let Some(response_tx) = take_response(message) {
                                    try_send_response(response_tx, window.is_some().into());
                                }
                            }
                            MessageKind::GetWindowRect { label } => {
                                let label = label.clone();
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                with_window(
                                    &app_,
                                    label.as_deref(),
                                    &webview_created_rx,
                                    move |window| {
                                        let scale_factor = window
                                            .scale_factor()
                                            .expect("Failed to get window scale factor");
                                        let size = window
                                            .inner_size()
                                            .expect("Failed to get window inner size")
                                            .to_logical::<i32>(scale_factor);
                                        let position = window
                                            .inner_position()
                                            .expect("Failed to get window inner position")
                                            .to_logical::<i32>(scale_factor);
                                        try_send_response(
                                            response_tx,
                                            serde_json::json!({
                                                "x": position.x,
                                                "y": position.y,
                                                "width": size.width,
                                                "height": size.height,
                                            }),
                                        );
                                    },
                                );
                            }
                            MessageKind::SetWindowRect {
                                label,
                                x,
                                y,
                                width,
                                height,
                            } => {
                                let label = label.clone();
                                let x = *x;
                                let y = *y;
                                let width = *width;
                                let height = *height;
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                with_window(
                                    &app_,
                                    label.as_deref(),
                                    &webview_created_rx,
                                    move |window| {
                                        if let (Some(x), Some(y)) = (x, y) {
                                            window
                                                .set_position(LogicalPosition::new(x, y))
                                                .expect("Failed to set window position");
                                        }
                                        if let (Some(width), Some(height)) = (width, height) {
                                            window
                                                .set_size(LogicalSize::new(width, height))
                                                .expect("Failed to set window size");
                                        }
                                        try_send_response(response_tx, true.into());
                                    },
                                );
                            }
                            MessageKind::FullscreenWindow { label } => {
                                let label = label.clone();
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                with_window(
                                    &app_,
                                    label.as_deref(),
                                    &webview_created_rx,
                                    move |window| {
                                        window
                                            .set_fullscreen(true)
                                            .expect("Failed to fullscreen the window");
                                        try_send_response(response_tx, true.into());
                                    },
                                );
                            }
                            MessageKind::MinimizeWindow { label } => {
                                let label = label.clone();
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                with_window(
                                    &app_,
                                    label.as_deref(),
                                    &webview_created_rx,
                                    move |window| {
                                        window.minimize().expect("Failed to minimize the window");
                                        try_send_response(response_tx, true.into());
                                    },
                                );
                            }
                            MessageKind::MaximizeWindow { label } => {
                                let label = label.clone();
                                let response_tx = match take_response(message) {
                                    Some(response_tx) => response_tx,
                                    None => return,
                                };
                                with_window(
                                    &app_,
                                    label.as_deref(),
                                    &webview_created_rx,
                                    move |window| {
                                        window.maximize().expect("Failed to maximize window");
                                        try_send_response(response_tx, true.into());
                                    },
                                );
                            }
                        }))
                        .unwrap_or_else(|_| {
                            panic!("Failed to set message handler");
                        });

                    start(handle_message);
                }
            }

            Ok(())
        })
        .build()
}

extern "C" fn handle_message(message: *mut c_void) {
    let message = unsafe { &mut *(message as *mut Message) };
    MESSAGE_HANDLER.get().unwrap()(message);
}

fn with_window<R: Runtime, F: FnOnce(tauri::WebviewWindow<R>) + Send + 'static>(
    app: &tauri::AppHandle<R>,
    label: Option<&str>,
    webview_created_rx: &tokio::sync::broadcast::Receiver<tauri::WebviewWindow<R>>,
    f: F,
) {
    if let Some(window) = window_by_label(app, label) {
        f(window);
    } else {
        let mut webview_created_rx = webview_created_rx.resubscribe();
        tauri::async_runtime::spawn(async move {
            loop {
                let window = webview_created_rx.recv().await;
                if let Ok(webview) = window {
                    f(webview);
                    break;
                }
            }
        });
    }
}

fn window_by_label<R: Runtime>(
    app: &tauri::AppHandle<R>,
    label: Option<&str>,
) -> Option<tauri::WebviewWindow<R>> {
    if let Some(label) = label {
        app.get_webview_window(label)
    } else {
        app.get_webview_window("main")
            .or_else(|| app.webview_windows().into_values().next())
    }
}
