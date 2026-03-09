use super::*;

#[cfg(test)]
use std::cell::Cell;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DesktopConnectionKind {
    None,
    Local,
    Ssh,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct DesktopConnectionInfo {
    pub(super) kind: DesktopConnectionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) remote_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) remote_data_dir: Option<String>,
}

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

fn cleanup_active_connection_result(active: ActiveConnection) -> Result<()> {
    match active {
        ActiveConnection::Local(c) => match c.ownership {
            LocalConnectionOwnership::OwnedChild {
                child,
                systemd_scope,
            } => {
                if systemd_scope {
                    stop_systemd_scope("ctx-daemon");
                    if let Some(scope) = systemd_scope_for_local_daemon_url(&c.base_url) {
                        stop_systemd_scope(&scope);
                    }
                }
                try_kill_child(child)
            }
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
        let previous = {
            let mut guard = match self.0.lock() {
                Ok(g) => g,
                Err(_) => {
                    let _ = try_kill_child(tunnel);
                    return;
                }
            };
            guard.active.replace(ActiveConnection::Ssh(SshConnection {
                base_url,
                token,
                tunnel,
                host,
                user,
                remote_port,
                remote_data_dir,
                runtime,
                http_client: std::sync::OnceLock::new(),
            }))
        };
        if let Some(previous) = previous {
            cleanup_active_connection(previous);
        }
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
        let mut req = client.post(url).timeout(Duration::from_secs(60)).multipart(form);
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
mod connection_manager_tests {
    use super::*;

    #[cfg(unix)]
    fn spawn_detached_sleep_pid() -> u32 {
        let output = Command::new("sh")
            .arg("-c")
            .arg("sleep 30 >/dev/null 2>&1 & echo $!")
            .output()
            .expect("spawn detached sleep");
        assert!(
            output.status.success(),
            "detached sleep spawn failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .trim()
            .parse::<u32>()
            .expect("parse detached sleep pid")
    }

    #[cfg(unix)]
    fn pid_is_alive(pid: u32) -> bool {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if !pid_is_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
        !pid_is_alive(pid)
    }

    #[cfg(unix)]
    fn spawn_tokio_sleep_child() -> Child {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30 >/dev/null 2>&1")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().expect("spawn tokio sleep child")
    }

    #[cfg(unix)]
    fn spawn_term_trap_child(term_marker: &Path) -> Child {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("trap 'printf term > \"$CTX_TEST_TERM_MARKER\"; exit 0' TERM; while :; do sleep 1; done")
            .env("CTX_TEST_TERM_MARKER", term_marker)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().expect("spawn term trap child")
    }

    #[test]
    #[cfg(unix)]
    fn disconnect_stops_reattached_compatible_local_daemon_pid() {
        let pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(pid),
            "sleep process should be alive before disconnect"
        );

        let manager = ConnectionManager::default();
        manager.set_local_attached(
            "http://127.0.0.1:65531".to_string(),
            "token".to_string(),
            Some(pid),
            LocalConnectionSource::ExistingCompatibleDaemon,
        );
        manager.disconnect();

        assert!(
            wait_for_pid_exit(pid, Duration::from_secs(3)),
            "reattached compatible local daemon pid {pid} should be terminated on disconnect"
        );
    }

    #[test]
    #[cfg(unix)]
    fn disconnect_reattached_compatible_local_daemon_prefers_graceful_shutdown() {
        let term_marker =
            std::env::temp_dir().join(format!("ctx-daemon-term-marker-{}", uuid::Uuid::new_v4()));
        let mut child = spawn_term_trap_child(&term_marker);
        let pid = child.id();
        assert!(
            pid_is_alive(pid),
            "term trap child should be alive before disconnect"
        );

        let manager = ConnectionManager::default();
        manager.set_local_attached(
            "http://127.0.0.1:65532".to_string(),
            "token".to_string(),
            Some(pid),
            LocalConnectionSource::ExistingCompatibleDaemon,
        );
        manager.disconnect();

        let status = child.wait().expect("wait term trap child");
        assert!(
            status.success(),
            "term trap child should exit successfully after graceful shutdown: {status:?}"
        );
        let marker = std::fs::read_to_string(&term_marker)
            .expect("term marker should be written by graceful TERM handler");
        assert_eq!(marker, "term");
        std::fs::remove_file(&term_marker).ok();
    }

    #[test]
    #[cfg(unix)]
    fn disconnect_does_not_stop_env_override_local_daemon_pid() {
        let pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(pid),
            "sleep process should be alive before disconnect"
        );

        let manager = ConnectionManager::default();
        manager.set_local_attached(
            "http://127.0.0.1:65530".to_string(),
            "token".to_string(),
            Some(pid),
            LocalConnectionSource::EnvOverride,
        );
        manager.disconnect();

        assert!(
            pid_is_alive(pid),
            "env override local daemon pid {pid} must not be terminated by disconnect"
        );
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .output();
    }

    #[test]
    #[cfg(unix)]
    fn replacing_env_override_local_connection_leaves_previous_pid_running() {
        let previous_pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(previous_pid),
            "previous pid should start alive"
        );

        let manager = ConnectionManager::default();
        manager.set_local_attached(
            "http://127.0.0.1:65527".to_string(),
            "token".to_string(),
            Some(previous_pid),
            LocalConnectionSource::EnvOverride,
        );
        manager.set_local_attached(
            "http://127.0.0.1:65526".to_string(),
            "token".to_string(),
            None,
            LocalConnectionSource::EnvOverride,
        );

        assert!(
            pid_is_alive(previous_pid),
            "replacing an env override pid {previous_pid} must not terminate it"
        );
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(previous_pid.to_string())
            .output();
    }

    #[test]
    #[cfg(unix)]
    fn replacing_local_connection_stops_previous_child() {
        let previous = spawn_tokio_sleep_child();
        let previous_pid = previous.id();
        assert!(
            pid_is_alive(previous_pid),
            "previous local child should start alive"
        );

        let next = spawn_tokio_sleep_child();
        let next_pid = next.id();
        assert!(
            pid_is_alive(next_pid),
            "next local child should start alive"
        );

        let manager = ConnectionManager::default();
        manager.set_local(
            "http://127.0.0.1:65525".to_string(),
            "token".to_string(),
            previous,
            false,
        );
        manager.set_local(
            "http://127.0.0.1:65524".to_string(),
            "token".to_string(),
            next,
            false,
        );

        assert!(
            wait_for_pid_exit(previous_pid, Duration::from_secs(3)),
            "replaced local child {previous_pid} should be terminated"
        );
        assert!(
            pid_is_alive(next_pid),
            "replacement local child {next_pid} should remain alive until disconnect"
        );

        manager.disconnect();
        assert!(
            wait_for_pid_exit(next_pid, Duration::from_secs(3)),
            "active replacement local child {next_pid} should be terminated on disconnect"
        );
    }

    #[test]
    #[cfg(unix)]
    fn reattaching_to_same_owned_local_daemon_preserves_process() {
        let child = spawn_tokio_sleep_child();
        let pid = child.id();
        assert!(pid_is_alive(pid), "local child should start alive");

        let manager = ConnectionManager::default();
        manager.set_local(
            "http://127.0.0.1:65524".to_string(),
            "token".to_string(),
            child,
            false,
        );
        manager.set_local_attached(
            "http://127.0.0.1:65524".to_string(),
            "token".to_string(),
            Some(pid),
            LocalConnectionSource::ExistingCompatibleDaemon,
        );

        assert!(
            pid_is_alive(pid),
            "same-daemon handoff must not kill the process being reattached"
        );

        manager.disconnect();
        assert!(
            wait_for_pid_exit(pid, Duration::from_secs(3)),
            "reattached owned local daemon pid {pid} should still be terminated on disconnect"
        );
    }

    #[test]
    #[cfg(unix)]
    fn replacing_ssh_connection_stops_previous_tunnel() {
        let previous = spawn_tokio_sleep_child();
        let previous_pid = previous.id();
        assert!(
            pid_is_alive(previous_pid),
            "previous ssh tunnel should start alive"
        );

        let next = spawn_tokio_sleep_child();
        let next_pid = next.id();
        assert!(pid_is_alive(next_pid), "next ssh tunnel should start alive");

        let manager = ConnectionManager::default();
        manager.set_ssh(
            "http://127.0.0.1:65523".to_string(),
            Some("token".to_string()),
            previous,
            "example.test".to_string(),
            Some("dev".to_string()),
            22,
            Some("/tmp/ctx".to_string()),
            SshRuntimeMetadata {
                managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
                active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            },
        );
        manager.set_ssh(
            "http://127.0.0.1:65522".to_string(),
            Some("token".to_string()),
            next,
            "example.test".to_string(),
            Some("dev".to_string()),
            22,
            Some("/tmp/ctx".to_string()),
            SshRuntimeMetadata {
                managed_ctx_bin: "~/.ctx/bin/ctx".to_string(),
                active_ctx_bin: Some("~/.ctx/bin/ctx".to_string()),
            },
        );

        assert!(
            wait_for_pid_exit(previous_pid, Duration::from_secs(3)),
            "replaced ssh tunnel {previous_pid} should be terminated"
        );
        assert!(
            pid_is_alive(next_pid),
            "replacement ssh tunnel {next_pid} should remain alive until disconnect"
        );

        manager.disconnect();
        assert!(
            wait_for_pid_exit(next_pid, Duration::from_secs(3)),
            "active replacement ssh tunnel {next_pid} should be terminated on disconnect"
        );
    }

    #[test]
    fn daemon_request_error_includes_method_and_url_context() {
        let manager = ConnectionManager::default();
        manager.set_local_attached(
            "http://127.0.0.1:65535".to_string(),
            "token".to_string(),
            None,
            LocalConnectionSource::ExistingCompatibleDaemon,
        );
        let err = manager
            .daemon_request(DesktopDaemonRequest {
                method: "GET".to_string(),
                path: "/api/health".to_string(),
                body: None,
                headers: Vec::new(),
            })
            .expect_err("request should fail on closed port");
        let message = format!("{err:#}");
        assert!(
            message.contains("sending request GET http://127.0.0.1:65535/api/health"),
            "expected method/url context in error, got: {message}"
        );
    }

    #[test]
    fn daemon_request_reuses_connection_http_client() {
        reset_connection_http_client_build_count();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buf = [0_u8; 1024];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                std::io::Write::write_all(
                    &mut stream,
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                )
                .expect("write response");
            }
        });

        let manager = ConnectionManager::default();
        manager.set_local_attached(
            format!("http://{}", addr),
            "token".to_string(),
            None,
            LocalConnectionSource::EnvOverride,
        );

        for _ in 0..2 {
            let response = manager
                .daemon_request(DesktopDaemonRequest {
                    method: "GET".to_string(),
                    path: "/api/health".to_string(),
                    body: None,
                    headers: Vec::new(),
                })
                .expect("daemon request succeeds");
            assert_eq!(response.status, 200);
        }

        server.join().expect("join test server");
        assert_eq!(connection_http_client_build_count(), 1);
    }
}
