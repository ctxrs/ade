use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .manage(ConnectionManager::default())
        .invoke_handler(tauri::generate_handler![
            desktop_get_connection,
            desktop_disconnect,
            desktop_connect_local,
            desktop_connect_ssh,
            desktop_pick_folder,
            desktop_git_clone,
            desktop_save_text_file,
            desktop_upload_blob,
            desktop_daemon_request,
        ])
        .setup(|app| {
            open_main_window(app.handle())?;
            Ok(())
        })
        .on_window_event(|event| {
            if matches!(event.event(), tauri::WindowEvent::CloseRequested { .. }) {
                let manager = event.window().state::<ConnectionManager>();
                manager.disconnect();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum DesktopConnectionKind {
    None,
    Local,
    Ssh,
}

#[derive(Debug, Clone, Serialize)]
struct DesktopConnectionInfo {
    kind: DesktopConnectionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SshConnectReq {
    host: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    remote_port: Option<u16>,
    #[serde(default)]
    start_remote: bool,
    #[serde(default)]
    auth_token: Option<String>,
    #[serde(default)]
    remote_data_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DesktopDaemonRequest {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    headers: Vec<(String, String)>,
}

#[derive(Debug, Serialize)]
struct DesktopHttpResponse {
    status: u16,
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_type: Option<String>,
}

#[tauri::command]
fn desktop_get_connection(state: tauri::State<ConnectionManager>) -> DesktopConnectionInfo {
    state.info()
}

#[tauri::command]
fn desktop_disconnect(state: tauri::State<ConnectionManager>) -> Result<(), String> {
    state.disconnect();
    Ok(())
}

#[tauri::command]
fn desktop_pick_folder() -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    tauri::api::dialog::FileDialogBuilder::new().pick_folder(move |path| {
        let _ = tx.send(path.map(|p| p.to_string_lossy().to_string()));
    });
    rx.recv_timeout(Duration::from_secs(60))
        .map_err(|_| "folder picker timed out".to_string())
}

#[tauri::command]
fn desktop_save_text_file(suggested_name: Option<String>, contents: String) -> Result<Option<String>, String> {
    let suggested = suggested_name.unwrap_or_else(|| "conversation.md".to_string());
    let suggested = suggested.trim();

    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    let mut dialog = tauri::api::dialog::FileDialogBuilder::new()
        .add_filter("Markdown", &["md"])
        .set_title("Save Conversation Export");
    if !suggested.is_empty() {
        dialog = dialog.set_file_name(suggested);
    }
    dialog.save_file(move |path| {
        let _ = tx.send(path.map(|p| p.to_string_lossy().to_string()));
    });

    let picked = rx
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "save file dialog timed out".to_string())?;
    let Some(path) = picked else {
        return Ok(None);
    };

    std::fs::write(&path, contents).map_err(|e| format!("failed to write file: {e}"))?;
    Ok(Some(path))
}

#[tauri::command]
fn desktop_git_clone(repo_url: String, dest_parent: String) -> Result<String, String> {
    let repo_url = repo_url.trim().to_string();
    if repo_url.is_empty() {
        return Err("repo_url is required".to_string());
    }
    let dest_parent = PathBuf::from(dest_parent);
    if !dest_parent.exists() {
        return Err(format!(
            "destination folder does not exist: {}",
            dest_parent.display()
        ));
    }

    let name = derive_repo_name(&repo_url).ok_or_else(|| "could not derive repo name".to_string())?;
    let dest = dest_parent.join(&name);
    if dest.exists() {
        return Err(format!("destination already exists: {}", dest.display()));
    }

    let output = Command::new("git")
        .arg("clone")
        .arg("--")
        .arg(&repo_url)
        .arg(&dest)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("failed to spawn git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(dest.to_string_lossy().to_string())
}

#[tauri::command]
fn desktop_connect_local(
    app: tauri::AppHandle,
    state: tauri::State<ConnectionManager>,
) -> Result<DesktopConnectionInfo, String> {
    state.disconnect();
    let token = uuid::Uuid::new_v4().to_string();
    let data_dir = daemon_data_dir(&app).map_err(to_err)?;
    let (url, child) = spawn_daemon(&app, &token, &data_dir).map_err(to_err)?;
    state.set_local(url.clone(), token.clone(), data_dir, child);
    Ok(state.info())
}

#[tauri::command]
fn desktop_connect_ssh(
    _app: tauri::AppHandle,
    state: tauri::State<ConnectionManager>,
    req: SshConnectReq,
) -> Result<DesktopConnectionInfo, String> {
    state.disconnect();

    let host = req.host.trim().to_string();
    if host.is_empty() {
        return Err("host is required".to_string());
    }
    let remote_port = req.remote_port.unwrap_or(4399);

    let token = if req.start_remote {
        Some(
            req.auth_token
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        )
    } else {
        req.auth_token
            .clone()
            .filter(|t| !t.trim().is_empty())
    };

    if req.start_remote {
        start_remote_daemon_over_ssh(&host, req.user.as_deref(), remote_port, token.as_deref(), req.remote_data_dir.as_deref())
            .map_err(to_err)?;
    }

    let local_port = pick_unused_local_port().map_err(to_err)?;
    let tunnel = start_ssh_tunnel(&host, req.user.as_deref(), local_port, remote_port).map_err(to_err)?;
    let base_url = format!("http://127.0.0.1:{local_port}");

    // Health check before returning.
    if let Err(e) = probe_daemon_health(&base_url, token.as_deref()) {
        let _ = try_kill_child(tunnel);
        return Err(format!("failed to reach remote daemon: {e:#}"));
    }

    state.set_ssh(base_url, token, tunnel);
    Ok(state.info())
}

#[tauri::command]
fn desktop_daemon_request(
    state: tauri::State<ConnectionManager>,
    req: DesktopDaemonRequest,
) -> Result<DesktopHttpResponse, String> {
    state.daemon_request(req).map_err(to_err)
}

#[tauri::command]
fn desktop_upload_blob(
    state: tauri::State<ConnectionManager>,
    bytes: Vec<u8>,
    mime_type: String,
    name: Option<String>,
) -> Result<serde_json::Value, String> {
    state.upload_blob(bytes, mime_type, name).map_err(to_err)
}

fn open_main_window(app: &tauri::AppHandle) -> Result<()> {
    if app.get_window("main").is_some() {
        return Ok(());
    }
    tauri::WindowBuilder::new(app, "main", tauri::WindowUrl::App("index.html".into()))
        .title("Context")
        .inner_size(1200.0, 900.0)
        .build()
        .context("creating window")?;
    Ok(())
}

#[derive(Default)]
struct ConnectionManager(std::sync::Mutex<ConnectionState>);

#[derive(Default)]
struct ConnectionState {
    active: Option<ActiveConnection>,
}

enum ActiveConnection {
    Local(LocalConnection),
    Ssh(SshConnection),
}

struct LocalConnection {
    base_url: String,
    token: String,
    data_dir: PathBuf,
    child: Child,
}

struct SshConnection {
    base_url: String,
    token: Option<String>,
    tunnel: Child,
}

impl ConnectionManager {
    fn info(&self) -> DesktopConnectionInfo {
        let guard = self.0.lock().ok();
        let Some(guard) = guard.as_ref() else {
            return DesktopConnectionInfo { kind: DesktopConnectionKind::None, base_url: None, token: None };
        };
        match &guard.active {
            None => DesktopConnectionInfo { kind: DesktopConnectionKind::None, base_url: None, token: None },
            Some(ActiveConnection::Local(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                token: Some(c.token.clone()),
            },
            Some(ActiveConnection::Ssh(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Ssh,
                base_url: Some(c.base_url.clone()),
                token: c.token.clone(),
            },
        }
    }

    fn disconnect(&self) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Some(active) = guard.active.take() {
            match active {
                ActiveConnection::Local(c) => {
                    let _ = try_kill_child(c.child);
                }
                ActiveConnection::Ssh(c) => {
                    let _ = try_kill_child(c.tunnel);
                }
            }
        }
    }

    fn set_local(&self, base_url: String, token: String, data_dir: PathBuf, child: Child) {
        let mut guard = self.0.lock().expect("connection manager lock");
        guard.active = Some(ActiveConnection::Local(LocalConnection { base_url, token, data_dir, child }));
    }

    fn set_ssh(&self, base_url: String, token: Option<String>, tunnel: Child) {
        let mut guard = self.0.lock().expect("connection manager lock");
        guard.active = Some(ActiveConnection::Ssh(SshConnection { base_url, token, tunnel }));
    }

    fn daemon_request(&self, req: DesktopDaemonRequest) -> Result<DesktopHttpResponse> {
        if !req.path.starts_with("/api/") {
            return Err(anyhow!("only /api/* paths are supported"));
        }

        let (base_url, token) = {
            let guard = self.0.lock().context("connection manager lock")?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::Ssh(c) => (c.base_url.clone(), c.token.clone()),
            }
        };

        let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building http client")?;

        let method = req.method.trim().to_uppercase();
        let mut builder = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "DELETE" => client.delete(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            other => return Err(anyhow!("unsupported method: {other}")),
        };

        if let Some(t) = token.as_deref() {
            if !t.trim().is_empty() {
                builder = builder.bearer_auth(t);
            }
        }
        for (k, v) in req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        let res = builder.send().context("sending request")?;
        let status = res.status().as_u16();
        let content_type = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = res.text().unwrap_or_default();
        Ok(DesktopHttpResponse { status, body, content_type })
    }

    fn upload_blob(
        &self,
        bytes: Vec<u8>,
        mime_type: String,
        name: Option<String>,
    ) -> Result<serde_json::Value> {
        let (base_url, token) = {
            let guard = self.0.lock().context("connection manager lock")?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::Ssh(c) => (c.base_url.clone(), c.token.clone()),
            }
        };

        let url = format!("{}/api/blobs", base_url.trim_end_matches('/'));
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("building http client")?;

        let mut part = reqwest::blocking::multipart::Part::bytes(bytes);
        if let Some(n) = name.as_deref().filter(|s| !s.trim().is_empty()) {
            part = part.file_name(n.to_string());
        }
        part = part
            .mime_str(&mime_type)
            .context("invalid mime_type for multipart")?;

        let form = reqwest::blocking::multipart::Form::new().part("file", part);
        let mut req = client.post(url).multipart(form);
        if let Some(t) = token.as_deref() {
            if !t.trim().is_empty() {
                req = req.bearer_auth(t);
            }
        }
        let res = req.send().context("uploading blob")?;
        let status = res.status();
        let body = res.text().unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("blob upload failed ({status}): {body}"));
        }
        Ok(serde_json::from_str(&body).context("parsing blob upload response")?)
    }
}

fn to_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn derive_repo_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit('/').next()?;
    let name = last.trim_end_matches(".git").trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn pick_unused_local_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("binding ephemeral port")?;
    let port = listener.local_addr().context("reading local addr")?.port();
    Ok(port)
}

