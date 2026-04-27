use super::types::{ActiveConnection, LocalConnectionOwnership, SshConnection, SshRuntimeMetadata};
use super::*;

pub(super) fn stop_owned_local_daemon_child(
    base_url: &str,
    auth_token: &str,
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
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(3), Some(auth_token)).is_ok() {
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
            } => stop_owned_local_daemon_child(&c.base_url, &c.token, child, systemd_scope),
            LocalConnectionOwnership::UnownedExternal => Ok(()),
        },
        ActiveConnection::Ssh(c) => try_kill_child(c.tunnel),
    }
}

pub(super) fn cleanup_active_connection(active: ActiveConnection) {
    let _ = cleanup_active_connection_result(active);
}

pub(super) fn cleanup_active_connection_result_for_restart(active: ActiveConnection) -> Result<()> {
    cleanup_active_connection_result(active)
}

pub(super) fn build_ssh_connection(
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

pub(super) fn should_preserve_local_handoff(
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
