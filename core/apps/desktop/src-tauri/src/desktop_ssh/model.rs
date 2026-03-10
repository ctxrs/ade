use super::*;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SshConnectReq {
    pub(crate) host: String,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) password_once: Option<String>,
    #[serde(default)]
    pub(crate) remote_port: Option<u16>,
    #[serde(default = "default_true")]
    pub(crate) start_remote: bool,
    #[serde(default)]
    pub(crate) remote_data_dir: Option<String>,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopSshConnectPollReq {
    pub(crate) job_id: String,
    #[serde(default)]
    pub(crate) consume: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DesktopSshConnectJobStatus {
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) info: Option<DesktopConnectionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) created_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) updated_at_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopSshTestReq {
    pub(crate) host: String,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) password_once: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopRemotePrewarmReq {
    pub(crate) host: String,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) remote_port: Option<u16>,
    #[serde(default)]
    pub(crate) remote_data_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopSshPathReq {
    pub(crate) host: String,
    #[serde(default)]
    pub(crate) user: Option<String>,
    #[serde(default)]
    pub(crate) path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DesktopSshPathEntry {
    pub(crate) name: String,
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopGitBranchReq {
    pub(crate) path: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DesktopSshHost {
    pub(crate) host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) port: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DesktopRemoteDaemonUpdateReq {
    #[serde(default)]
    pub(crate) confirm: bool,
    #[serde(default)]
    pub(crate) channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct DesktopRemoteDaemonUpdateResp {
    pub(crate) updated: bool,
    pub(crate) message: String,
}

pub(super) const MANAGED_REMOTE_CTX_BIN: &str = "~/.ctx/bin/ctx";
pub(super) const WINDOWS_REMOTE_UNSUPPORTED_MSG: &str =
    "Remote Windows hosts are not supported yet. Use a Linux host (x86_64 or arm64).";
pub(super) const REMOTE_BOOTSTRAP_CAPABILITY_MSG: &str =
    "Remote daemon bootstrap failed while retrieving managed daemon artifact. Check network connectivity and release metadata.";
pub(super) const PLATFORM_PROBE_OS_MARKER: &str = "__CTX_PLATFORM_OS__";
pub(super) const PLATFORM_PROBE_ARCH_MARKER: &str = "__CTX_PLATFORM_ARCH__";
pub(super) const SSH_CONFIG_OVERRIDE_ENV: &str = "CTX_DESKTOP_SSH_CONFIG_PATH";
pub(super) const SSH_TUNNEL_BOOTSTRAP_HEALTH_RETRIES: usize = 12;
pub(super) const SSH_TUNNEL_BOOTSTRAP_HEALTH_BASE_DELAY_MS: u64 = 150;
pub(super) const SSH_TUNNEL_LOG_BYTES: usize = 4096;
pub(super) const DEFAULT_DOWNLOAD_BASE_URL: &str = "https://api.ctx.rs/functions/v1";
pub(super) const REMOTE_DAEMON_DOWNLOAD_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RemoteLinuxPlatform {
    pub(super) arch: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteAuthBootstrap {
    None,
    PasswordOncePubkeyInstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemoteProbe {
    pub(super) platform: RemoteLinuxPlatform,
    pub(super) auth_bootstrap_used: RemoteAuthBootstrap,
    pub(super) managed_binary_present: bool,
    pub(super) existing_daemon_reachable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SshConnectTarget {
    pub(super) host: String,
    pub(super) user: Option<String>,
    pub(super) password_once: Option<String>,
    pub(super) remote_port: u16,
    pub(super) start_remote: bool,
    pub(super) remote_data_dir: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectJobPhase {
    Queued,
    Probing,
    Planning,
    InstallingManagedDaemon,
    StartingRemoteDaemon,
    OpeningTunnel,
    ReadingAuth,
    HandingOffConnection,
    Succeeded,
    Failed,
}

impl ConnectJobPhase {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Probing => "probing",
            Self::Planning => "planning",
            Self::InstallingManagedDaemon => "installing_managed_daemon",
            Self::StartingRemoteDaemon => "starting_remote_daemon",
            Self::OpeningTunnel => "opening_tunnel",
            Self::ReadingAuth => "reading_auth",
            Self::HandingOffConnection => "handing_off_connection",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }

    pub(super) fn status(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            _ => "pending",
        }
    }
}

pub(super) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
}

pub(crate) fn normalize_update_channel(raw: Option<&str>) -> Result<String, String> {
    let channel = raw
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("stable");
    let valid = channel
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.');
    if !valid {
        return Err("invalid channel (expected [A-Za-z0-9._-])".to_string());
    }
    Ok(channel.to_string())
}
