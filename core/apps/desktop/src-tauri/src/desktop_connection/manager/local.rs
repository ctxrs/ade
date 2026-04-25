use super::*;

impl ConnectionManager {
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
}
