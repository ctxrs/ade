use super::*;

impl AppState {
    pub async fn set_install_progress_pct_override(&self, install_id: InstallId, pct: Option<u8>) {
        self.providers
            .with_provider_installs(|installs| {
                let Some(state) = installs.get_mut(&install_id) else {
                    return;
                };
                state.progress_pct_override = pct;
            })
            .await;
    }

    pub async fn get_install_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        self.providers
            .with_provider_installs(|installs| installs.get(&install_id).map(|s| s.tx.clone()))
            .await
    }

    pub async fn get_install_info(
        &self,
        install_id: InstallId,
    ) -> Option<ctx_provider_install::install_state::InstallInfo> {
        self.providers
            .with_provider_installs(|map| {
                let st = map.get_mut(&install_id)?;
                let _ = self.reconcile_stale_running_install_locked(install_id, st);
                Some(st.info(install_id))
            })
            .await
    }

    pub async fn get_install_polling_info(
        &self,
        install_id: InstallId,
    ) -> Option<ctx_provider_install::install_state::InstallInfo> {
        self.providers
            .with_provider_installs(|map| {
                let st = map.get_mut(&install_id)?;
                let _ = self.reconcile_stale_running_install_locked(install_id, st);
                Some(st.polling_info(install_id))
            })
            .await
    }

    pub async fn get_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        self.providers
            .with_provider_installs(|map| {
                let st = map.get_mut(&install_id)?;
                let _ = self.reconcile_stale_running_install_locked(install_id, st);
                Some(st.events.iter().cloned().collect())
            })
            .await
    }

    pub async fn is_install_cancelled(&self, install_id: InstallId) -> bool {
        self.providers
            .with_provider_installs(|map| {
                map.get(&install_id)
                    .map(|st| matches!(st.state, InstallStateKind::Cancelled))
                    .unwrap_or(false)
            })
            .await
    }
}
