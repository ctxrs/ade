use super::*;

impl ConnectionManager {
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
}
