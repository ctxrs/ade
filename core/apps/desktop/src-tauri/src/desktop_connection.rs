use super::*;

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
    pub(super) remote_ctx_bin: Option<String>,
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
    LocalExternal(LocalExternalConnection),
    Ssh(SshConnection),
}

struct LocalConnection {
    base_url: String,
    token: String,
    child: Child,
    systemd_scope: bool,
}

struct LocalExternalConnection {
    base_url: String,
    token: String,
    daemon_pid: Option<u32>,
    owns_lifecycle: bool,
    child: Option<Child>,
}

struct SshConnection {
    base_url: String,
    token: Option<String>,
    tunnel: Child,
    host: String,
    user: Option<String>,
    remote_port: u16,
    remote_data_dir: Option<String>,
    remote_ctx_bin: Option<String>,
}

fn cleanup_active_connection(active: ActiveConnection) {
    match active {
        ActiveConnection::Local(c) => {
            if c.systemd_scope {
                stop_systemd_scope("ctx-daemon");
                if let Some(scope) = systemd_scope_for_local_daemon_url(&c.base_url) {
                    stop_systemd_scope(&scope);
                }
            }
            let _ = try_kill_child(c.child);
        }
        ActiveConnection::LocalExternal(c) => {
            if c.owns_lifecycle {
                stop_systemd_scope("ctx-daemon");
                if let Some(scope) = systemd_scope_for_local_daemon_url(&c.base_url) {
                    stop_systemd_scope(&scope);
                }
                if let Some(child) = c.child {
                    let _ = try_kill_child(child);
                } else if let Some(pid) = c.daemon_pid {
                    let _ = stop_local_daemon_pid(pid);
                }
            }
        }
        ActiveConnection::Ssh(c) => {
            let _ = try_kill_child(c.tunnel);
        }
    }
}