fn start_ssh_tunnel(
    host: &str,
    user: Option<&str>,
    local_port: u16,
    remote_port: u16,
) -> Result<Child> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let mut cmd = Command::new("ssh");
    cmd.arg("-N")
        .arg("-o")
        .arg("ExitOnForwardFailure=yes")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg("-L")
        .arg(format!("{local_port}:127.0.0.1:{remote_port}"))
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    cmd.spawn().context("spawning ssh tunnel")
}

fn start_remote_daemon_over_ssh(
    host: &str,
    user: Option<&str>,
    remote_port: u16,
    token: Option<&str>,
    remote_data_dir: Option<&str>,
) -> Result<()> {
    let target = match user {
        Some(u) if !u.trim().is_empty() => format!("{}@{}", u.trim(), host),
        _ => host.to_string(),
    };

    let data_dir = remote_data_dir.unwrap_or("~/.context");
    let mut serve_cmd = format!(
        "nohup context serve --bind 127.0.0.1:{remote_port} --data-dir {}",
        shell_escape(data_dir)
    );
    if let Some(t) = token {
        if !t.trim().is_empty() {
            serve_cmd.push_str(&format!(" --auth-token {}", shell_escape(t)));
        }
    }
    serve_cmd.push_str(" > ~/.context/logs/daemon.log 2>&1 &");

    let output = Command::new("ssh")
        .arg(target)
        .arg("sh")
        .arg("-lc")
        .arg(serve_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("starting remote daemon over ssh")?;
    if !output.status.success() {
        return Err(anyhow!(
            "ssh start failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn shell_escape(s: &str) -> String {
    // Minimal POSIX shell escaping for tokens: wrap in single quotes and escape inner single quotes.
    let inner = s.replace('\'', "'\"'\"'");
    format!("'{}'", inner)
}

fn probe_daemon_health(base_url: &str, token: Option<&str>) -> Result<()> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("building http client")?;
    let mut req = client.get(url);
    if let Some(t) = token {
        if !t.trim().is_empty() {
            req = req.bearer_auth(t);
        }
    }
    let res = req.send().context("requesting /api/health")?;
    res.error_for_status().context("health status")?;
    Ok(())
}

fn daemon_data_dir(app: &tauri::AppHandle) -> Result<PathBuf> {
    let root = app
        .path_resolver()
        .app_data_dir()
        .context("resolving app_data_dir")?;
    Ok(root.join("daemon"))
}

fn current_arch_token() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        std::env::consts::ARCH
    }
}

fn resource_bin(app: &tauri::AppHandle, name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(res) = app.path_resolver().resource_dir() {
        let bin_ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
        candidates.push(res.join("bin").join(format!("{name}{bin_ext}")));
        candidates.push(res.join(format!("{name}{bin_ext}")));

        let arch = current_arch_token();
        let prefix = format!("{name}-{arch}");
        for base in [res.join("bin"), res.clone()] {
            let Ok(entries) = std::fs::read_dir(&base) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for p in paths {
                if !p.is_file() {
                    continue;
                }
                let Some(file_name) = p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if !file_name.starts_with(&prefix) {
                    continue;
                }
                if !bin_ext.is_empty() && !file_name.ends_with(bin_ext) {
                    continue;
                }
                candidates.push(p);
            }
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

fn try_kill_child(mut child: Child) -> Result<()> {
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
