use super::*;

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
}
