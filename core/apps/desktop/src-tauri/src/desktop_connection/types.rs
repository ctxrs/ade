use super::*;

#[derive(Debug, Clone)]
pub(crate) struct SshConnectionTarget {
    pub(crate) host: String,
    pub(crate) user: Option<String>,
    pub(crate) remote_port: u16,
    pub(crate) remote_data_dir: Option<String>,
    pub(crate) runtime: SshRuntimeMetadata,
}

#[derive(Debug, Clone)]
pub(crate) struct SshRuntimeMetadata {
    pub(crate) managed_ctx_bin: String,
    pub(crate) active_ctx_bin: Option<String>,
    pub(crate) ssh_password_once: Option<String>,
    pub(crate) admin_password_once: Option<String>,
}

pub(super) struct ConnectionState {
    pub(super) active: Option<ActiveConnection>,
    pub(super) intent: ConnectionIntent,
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
    pub(super) fn local_auto_bootstrap_allowed(&self) -> bool {
        self.intent.allows_local_auto_bootstrap()
            && !matches!(self.active, Some(ActiveConnection::Ssh(_)))
    }

    pub(super) fn auto_local_install_intent(&self) -> Option<ConnectionIntent> {
        if !self.local_auto_bootstrap_allowed() || self.active.is_some() {
            return None;
        }
        Some(match self.intent {
            ConnectionIntent::ExplicitLocal => ConnectionIntent::ExplicitLocal,
            _ => ConnectionIntent::AutoLocalBootstrap,
        })
    }
}

pub(super) enum ActiveConnection {
    Local(LocalConnection),
    Ssh(SshConnection),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionIntent {
    AutoLocalBootstrap,
    ExplicitLocal,
    ExplicitRemote,
    ExplicitDisconnected,
}

impl ConnectionIntent {
    pub(super) fn as_ipc(self) -> DesktopConnectionIntent {
        match self {
            ConnectionIntent::AutoLocalBootstrap => DesktopConnectionIntent::AutoLocalBootstrap,
            ConnectionIntent::ExplicitLocal => DesktopConnectionIntent::ExplicitLocal,
            ConnectionIntent::ExplicitRemote => DesktopConnectionIntent::ExplicitRemote,
            ConnectionIntent::ExplicitDisconnected => DesktopConnectionIntent::ExplicitDisconnected,
        }
    }

    pub(super) fn allows_local_auto_bootstrap(self) -> bool {
        matches!(
            self,
            ConnectionIntent::AutoLocalBootstrap | ConnectionIntent::ExplicitLocal
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalConnectionSource {
    EnvOverride,
    ExistingCompatibleDaemon,
    SpawnedByDesktop,
}

pub(super) struct LocalConnection {
    pub(super) base_url: String,
    pub(super) token: String,
    pub(super) daemon_pid: Option<u32>,
    pub(super) source: LocalConnectionSource,
    pub(super) ownership: LocalConnectionOwnership,
    pub(super) http_client: std::sync::OnceLock<reqwest::blocking::Client>,
}

pub(super) enum LocalConnectionOwnership {
    OwnedChild { child: Child, systemd_scope: bool },
    UnownedExternal,
}

pub(super) struct SshConnection {
    pub(super) base_url: String,
    pub(super) token: Option<String>,
    pub(super) tunnel: Child,
    pub(super) host: String,
    pub(super) user: Option<String>,
    pub(super) remote_port: u16,
    pub(super) remote_data_dir: Option<String>,
    pub(super) runtime: SshRuntimeMetadata,
    pub(super) remote_update_status: Option<SshRemoteUpdateStatus>,
    pub(super) http_client: std::sync::OnceLock<reqwest::blocking::Client>,
}

#[derive(Debug, Clone)]
pub(super) struct SshRemoteUpdateStatus {
    pub(super) state: DesktopRemoteDaemonUpdateState,
    pub(super) message: Option<String>,
}

fn local_connection_source_label(source: LocalConnectionSource) -> &'static str {
    match source {
        LocalConnectionSource::EnvOverride => "env_override",
        LocalConnectionSource::ExistingCompatibleDaemon => "existing_compatible_daemon",
        LocalConnectionSource::SpawnedByDesktop => "spawned_by_desktop",
    }
}

pub(super) fn log_local_connection_established(
    source: LocalConnectionSource,
    daemon_pid: Option<u32>,
) {
    log_desktop_startup(&format!(
        "desktop_startup: daemon_connected kind=local source={} daemon_pid={}",
        local_connection_source_label(source),
        daemon_pid
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "none".to_string()),
    ));
}

pub(super) fn log_ssh_connection_established(host: &str, user: Option<&str>, remote_port: u16) {
    log_desktop_startup(&format!(
        "desktop_startup: daemon_connected kind=ssh host={} user={} remote_port={remote_port}",
        serde_json::to_string(host).unwrap_or_else(|_| "\"unknown\"".to_string()),
        serde_json::to_string(user.unwrap_or("")).unwrap_or_else(|_| "\"\"".to_string()),
    ));
}
