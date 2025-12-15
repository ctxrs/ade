use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DesktopTokenFile {
    token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DaemonDescriptor {
    url: String,
    pid: u32,
    data_dir: String,
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            app.manage(DaemonSupervisor(std::sync::Mutex::new(None)));

            let app_handle = app.handle();
            let token = load_or_create_desktop_token(&app_handle)?;
            let data_dir = daemon_data_dir(&app_handle)?;

            let mut url = None;
            let mut child = None;

            if let Ok(desc) = load_descriptor(&app_handle) {
                if desc.data_dir == data_dir.to_string_lossy()
                    && daemon_healthy(&desc.url)
                    && daemon_authed(&desc.url, &token)
                {
                    url = Some(desc.url);
                }
            }

            if url.is_none() {
                let (u, c) = spawn_daemon(&app_handle, &token, &data_dir)?;
                let pid = c.id();
                save_descriptor(
                    &app_handle,
                    &DaemonDescriptor {
                        url: u.clone(),
                        pid,
                        data_dir: data_dir.to_string_lossy().to_string(),
                    },
                )?;
                url = Some(u);
                child = Some(c);
            }

            let url = url.context("missing daemon url")?;
            open_main_window(&app_handle, &url, &token)?;

            // Start a lightweight supervisor that restarts the daemon if it exits or becomes unhealthy.
            let (tx, rx) = mpsc::channel::<SupervisorCmd>();
            {
                let state = app_handle.state::<DaemonSupervisor>();
                state.set(tx);
            }
            start_supervisor_thread(app_handle, rx, token, data_dir, url, child);
            Ok(())
        })
        .on_window_event(|event| {
            if matches!(event.event(), tauri::WindowEvent::CloseRequested { .. }) {
                let state = event.window().state::<DaemonSupervisor>();
                state.stop();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Clone, Copy)]
enum SupervisorCmd {
    Stop,
}

struct DaemonSupervisor(std::sync::Mutex<Option<mpsc::Sender<SupervisorCmd>>>);

impl DaemonSupervisor {
    fn set(&self, tx: mpsc::Sender<SupervisorCmd>) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = Some(tx);
        }
    }

    fn stop(&self) {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(SupervisorCmd::Stop);
            }
        }
    }
}

fn start_supervisor_thread(
    app: tauri::AppHandle,
    rx: mpsc::Receiver<SupervisorCmd>,
    token: String,
    data_dir: PathBuf,
    initial_url: String,
    initial_child: Option<Child>,
) {
    std::thread::spawn(move || {
        let mut url = initial_url;
        let mut child = initial_child;
        let mut consecutive_unhealthy = 0u32;
        let mut backoff_ms = 250u64;

        loop {
            if let Ok(SupervisorCmd::Stop) = rx.try_recv() {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                }
                break;
            }

            let child_exited = match child.as_mut() {
                Some(c) => match c.try_wait() {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(_) => false,
                },
                None => false,
            };
            if child_exited {
                child = None;
            }

            let healthy = daemon_healthy(&url) && daemon_authed(&url, &token);
            if healthy {
                consecutive_unhealthy = 0;
                backoff_ms = 250;
            } else {
                consecutive_unhealthy = consecutive_unhealthy.saturating_add(1);
            }

            let should_restart = child_exited || consecutive_unhealthy >= 2;
            if should_restart {
                if let Some(mut c) = child.take() {
                    let _ = c.kill();
                }

                match spawn_daemon(&app, &token, &data_dir) {
                    Ok((new_url, new_child)) => {
                        let pid = new_child.id();
                        let _ = save_descriptor(
                            &app,
                            &DaemonDescriptor {
                                url: new_url.clone(),
                                pid,
                                data_dir: data_dir.to_string_lossy().to_string(),
                            },
                        );
                        url = new_url.clone();
                        child = Some(new_child);
                        consecutive_unhealthy = 0;
                        backoff_ms = 250;
                        let _ = navigate_main_window(&app, &new_url, &token);
                    }
                    Err(e) => {
                        eprintln!("desktop: failed to restart daemon: {e:#}");
                        backoff_ms = (backoff_ms * 2).min(10_000);
                    }
                }
            }

            std::thread::sleep(Duration::from_millis(backoff_ms.max(500)));
        }
    });
}

fn open_main_window(app: &tauri::AppHandle, daemon_url: &str, token: &str) -> Result<()> {
    let url = format!(
        "{}/?desktop=1&token={}",
        daemon_url.trim_end_matches('/'),
        urlencoding::encode(token)
    );
    let url = url.parse().context("parsing daemon url")?;
    tauri::WindowBuilder::new(app, "main", tauri::WindowUrl::External(url))
        .title("Context")
        .inner_size(1200.0, 900.0)
        .build()
        .context("creating window")?;
    Ok(())
}

