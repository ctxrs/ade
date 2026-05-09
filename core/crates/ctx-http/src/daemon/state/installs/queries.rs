use super::*;

impl AppState {
    pub async fn set_install_progress_pct_override(&self, install_id: InstallId, pct: Option<u8>) {
        let mut installs = self.providers.installs.lock().await;
        let Some(state) = installs.get_mut(&install_id) else {
            return;
        };
        state.progress_pct_override = pct;
    }

    pub async fn get_install_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        self.providers
            .installs
            .lock()
            .await
            .get(&install_id)
            .map(|s| s.tx.clone())
    }

    pub async fn get_install_info(
        &self,
        install_id: InstallId,
    ) -> Option<ctx_provider_install::install_state::InstallInfo> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        let _ = self.reconcile_stale_running_install_locked(install_id, st);
        Some(st.info(install_id))
    }

    pub async fn get_install_polling_info(
        &self,
        install_id: InstallId,
    ) -> Option<ctx_provider_install::install_state::InstallInfo> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        let _ = self.reconcile_stale_running_install_locked(install_id, st);
        Some(st.polling_info(install_id))
    }

    pub async fn get_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        let _ = self.reconcile_stale_running_install_locked(install_id, st);
        Some(st.events.iter().cloned().collect())
    }

    pub async fn is_install_cancelled(&self, install_id: InstallId) -> bool {
        let map = self.providers.installs.lock().await;
        map.get(&install_id)
            .map(|st| matches!(st.state, InstallStateKind::Cancelled))
            .unwrap_or(false)
    }
}