fn should_preserve_local_handoff(
    base_url: &str,
    token: &str,
    daemon_pid: Option<u32>,
    owns_lifecycle: bool,
    previous_base_url: &str,
    previous_token: &str,
    previous_daemon_pid: Option<u32>,
) -> bool {
    if !owns_lifecycle {
        return false;
    }
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
            Some(ActiveConnection::LocalExternal(c)) => DesktopConnectionInfo {
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

    pub(super) fn set_local(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
    ) {
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
                    child,
                    systemd_scope,
                }))
        };
        if let Some(previous) = previous {
            cleanup_active_connection(previous);
        }
    }

    pub(super) fn set_local_external(
        &self,
        base_url: String,
        token: String,
        daemon_pid: Option<u32>,
        owns_lifecycle: bool,
    ) {
        let previous = {
            let mut next = LocalExternalConnection {
                base_url,
                token,
                daemon_pid,
                owns_lifecycle,
                child: None,
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
                        next.owns_lifecycle,
                        &c.base_url,
                        &c.token,
                        Some(c.child.id()),
                    ) =>
                {
                    next.child = Some(c.child);
                    None
                }
                Some(ActiveConnection::LocalExternal(c))
                    if should_preserve_local_handoff(
                        &next.base_url,
                        &next.token,
                        next.daemon_pid,
                        next.owns_lifecycle,
                        &c.base_url,
                        &c.token,
                        c.daemon_pid,
                    ) =>
                {
                    next.child = c.child;
                    None
                }
                other => other,
            };
            guard.active = Some(ActiveConnection::LocalExternal(next));
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
        remote_ctx_bin: Option<String>,
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
                remote_ctx_bin,
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
            remote_ctx_bin: c.remote_ctx_bin.clone(),
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

    pub(super) fn daemon_request(&self, req: DesktopDaemonRequest) -> Result<DesktopHttpResponse> {
        if !req.path.starts_with("/api/") {
            return Err(anyhow!("only /api/* paths are supported"));
        }

        let (base_url, token) = {
            let guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::LocalExternal(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::Ssh(c) => (c.base_url.clone(), c.token.clone()),
            }
        };

        let url = format!("{}{}", base_url.trim_end_matches('/'), req.path);
        // Some daemon operations (notably container provisioning on first run) can legitimately
        // take minutes. Keep a short connect timeout so a dead daemon fails fast, but allow
        // long-running requests to complete.
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10 * 60))
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
        let (base_url, token) = {
            let guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            let active = guard
                .active
                .as_ref()
                .ok_or_else(|| anyhow!("not connected (open a workspace first)"))?;
            match active {
                ActiveConnection::Local(c) => (c.base_url.clone(), Some(c.token.clone())),
                ActiveConnection::LocalExternal(c) => (c.base_url.clone(), Some(c.token.clone())),
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

    #[test]
    #[cfg(unix)]
    fn disconnect_stops_owned_local_external_pid() {
        let pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(pid),
            "sleep process should be alive before disconnect"
        );

        let manager = ConnectionManager::default();
        manager.set_local_external(
            "http://127.0.0.1:65531".to_string(),
            "token".to_string(),
            Some(pid),
            true,
        );
        manager.disconnect();

        assert!(
            wait_for_pid_exit(pid, Duration::from_secs(3)),
            "owned local external pid {pid} should be terminated on disconnect"
        );
    }

    #[test]
    #[cfg(unix)]
    fn disconnect_does_not_stop_unowned_local_external_pid() {
        let pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(pid),
            "sleep process should be alive before disconnect"
        );

        let manager = ConnectionManager::default();
        manager.set_local_external(
            "http://127.0.0.1:65530".to_string(),
            "token".to_string(),
            Some(pid),
            false,
        );
        manager.disconnect();

        assert!(
            pid_is_alive(pid),
            "unowned local external pid {pid} must not be terminated by disconnect"
        );
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(pid.to_string())
            .output();
    }

    #[test]
    #[cfg(unix)]
    fn replacing_owned_local_external_connection_stops_previous_pid() {
        let previous_pid = spawn_detached_sleep_pid();
        let next_pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(previous_pid),
            "previous pid should start alive"
        );
        assert!(pid_is_alive(next_pid), "next pid should start alive");

        let manager = ConnectionManager::default();
        manager.set_local_external(
            "http://127.0.0.1:65529".to_string(),
            "token".to_string(),
            Some(previous_pid),
            true,
        );
        manager.set_local_external(
            "http://127.0.0.1:65528".to_string(),
            "token".to_string(),
            Some(next_pid),
            true,
        );

        assert!(
            wait_for_pid_exit(previous_pid, Duration::from_secs(3)),
            "replaced owned local external pid {previous_pid} should be terminated"
        );
        assert!(
            pid_is_alive(next_pid),
            "replacement pid {next_pid} should remain alive until disconnect"
        );

        manager.disconnect();
        assert!(
            wait_for_pid_exit(next_pid, Duration::from_secs(3)),
            "active replacement pid {next_pid} should be terminated on disconnect"
        );
    }

    #[test]
    #[cfg(unix)]
    fn replacing_unowned_local_external_connection_leaves_previous_pid_running() {
        let previous_pid = spawn_detached_sleep_pid();
        assert!(
            pid_is_alive(previous_pid),
            "previous pid should start alive"
        );

        let manager = ConnectionManager::default();
        manager.set_local_external(
            "http://127.0.0.1:65527".to_string(),
            "token".to_string(),
            Some(previous_pid),
            false,
        );
        manager.set_local_external(
            "http://127.0.0.1:65526".to_string(),
            "token".to_string(),
            None,
            false,
        );

        assert!(
            pid_is_alive(previous_pid),
            "replacing an unowned local external pid {previous_pid} must not terminate it"
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
        manager.set_local_external(
            "http://127.0.0.1:65524".to_string(),
            "token".to_string(),
            Some(pid),
            true,
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
            None,
        );
        manager.set_ssh(
            "http://127.0.0.1:65522".to_string(),
            Some("token".to_string()),
            next,
            "example.test".to_string(),
            Some("dev".to_string()),
            22,
            Some("/tmp/ctx".to_string()),
            None,
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
        manager.set_local_external(
            "http://127.0.0.1:65535".to_string(),
            "token".to_string(),
            None,
            false,
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
}
