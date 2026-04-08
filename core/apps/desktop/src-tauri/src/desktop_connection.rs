use super::*;
pub(super) use ctx_desktop_ipc::{DesktopConnectionInfo, DesktopConnectionKind};

#[cfg(test)]
use std::cell::Cell;

#[derive(Debug, Clone)]
pub(super) struct SshConnectionTarget {
    pub(super) host: String,
    pub(super) user: Option<String>,
    pub(super) remote_port: u16,
    pub(super) remote_data_dir: Option<String>,
    pub(super) runtime: SshRuntimeMetadata,
}

#[derive(Debug, Clone)]
pub(super) struct SshRuntimeMetadata {
    pub(super) managed_ctx_bin: String,
    pub(super) active_ctx_bin: Option<String>,
    pub(super) ssh_password_once: Option<String>,
    pub(super) admin_password_once: Option<String>,
}

#[tauri::command]
pub(super) fn desktop_get_connection(
    state: tauri::State<ConnectionManager>,
) -> DesktopConnectionInfo {
    state.info()
}

#[tauri::command]
pub(super) fn desktop_disconnect(state: tauri::State<ConnectionManager>) -> Result<(), String> {
    state.disconnect();
    Ok(())
}

#[cfg_attr(not(feature = "automation"), allow(dead_code))]
#[cfg_attr(not(feature = "automation"), allow(dead_code))]
fn demo_commands_enabled() -> bool {
    fn parse_boolish(value: &str) -> Option<bool> {
        match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    std::env::var("CTX_DESKTOP_ALLOW_DEMO_COMMANDS")
        .ok()
        .as_deref()
        .and_then(parse_boolish)
        .unwrap_or(false)
}

#[cfg_attr(not(feature = "automation"), allow(dead_code))]
#[derive(Debug, Deserialize)]
pub(super) struct DesktopDemoConnectionRequest {
    pub(super) base_url: String,
    pub(super) token: String,
}

#[tauri::command]
pub(super) fn desktop_set_demo_connection(
    state: tauri::State<ConnectionManager>,
    req: DesktopDemoConnectionRequest,
) -> Result<DesktopConnectionInfo, String> {
    #[cfg(feature = "automation")]
    {
        if !demo_commands_enabled() {
            return Err(
                "desktop_set_demo_connection requires CTX_DESKTOP_ALLOW_DEMO_COMMANDS=1"
                    .to_string(),
            );
        }
        let base_url = req.base_url.trim().to_string();
        if base_url.is_empty() {
            return Err("base_url is required".to_string());
        }
        let token = req.token.trim().to_string();
        if token.is_empty() {
            return Err("token is required".to_string());
        }
        state.set_local_attached(base_url, token, None, LocalConnectionSource::EnvOverride);
        return Ok(state.info());
    }

    #[cfg(not(feature = "automation"))]
    {
        let _ = state;
        let _ = req;
        Err("desktop_set_demo_connection is automation-only".to_string())
    }
}

#[derive(Default)]
pub(super) struct ConnectionManager(std::sync::Mutex<ConnectionState>);

#[derive(Default)]
struct ConnectionState {
    active: Option<ActiveConnection>,
}

enum ActiveConnection {
    Local(LocalConnection),
    Ssh(SshConnection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalConnectionSource {
    EnvOverride,
    ExistingCompatibleDaemon,
    SpawnedByDesktop,
}

struct LocalConnection {
    base_url: String,
    token: String,
    daemon_pid: Option<u32>,
    source: LocalConnectionSource,
    ownership: LocalConnectionOwnership,
    http_client: std::sync::OnceLock<reqwest::blocking::Client>,
}

enum LocalConnectionOwnership {
    OwnedChild { child: Child, systemd_scope: bool },
    OwnedPid { pid: u32 },
    UnownedExternal,
}

struct SshConnection {
    base_url: String,
    token: Option<String>,
    tunnel: Child,
    host: String,
    user: Option<String>,
    remote_port: u16,
    remote_data_dir: Option<String>,
    runtime: SshRuntimeMetadata,
    http_client: std::sync::OnceLock<reqwest::blocking::Client>,
}

#[cfg(test)]
thread_local! {
    static CONNECTION_HTTP_CLIENT_BUILD_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn reset_connection_http_client_build_count() {
    CONNECTION_HTTP_CLIENT_BUILD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
fn connection_http_client_build_count() -> usize {
    CONNECTION_HTTP_CLIENT_BUILD_COUNT.with(Cell::get)
}

fn build_connection_http_client() -> Result<reqwest::blocking::Client> {
    #[cfg(test)]
    CONNECTION_HTTP_CLIENT_BUILD_COUNT.with(|count| count.set(count.get() + 1));
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .context("building http client")
}

fn get_connection_http_client(
    client: &std::sync::OnceLock<reqwest::blocking::Client>,
) -> Result<reqwest::blocking::Client> {
    if let Some(existing) = client.get() {
        return Ok(existing.clone());
    }
    let built = build_connection_http_client()?;
    let _ = client.set(built);
    client
        .get()
        .cloned()
        .context("connection http client missing after initialization")
}

fn stop_owned_local_daemon_pid(base_url: &str, pid: u32) -> Result<()> {
    stop_systemd_scope("ctx-daemon");
    if let Some(scope) = systemd_scope_for_local_daemon_url(base_url) {
        stop_systemd_scope(&scope);
    }
    let graceful_err = terminate_pid(pid, false).err();
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(3)).is_ok() {
        return Ok(());
    }

    let force_err = terminate_pid(pid, true).err();
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(2)).is_ok() {
        return Ok(());
    }

    let mut details = Vec::new();
    if let Some(err) = graceful_err {
        details.push(format!("graceful terminate failed: {err:#}"));
    }
    if let Some(err) = force_err {
        details.push(format!("force terminate failed: {err:#}"));
    }
    if details.is_empty() {
        anyhow::bail!("local daemon pid {pid} did not exit");
    }
    anyhow::bail!(
        "local daemon pid {pid} did not exit ({})",
        details.join("; ")
    );
}

fn stop_owned_local_daemon_child(
    base_url: &str,
    mut child: Child,
    systemd_scope: bool,
) -> Result<()> {
    if systemd_scope {
        stop_systemd_scope("ctx-daemon");
        if let Some(scope) = systemd_scope_for_local_daemon_url(base_url) {
            stop_systemd_scope(&scope);
        }
    }

    let pid = child.id();
    let graceful_err = terminate_pid(pid, false).err();
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(3)).is_ok() {
        let _ = child.wait();
        return Ok(());
    }
    if let Some(err) = graceful_err {
        eprintln!("failed to gracefully terminate local daemon child {pid}: {err:#}");
    }

    try_kill_child(child)
}

fn cleanup_active_connection_result(active: ActiveConnection) -> Result<()> {
    match active {
        ActiveConnection::Local(c) => match c.ownership {
            LocalConnectionOwnership::OwnedChild {
                child,
                systemd_scope,
            } => stop_owned_local_daemon_child(&c.base_url, child, systemd_scope),
            LocalConnectionOwnership::OwnedPid { pid } => {
                stop_owned_local_daemon_pid(&c.base_url, pid)
            }
            LocalConnectionOwnership::UnownedExternal => Ok(()),
        },
        ActiveConnection::Ssh(c) => try_kill_child(c.tunnel),
    }
}

fn cleanup_active_connection(active: ActiveConnection) {
    let _ = cleanup_active_connection_result(active);
}

fn build_ssh_connection(
    base_url: String,
    token: Option<String>,
    tunnel: Child,
    host: String,
    user: Option<String>,
    remote_port: u16,
    remote_data_dir: Option<String>,
    runtime: SshRuntimeMetadata,
) -> ActiveConnection {
    ActiveConnection::Ssh(SshConnection {
        base_url,
        token,
        tunnel,
        host,
        user,
        remote_port,
        remote_data_dir,
        runtime,
        http_client: std::sync::OnceLock::new(),
    })
}

fn should_preserve_local_handoff(
    base_url: &str,
    token: &str,
    daemon_pid: Option<u32>,
    previous_base_url: &str,
    previous_token: &str,
    previous_daemon_pid: Option<u32>,
) -> bool {
    daemon_pid.is_some()
        && daemon_pid == previous_daemon_pid
        && previous_base_url == base_url
        && previous_token == token
}

impl ConnectionManager {
    pub(super) fn info(&self) -> DesktopConnectionInfo {
        let guard = self.0.lock().ok();
        let Some(guard) = guard.as_ref() else {
            return DesktopConnectionInfo {
                kind: DesktopConnectionKind::None,
                base_url: None,
                token: None,
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
            };
        };
        match &guard.active {
            None => DesktopConnectionInfo {
                kind: DesktopConnectionKind::None,
                base_url: None,
                token: None,
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
            },
            Some(ActiveConnection::Local(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                token: Some(c.token.clone()),
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
            },
            Some(ActiveConnection::Ssh(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Ssh,
                base_url: Some(c.base_url.clone()),
                token: c.token.clone(),
                host: Some(c.host.clone()),
                user: c.user.clone(),
                remote_port: Some(c.remote_port),
                remote_data_dir: c.remote_data_dir.clone(),
            },
        }
    }

    pub(super) fn is_remote(&self) -> bool {
        let guard = self.0.lock().ok();
        matches!(
            guard.as_ref().and_then(|g| g.active.as_ref()),
            Some(ActiveConnection::Ssh(_))
        )
    }

    pub(super) fn disconnect(&self) {
        let active = {
            let mut guard = match self.0.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            guard.active.take()
        };
        if let Some(active) = active {
            cleanup_active_connection(active);
        }
    }

    pub(super) fn disconnect_for_local_restart(&self) -> Result<()> {
        let active = {
            let mut guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            guard.active.take()
        };
        if let Some(active) = active {
            cleanup_active_connection_result(active)?;
        }
        Ok(())
    }

    pub(super) fn should_disconnect_for_local_restart(&self) -> bool {
        let guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        matches!(
            guard.active.as_ref(),
            Some(ActiveConnection::Local(LocalConnection {
                ownership: LocalConnectionOwnership::OwnedChild { .. }
                    | LocalConnectionOwnership::OwnedPid { .. },
                ..
            }))
        )
    }

    pub(super) fn set_local(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
    ) {
        let daemon_pid = Some(child.id());
        let previous = {
            let mut guard = match self.0.lock() {
                Ok(g) => g,
                Err(_) => {
                    let _ = try_kill_child(child);
                    return;
                }
            };
            guard
                .active
                .replace(ActiveConnection::Local(LocalConnection {
                    base_url,
                    token,
                    daemon_pid,
                    source: LocalConnectionSource::SpawnedByDesktop,
                    ownership: LocalConnectionOwnership::OwnedChild {
                        child,
                        systemd_scope,
                    },
                    http_client: std::sync::OnceLock::new(),
                }))
        };
        if let Some(previous) = previous {
            cleanup_active_connection(previous);
        }
    }

    pub(super) fn set_local_attached(
        &self,
        base_url: String,
        token: String,
        daemon_pid: Option<u32>,
        source: LocalConnectionSource,
    ) {
        let previous = {
            let mut next = LocalConnection {
                base_url,
                token,
                daemon_pid,
                source,
                ownership: match (source, daemon_pid) {
                    (LocalConnectionSource::ExistingCompatibleDaemon, Some(pid)) => {
                        LocalConnectionOwnership::OwnedPid { pid }
                    }
                    _ => LocalConnectionOwnership::UnownedExternal,
                },
                http_client: std::sync::OnceLock::new(),
            };
            let mut guard = match self.0.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let previous = guard.active.take();
            let previous = match previous {
                Some(ActiveConnection::Local(c))
                    if should_preserve_local_handoff(
                        &next.base_url,
                        &next.token,
                        next.daemon_pid,
                        &c.base_url,
                        &c.token,
                        c.daemon_pid,
                    ) =>
                {
                    next.ownership = c.ownership;
                    next.source = c.source;
                    next.http_client = c.http_client;
                    None
                }
                other => other,
            };
            guard.active = Some(ActiveConnection::Local(next));
            previous
        };
        if let Some(previous) = previous {
            cleanup_active_connection(previous);
        }
    }

    #[cfg(test)]
    pub(super) fn set_ssh(
        &self,
        base_url: String,
        token: Option<String>,
        tunnel: Child,
        host: String,
        user: Option<String>,
        remote_port: u16,
        remote_data_dir: Option<String>,
        runtime: SshRuntimeMetadata,
    ) {
        let previous = self.replace_with_ssh(
            base_url,
            token,
            tunnel,
            host,
            user,
            remote_port,
            remote_data_dir,
            runtime,
        );
        if let Some(previous) = previous {
            cleanup_active_connection(previous);
        }
    }

    pub(super) async fn set_ssh_with_blocking_cleanup(
        &self,
        base_url: String,
        token: Option<String>,
        tunnel: Child,
        host: String,
        user: Option<String>,
        remote_port: u16,
        remote_data_dir: Option<String>,
        runtime: SshRuntimeMetadata,
    ) -> Result<(), String> {
        let previous = self.replace_with_ssh(
            base_url,
            token,
            tunnel,
            host,
            user,
            remote_port,
            remote_data_dir,
            runtime,
        );
        if let Some(previous) = previous {
            tauri::async_runtime::spawn_blocking(move || {
                cleanup_active_connection(previous);
            })
            .await
            .map_err(|e| {
                format!("failed to clean up previous desktop connection after ssh handoff: {e}")
            })?;
        }
        Ok(())
    }

    fn replace_with_ssh(
        &self,
        base_url: String,
        token: Option<String>,
        tunnel: Child,
        host: String,
        user: Option<String>,
        remote_port: u16,
        remote_data_dir: Option<String>,
        runtime: SshRuntimeMetadata,
    ) -> Option<ActiveConnection> {
        let next = build_ssh_connection(
            base_url,
            token,
            tunnel,
            host,
            user,
            remote_port,
            remote_data_dir,
            runtime,
        );
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => {
                if let ActiveConnection::Ssh(ssh) = next {
                    let _ = try_kill_child(ssh.tunnel);
                }
                return None;
            }
        };
        guard.active.replace(next)
    }

    pub(super) fn ssh_target(&self) -> Result<SshConnectionTarget> {
        let guard = self
            .0
            .lock()
            .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
        let Some(active) = guard.active.as_ref() else {
            anyhow::bail!("not connected (open a workspace first)");
        };
        let ActiveConnection::Ssh(c) = active else {
            anyhow::bail!("current connection is not SSH");
        };
        Ok(SshConnectionTarget {
            host: c.host.clone(),
            user: c.user.clone(),
            remote_port: c.remote_port,
            remote_data_dir: c.remote_data_dir.clone(),
            runtime: c.runtime.clone(),
        })
    }

    pub(super) fn update_ssh_token(&self, token: String) -> Result<()> {
        let mut guard = self
            .0
            .lock()
            .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
        let Some(active) = guard.active.as_mut() else {
            anyhow::bail!("not connected (open a workspace first)");
        };
        let ActiveConnection::Ssh(c) = active else {
            anyhow::bail!("current connection is not SSH");
        };
        c.token = Some(token);
        Ok(())
    }

    pub(super) fn update_ssh_runtime(&self, runtime: SshRuntimeMetadata) -> Result<()> {
        let mut guard = self
            .0
            .lock()
            .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
        let Some(active) = guard.active.as_mut() else {
            anyhow::bail!("not connected (open a workspace first)");
        };
        let ActiveConnection::Ssh(c) = active else {
            anyhow::bail!("current connection is not SSH");
        };
        c.runtime = runtime;
        Ok(())
    }

    pub(super) fn daemon_request(&self, req: DesktopDaemonRequest) -> Result<DesktopHttpResponse> {
        if !req.path.starts_with("/api/") {
            return Err(anyhow!("only /api/* paths are supported"));
        }

        let (base_url, token, client) = {
            let guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (
                    c.base_url.clone(),
                    Some(c.token.clone()),
                    get_connection_http_client(&c.http_client)?,
                ),
                ActiveConnection::Ssh(c) => (
                    c.base_url.clone(),
                    c.token.clone(),
                    get_connection_http_client(&c.http_client)?,
                ),
            }
        };

        let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
        let method = req.method.trim().to_uppercase();
        let mut builder = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "DELETE" => client.delete(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            other => return Err(anyhow!("unsupported method: {other}")),
        }
        .timeout(Duration::from_secs(10 * 60));

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
        let res = builder
            .send()
            .with_context(|| format!("sending request {method} {url}"))?;
        let status = res.status().as_u16();
        let content_type = res
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = res.text().unwrap_or_default();
        Ok(DesktopHttpResponse {
            status,
            body,
            content_type,
        })
    }

    pub(super) fn upload_blob(
        &self,
        bytes: Vec<u8>,
        mime_type: String,
        name: Option<String>,
    ) -> Result<serde_json::Value> {
        let (base_url, token, client) = {
            let guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (
                    c.base_url.clone(),
                    Some(c.token.clone()),
                    get_connection_http_client(&c.http_client)?,
                ),
                ActiveConnection::Ssh(c) => (
                    c.base_url.clone(),
                    c.token.clone(),
                    get_connection_http_client(&c.http_client)?,
                ),
            }
        };
        let url = format!("{}/api/blobs", base_url.trim_end_matches('/'));
        let mut part = reqwest::blocking::multipart::Part::bytes(bytes);
        if let Some(n) = name.as_deref().filter(|s| !s.trim().is_empty()) {
            part = part.file_name(n.to_string());
        }
        part = part
            .mime_str(&mime_type)
            .context("invalid mime_type for multipart")?;
        let form = reqwest::blocking::multipart::Form::new().part("file", part);
        let mut req = client
            .post(url)
            .timeout(Duration::from_secs(60))
            .multipart(form);
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

#[cfg(test)]
#[path = "desktop_connection/connection_manager_tests.rs"]
mod connection_manager_tests;
