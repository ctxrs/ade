use super::http_client::get_connection_http_client;
use super::lifecycle::{
    build_ssh_connection, cleanup_active_connection, cleanup_active_connection_result_for_restart,
    should_preserve_local_handoff,
};
use super::types::{
    log_local_connection_established, log_ssh_connection_established, ActiveConnection,
    ConnectionIntent, ConnectionState, LocalConnection, LocalConnectionOwnership,
    LocalConnectionSource, SshConnectionTarget, SshRemoteUpdateStatus, SshRuntimeMetadata,
};
use super::*;

pub(crate) struct ConnectionManager(pub(super) std::sync::Mutex<ConnectionState>);

impl Default for ConnectionManager {
    fn default() -> Self {
        Self(std::sync::Mutex::new(ConnectionState::default()))
    }
}

impl ConnectionManager {
    pub(crate) fn info(&self) -> DesktopConnectionInfo {
        let guard = self.0.lock().ok();
        let Some(guard) = guard.as_ref() else {
            return DesktopConnectionInfo {
                kind: DesktopConnectionKind::None,
                base_url: None,
                intent: ConnectionIntent::ExplicitDisconnected.as_ipc(),
                local_auto_bootstrap_allowed: false,
                token: None,
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
                remote_update_message: None,
                remote_update_state: None,
            };
        };
        let intent = guard.intent.as_ipc();
        let local_auto_bootstrap_allowed = guard.local_auto_bootstrap_allowed();
        match &guard.active {
            None => DesktopConnectionInfo {
                kind: DesktopConnectionKind::None,
                base_url: None,
                intent,
                local_auto_bootstrap_allowed,
                token: None,
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
                remote_update_message: None,
                remote_update_state: None,
            },
            Some(ActiveConnection::Local(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Local,
                base_url: Some(c.base_url.clone()),
                intent,
                local_auto_bootstrap_allowed,
                token: Some(c.token.clone()),
                host: None,
                user: None,
                remote_port: None,
                remote_data_dir: None,
                remote_update_message: None,
                remote_update_state: None,
            },
            Some(ActiveConnection::Ssh(c)) => DesktopConnectionInfo {
                kind: DesktopConnectionKind::Ssh,
                base_url: Some(c.base_url.clone()),
                intent,
                local_auto_bootstrap_allowed,
                token: c.token.clone(),
                host: Some(c.host.clone()),
                user: c.user.clone(),
                remote_port: Some(c.remote_port),
                remote_data_dir: c.remote_data_dir.clone(),
                remote_update_message: c
                    .remote_update_status
                    .as_ref()
                    .and_then(|status| status.message.clone()),
                remote_update_state: c.remote_update_status.as_ref().map(|status| status.state),
            },
        }
    }

    pub(crate) fn is_remote(&self) -> bool {
        let guard = self.0.lock().ok();
        matches!(
            guard.as_ref().and_then(|g| g.active.as_ref()),
            Some(ActiveConnection::Ssh(_))
        )
    }

    pub(crate) fn local_auto_bootstrap_allowed(&self) -> bool {
        let guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        guard.local_auto_bootstrap_allowed()
    }

    pub(crate) fn mark_explicit_local_intent_if_local(&self) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if matches!(guard.active, Some(ActiveConnection::Local(_))) {
            guard.intent = ConnectionIntent::ExplicitLocal;
        }
    }