fn navigate_main_window(app: &tauri::AppHandle, daemon_url: &str, token: &str) -> Result<()> {
    let Some(window) = app.get_window("main") else {
        return Ok(());
    };
    let url = format!(
        "{}/?desktop=1&token={}",
        daemon_url.trim_end_matches('/'),
        urlencoding::encode(token)
    );
    let js = format!(
        "window.location.replace({});",
        serde_json::to_string(&url).unwrap_or_else(|_| "\"/\"".to_string())
    );
    window.eval(&js).ok();
    Ok(())
}

fn desktop_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = app
        .path_resolver()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("desktop"))
}

fn token_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(desktop_dir(app)?.join("token.json"))
}

fn descriptor_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(desktop_dir(app)?.join("daemon.json"))
}

fn daemon_data_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = app
        .path_resolver()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("daemon"))
}

fn load_or_create_desktop_token(app: &tauri::AppHandle) -> Result<String> {
    let path = token_path(app)?;
    if path.exists() {
        let txt = std::fs::read_to_string(&path)?;
        let parsed: DesktopTokenFile = serde_json::from_str(&txt)?;
        return Ok(parsed.token);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let token = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&DesktopTokenFile {
            token: token.clone(),
        })?,
    )?;
    Ok(token)
}

fn load_descriptor(app: &tauri::AppHandle) -> Result<DaemonDescriptor> {
    let path = descriptor_path(app)?;
    let txt = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&txt)?)
}

fn save_descriptor(app: &tauri::AppHandle, desc: &DaemonDescriptor) -> Result<()> {
    let path = descriptor_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(desc)?)?;
    Ok(())
}

fn daemon_healthy(url: &str) -> bool {
    let client = reqwest::blocking::Client::new();
    client
        .get(format!("{}/api/health", url.trim_end_matches('/')))
        .send()
        .ok()
        .and_then(|r| r.error_for_status().ok())
        .is_some()
}

fn daemon_authed(url: &str, token: &str) -> bool {
    let client = reqwest::blocking::Client::new();
    client
        .get(format!("{}/api/providers", url.trim_end_matches('/')))
        .bearer_auth(token)
        .send()
        .ok()
        .and_then(|r| r.error_for_status().ok())
        .is_some()
}

fn resource_bin(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(res) = app.path_resolver().resource_dir() {
        candidates.push(res.join("bin").join(name));
        candidates.push(res.join(name));
        #[cfg(target_os = "windows")]
        {
            candidates.push(res.join("bin").join(format!("{name}.exe")));
            candidates.push(res.join(format!("{name}.exe")));
        }
    }

    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    None
}

fn dev_bin(name: &str) -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())?
        .to_path_buf(); // core/
    let candidate = root.join("target").join("debug").join(name);
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

fn dev_web_dist() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())?
        .join("web")
        .join("dist"); // core/apps/web/dist
    if dir.exists() {
        Some(dir)
    } else {
        None
    }
}

fn spawn_daemon(app: &tauri::AppHandle, token: &str, data_dir: &Path) -> Result<(String, Child)> {
    let context_bin = resource_bin(app, "context")
        .or_else(|| dev_bin("context"))
        .unwrap_or_else(|| PathBuf::from("context"));

    let mcp_bin = resource_bin(app, "context-mcp")
        .or_else(|| dev_bin("context-mcp"));

    let web_dist = app
        .path_resolver()
        .resource_dir()
        .and_then(|p| {
            let candidates = [p.join("web").join("dist"), p.join("web-dist"), p.join("dist")];
            candidates.into_iter().find(|c| c.exists())
        })
        .or_else(dev_web_dist);

    let mut cmd = Command::new(&context_bin);
    cmd.arg("serve")
        .arg("--bind")
        .arg("127.0.0.1:0")
        .arg("--data-dir")
        .arg(data_dir.to_string_lossy().to_string())
        .arg("--auth-token")
        .arg(token)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    if let Some(dist) = web_dist.as_ref() {
        cmd.env("CONTEXT_WEB_DIST", dist.to_string_lossy().to_string());
    }
    if let Some(mcp) = mcp_bin.as_ref() {
        cmd.env("CONTEXT_MCP_COMMAND", mcp.to_string_lossy().to_string());
    }
    if let Ok(appimage) = std::env::var("APPIMAGE") {
        cmd.env("CONTEXT_APPIMAGE_PATH", appimage);
    }

    let mut child = cmd.spawn().context("spawning context daemon")?;
    let stdout = child.stdout.take().context("capturing daemon stdout")?;
    let mut reader = BufReader::new(stdout).lines();

    let mut url: Option<String> = None;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if let Some(line) = reader.next() {
            let line = line?;
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                if v.get("event").and_then(|e| e.as_str()) == Some("listening") {
                    if let Some(u) = v.get("url").and_then(|u| u.as_str()) {
                        url = Some(u.to_string());
                        break;
                    }
                }
            }
        } else {
            break;
        }
    }

    let url = url.context("daemon did not emit listening URL")?;
    Ok((url, child))
}

#[allow(dead_code)]
fn is_executable(path: &Path) -> bool {
    path.exists()
}
