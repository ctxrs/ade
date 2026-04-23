use super::*;
pub(super) use ctx_desktop_ipc::{
    DesktopConnectionInfo, DesktopConnectionIntent, DesktopConnectionKind,
    DesktopRemoteDaemonUpdateState,
};

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

struct ConnectionState {
    active: Option<ActiveConnection>,
    intent: ConnectionIntent,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self {
            active: None,
            intent: ConnectionIntent::AutoLocalBootstrap,
        }
    }
}

impl ConnectionState {
    fn local_auto_bootstrap_allowed(&self) -> bool {
        self.intent.allows_local_auto_bootstrap()
            && !matches!(self.active, Some(ActiveConnection::Ssh(_)))
    }

    fn auto_local_install_intent(&self) -> Option<ConnectionIntent> {
        if !self.local_auto_bootstrap_allowed() || self.active.is_some() {
            return None;
        }
        Some(match self.intent {
            ConnectionIntent::ExplicitLocal => ConnectionIntent::ExplicitLocal,
            _ => ConnectionIntent::AutoLocalBootstrap,
        })
    }
}

enum ActiveConnection {
    Local(LocalConnection),
    Ssh(SshConnection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionIntent {
    AutoLocalBootstrap,
    ExplicitLocal,
    ExplicitRemote,
    ExplicitDisconnected,
}

impl ConnectionIntent {
    fn as_ipc(self) -> DesktopConnectionIntent {
        match self {
            ConnectionIntent::AutoLocalBootstrap => DesktopConnectionIntent::AutoLocalBootstrap,
            ConnectionIntent::ExplicitLocal => DesktopConnectionIntent::ExplicitLocal,
            ConnectionIntent::ExplicitRemote => DesktopConnectionIntent::ExplicitRemote,
            ConnectionIntent::ExplicitDisconnected => DesktopConnectionIntent::ExplicitDisconnected,
        }
    }

    fn allows_local_auto_bootstrap(self) -> bool {
        matches!(
            self,
            ConnectionIntent::AutoLocalBootstrap | ConnectionIntent::ExplicitLocal
        )
    }
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
    remote_update_status: Option<SshRemoteUpdateStatus>,
    http_client: std::sync::OnceLock<reqwest::blocking::Client>,
}

#[derive(Debug, Clone)]
struct SshRemoteUpdateStatus {
    state: DesktopRemoteDaemonUpdateState,
    message: Option<String>,
}

fn local_connection_source_label(source: LocalConnectionSource) -> &'static str {
    match source {
        LocalConnectionSource::EnvOverride => "env_override",
        LocalConnectionSource::ExistingCompatibleDaemon => "existing_compatible_daemon",
        LocalConnectionSource::SpawnedByDesktop => "spawned_by_desktop",
    }
}

fn log_local_connection_established(source: LocalConnectionSource, daemon_pid: Option<u32>) {
    log_desktop_startup(&format!(
        "desktop_startup: daemon_connected kind=local source={} daemon_pid={}",
        local_connection_source_label(source),
        daemon_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "none".to_string()),
    ));
}

fn log_ssh_connection_established(host: &str, user: Option<&str>, remote_port: u16) {
    log_desktop_startup(&format!(
        "desktop_startup: daemon_connected kind=ssh host={} user={} remote_port={remote_port}",
        serde_json::to_string(host).unwrap_or_else(|_| "\"unknown\"".to_string()),
        serde_json::to_string(user.unwrap_or("")).unwrap_or_else(|_| "\"\"".to_string()),
    ));
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
        remote_update_status: None,
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

    pub(super) fn is_remote(&self) -> bool {
        let guard = self.0.lock().ok();
        matches!(
            guard.as_ref().and_then(|g| g.active.as_ref()),
            Some(ActiveConnection::Ssh(_))
        )
    }

    pub(super) fn local_auto_bootstrap_allowed(&self) -> bool {
        let guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        guard.local_auto_bootstrap_allowed()
    }

    pub(super) fn mark_explicit_local_intent_if_local(&self) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if matches!(guard.active, Some(ActiveConnection::Local(_))) {
            guard.intent = ConnectionIntent::ExplicitLocal;
        }
    }

    pub(super) fn mark_explicit_remote_intent(&self) {
        let mut guard = match self.0.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.intent = ConnectionIntent::ExplicitRemote;
    }

    pub(super) fn disconnect(&self) {
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
                ownership: LocalConnectionOwnership::OwnedChild { .. },
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
        self.set_local_with_intent(
            base_url,
            token,
            child,
            systemd_scope,
            ConnectionIntent::ExplicitLocal,
        );
    }

    pub(super) fn set_local_auto_bootstrap(
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

    pub(super) fn set_local_attached(
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

    pub(super) fn set_local_attached_auto_bootstrap(
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
        guard.intent = ConnectionIntent::ExplicitRemote;
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

    pub(super) fn set_ssh_remote_update_state(
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

    pub(super) fn clear_ssh_remote_update_state(&self) -> Result<()> {
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
