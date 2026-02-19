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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) remote_ctx_bin: Option<String>,
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
                remote_ctx_bin: None,
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
                remote_ctx_bin: None,
            },
            Some(ActiveConnection::Local(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                token: Some(c.token.clone()),
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
                remote_ctx_bin: None,
            },
            Some(ActiveConnection::LocalExternal(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                token: Some(c.token.clone()),
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
                remote_ctx_bin: None,
            },
            Some(ActiveConnection::Ssh(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Ssh,
                base_url: Some(c.base_url.clone()),
                token: c.token.clone(),
                host: Some(c.host.clone()),
                user: c.user.clone(),
                remote_port: Some(c.remote_port),
                remote_data_dir: c.remote_data_dir.clone(),
                remote_ctx_bin: c.remote_ctx_bin.clone(),
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
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if let Some(active) = guard.active.take() {
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
                ActiveConnection::LocalExternal(_) => {}
                ActiveConnection::Ssh(c) => {
                    let _ = try_kill_child(c.tunnel);
                }
            }
        }
    }

    pub(super) fn set_local(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
    ) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => {
                let _ = try_kill_child(child);
                return;
            }
        };
        guard.active = Some(ActiveConnection::Local(LocalConnection {
            base_url,
            token,
            child,
            systemd_scope,
        }));
    }

    pub(super) fn set_local_external(&self, base_url: String, token: String) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.active = Some(ActiveConnection::LocalExternal(LocalExternalConnection {
            base_url,
            token,
        }));
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
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => {
                let _ = try_kill_child(tunnel);
                return;
            }
        };
        guard.active = Some(ActiveConnection::Ssh(SshConnection {
            base_url,
            token,
            tunnel,
            host,
            user,
            remote_port,
            remote_data_dir,
            remote_ctx_bin,
        }));
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
        let res = builder.send().context("sending request")?;
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
