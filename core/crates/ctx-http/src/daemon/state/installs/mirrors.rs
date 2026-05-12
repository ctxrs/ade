use super::*;

impl AppState {
    pub async fn register_install_progress_mirror(
        &self,
        source_install_id: InstallId,
        mirror_install_id: InstallId,
    ) -> bool {
        self.providers
            .with_provider_installs(|installs| {
                let (source_provider_id, source_target, inserted, last_event) = {
                    let Some(source_state) = installs.get_mut(&source_install_id) else {
                        return false;
                    };
                    let inserted = source_state.mirrors.insert(mirror_install_id);
                    let source_provider_id = source_state.provider_id.clone();
                    let source_target = source_state.target;
                    let last_event = source_state.events.back().cloned();
                    (source_provider_id, source_target, inserted, last_event)
                };
                let Some(mirror_state) = installs.get_mut(&mirror_install_id) else {
                    return false;
                };
                if inserted {
                    let source_event = last_event.unwrap_or_else(|| InstallProgressEvent {
                        install_id: source_install_id,
                        provider_id: source_provider_id.clone(),
                        target: source_target,
                        at: chrono::Utc::now(),
                        stage: "start".to_string(),
                        message: "Waiting for tracked prerequisite install to report progress"
                            .to_string(),
                        level: InstallEventLevel::Info,
                        bytes: None,
                        total_bytes: None,
                        attempt: None,
                        error_code: None,
                    });
                    let mirrored_event = Self::mirrored_install_event(
                        source_install_id,
                        &source_provider_id,
                        &source_event,
                        mirror_install_id,
                        mirror_state,
                    );
                    Self::set_install_info_event_override_locked(mirror_state, &mirrored_event);
                    Self::push_install_event_locked(mirror_state, mirrored_event);
                }
                true
            })
            .await
    }

    pub async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        self.providers
            .with_provider_installs(|installs| {
                let Some(st) = installs.get_mut(&install_id) else {
                    return;
                };
                let source_provider_id = st.provider_id.clone();
                let mirrors = st.mirrors.iter().copied().collect::<Vec<_>>();
                Self::push_install_event_locked(st, event.clone());
                for mirror_install_id in mirrors {
                    let Some(mirror_state) = installs.get_mut(&mirror_install_id) else {
                        continue;
                    };
                    let mirrored_event = Self::mirrored_install_event(
                        install_id,
                        &source_provider_id,
                        &event,
                        mirror_install_id,
                        mirror_state,
                    );
                    Self::set_install_info_event_override_locked(mirror_state, &mirrored_event);
                    Self::push_install_event_locked(mirror_state, mirrored_event);
                }
            })
            .await;
    }
}