    pub(crate) fn mark_explicit_remote_intent(&self) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.intent = ConnectionIntent::ExplicitRemote;
    }

    pub(crate) fn disconnect(&self) {
        let active = {
            let mut guard = match self.0.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            guard.intent = ConnectionIntent::ExplicitDisconnected;
            guard.active.take()
        };
        if let Some(active) = active {
            cleanup_active_connection(active);
        }
    }

    pub(crate) fn disconnect_for_local_restart(&self) -> Result<()> {
        let active = {
            let mut guard = self
                .0
                .lock()
                .map_err(|e| anyhow!("connection manager lock poisoned: {e}"))?;
            guard.active.take()
        };
        if let Some(active) = active {
            cleanup_active_connection_result_for_restart(active)?;
        }
        Ok(())
    }

    pub(crate) fn should_disconnect_for_local_restart(&self) -> bool {
        let guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        matches!(
            guard.active.as_ref(),
            Some(ActiveConnection::Local(LocalConnection {
                ownership: LocalConnectionOwnership::OwnedChild { .. },
                ..
            }))
        )
    }

    pub(crate) fn set_local(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
    ) {
        self.set_local_with_intent(
            base_url,
            token,
            child,
            systemd_scope,
            ConnectionIntent::ExplicitLocal,
        );
    }

    pub(crate) fn set_local_auto_bootstrap(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
    ) -> bool {
        self.set_local_with_auto_bootstrap_gate(base_url, token, child, systemd_scope)
    }

    fn set_local_with_auto_bootstrap_gate(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
    ) -> bool {
        let daemon_pid = Some(child.id());
        let next = ActiveConnection::Local(LocalConnection {
            base_url,
            token,
            daemon_pid,
            source: LocalConnectionSource::SpawnedByDesktop,
            ownership: LocalConnectionOwnership::OwnedChild {
                child,
                systemd_scope,
            },
            http_client: std::sync::OnceLock::new(),
        });
        {
            let mut guard = match self.0.lock() {
                Ok(g) => g,
                Err(_) => {
                    cleanup_active_connection(next);
                    return false;
                }
            };
            let Some(intent) = guard.auto_local_install_intent() else {
                drop(guard);
                cleanup_active_connection(next);
                return false;
            };
            guard.intent = intent;
            guard.active = Some(next);
        }
        log_local_connection_established(LocalConnectionSource::SpawnedByDesktop, daemon_pid);
        true
    }

    fn set_local_with_intent(
        &self,
        base_url: String,
        token: String,
        child: Child,
        systemd_scope: bool,
        intent: ConnectionIntent,
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
            guard.intent = intent;
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
        log_local_connection_established(LocalConnectionSource::SpawnedByDesktop, daemon_pid);
    }

    pub(crate) fn set_local_attached(
        &self,
        base_url: String,
        token: String,
        daemon_pid: Option<u32>,
        source: LocalConnectionSource,
    ) {
        self.set_local_attached_with_intent(
            base_url,
            token,
            daemon_pid,
            source,
            ConnectionIntent::ExplicitLocal,
        );
    }

    pub(crate) fn set_local_attached_auto_bootstrap(
        &self,
        base_url: String,
        token: String,
        daemon_pid: Option<u32>,
        source: LocalConnectionSource,
    ) -> bool {
        self.set_local_attached_with_auto_bootstrap_gate(base_url, token, daemon_pid, source)
    }

    fn set_local_attached_with_auto_bootstrap_gate(
        &self,
        base_url: String,
        token: String,
        daemon_pid: Option<u32>,
        source: LocalConnectionSource,
    ) -> bool {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        let Some(intent) = guard.auto_local_install_intent() else {
            return false;
        };
        guard.intent = intent;
        guard.active = Some(ActiveConnection::Local(LocalConnection {
            base_url,
            token,
            daemon_pid,
            source,
            ownership: LocalConnectionOwnership::UnownedExternal,
            http_client: std::sync::OnceLock::new(),
        }));
        drop(guard);
        log_local_connection_established(source, daemon_pid);
        true
    }

    fn set_local_attached_with_intent(
        &self,
        base_url: String,
        token: String,
        daemon_pid: Option<u32>,
        source: LocalConnectionSource,
        intent: ConnectionIntent,
    ) {
        let previous = {
            let mut next = LocalConnection {
                base_url,
                token,
                daemon_pid,
                source,
                ownership: LocalConnectionOwnership::UnownedExternal,
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
                    guard.intent = intent;
                    None
                }
                other => other,
            };
            guard.intent = intent;
            guard.active = Some(ActiveConnection::Local(next));
            previous
        };
        if let Some(previous) = previous {
            cleanup_active_connection(previous);
        }
        log_local_connection_established(source, daemon_pid);
    }

    #[cfg(test)]
    pub(crate) fn set_ssh(
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

    pub(crate) async fn set_ssh_with_blocking_cleanup(
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
        let log_host = host.clone();
        let log_user = user.clone();
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
        log_ssh_connection_established(&log_host, log_user.as_deref(), remote_port);
        Ok(())
    }

    pub(super) fn replace_with_ssh(
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
        guard.intent = ConnectionIntent::ExplicitRemote;
        guard.active.replace(next)
    }

    pub(crate) fn ssh_target(&self) -> Result<SshConnectionTarget> {
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

    pub(crate) fn update_ssh_token(&self, token: String) -> Result<()> {
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

    pub(crate) fn update_ssh_runtime(&self, runtime: SshRuntimeMetadata) -> Result<()> {
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

    pub(crate) fn set_ssh_remote_update_state(
        &self,
        state: DesktopRemoteDaemonUpdateState,
        message: Option<String>,
    ) -> Result<()> {
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
        c.remote_update_status = Some(SshRemoteUpdateStatus {
            state,
            message: message
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        });
        Ok(())
    }

    pub(crate) fn clear_ssh_remote_update_state(&self) -> Result<()> {
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
        c.remote_update_status = None;
        Ok(())
    }

    pub(crate) fn daemon_request(&self, req: DesktopDaemonRequest) -> Result<DesktopHttpResponse> {
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

    pub(crate) fn upload_blob(
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
