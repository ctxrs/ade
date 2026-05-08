use super::*;

mod progress;
mod stale;

impl AppState {
    pub async fn set_install_progress_pct_override(&self, install_id: InstallId, pct: Option<u8>) {
        let mut installs = self.providers.installs.lock().await;
        let Some(state) = installs.get_mut(&install_id) else {
            return;
        };
        state.progress_pct_override = pct;
    }

    pub async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        let mut map = self.providers.installs.lock().await;
        self.find_running_install_locked(&mut map, provider_id, target)
    }

    pub async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        let _start_gate = self.providers.install_start_gate.lock().await;
        let existing = {
            let mut installs = self.providers.installs.lock().await;
            self.find_running_install_locked(&mut installs, &provider_id, target)
        };
        if let Some(existing) = existing {
            let mut event = OpsEvent::new("info", "provider_install_joined");
            event.provider_id = Some(provider_id.clone());
            event.meta = Some(serde_json::json!({
                "install_id": existing.to_string(),
                "target": target.map(|value| value.as_str()),
            }));
            self.telemetry.ops_events.emit(event);
            return (existing, false);
        }
        let install_id = InstallId::new_v4();
        let mut state = InstallState::new(provider_id, target);
        let start_event = state.canonical_start_event(install_id);
        Self::push_install_event_locked(&mut state, start_event);
        let (provider_id, target) = (state.provider_id.clone(), state.target);
        self.providers
            .installs
            .lock()
            .await
            .insert(install_id, state);
        let mut event = OpsEvent::new("info", "provider_install_started");
        event.provider_id = Some(provider_id);
        event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
        }));
        self.telemetry.ops_events.emit(event);
        (install_id, true)
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

    pub async fn register_install_progress_mirror(
        &self,
        source_install_id: InstallId,
        mirror_install_id: InstallId,
    ) -> bool {
        let mut installs = self.providers.installs.lock().await;
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
                message: "Waiting for tracked prerequisite install to report progress".to_string(),
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
    }

    pub async fn emit_install_event(&self, install_id: InstallId, event: InstallProgressEvent) {
        let mut installs = self.providers.installs.lock().await;
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
    }

    pub async fn finish_install(
        &self,
        install_id: InstallId,
        ok: bool,
        error: Option<String>,
        error_code: Option<InstallErrorCode>,
    ) {
        let mut map = self.providers.installs.lock().await;
        let Some(st) = map.get_mut(&install_id) else {
            return;
        };
        if !matches!(st.state, InstallStateKind::Cancelled) {
            st.state = if ok {
                InstallStateKind::Succeeded
            } else {
                InstallStateKind::Failed
            };
        }
        if !ok || matches!(st.state, InstallStateKind::Cancelled) {
            st.error = error.or_else(|| {
                if matches!(st.state, InstallStateKind::Cancelled) {
                    Some("Install canceled by user".to_string())
                } else {
                    None
                }
            });
            st.error_code = error_code.or({
                if matches!(st.state, InstallStateKind::Cancelled) {
                    Some(InstallErrorCode::Cancelled)
                } else {
                    None
                }
            });
        } else {
            st.error = None;
            st.error_code = None;
        }
        if ok {
            st.progress_pct = Some(100);
        }
        st.progress_pct_override = None;
        st.info_event_override = None;
        st.info_event_override_until = None;
        st.finished_at = Some(chrono::Utc::now());
        let provider_id = st.provider_id.clone();
        let target = st.target;
        let state = st.state;
        let error = st.error.clone();
        let error_code = st.error_code;
        drop(map);

        let event_name = match state {
            InstallStateKind::Succeeded => "provider_install_succeeded",
            InstallStateKind::Failed => "provider_install_failed",
            InstallStateKind::Cancelled => "provider_install_cancelled",
            InstallStateKind::Running => "provider_install_running",
        };
        let mut event = OpsEvent::new(
            if matches!(state, InstallStateKind::Failed) {
                "warn"
            } else {
                "info"
            },
            event_name,
        );
        event.provider_id = Some(provider_id);
        event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
            "state": match state {
                InstallStateKind::Running => "running",
                InstallStateKind::Succeeded => "succeeded",
                InstallStateKind::Failed => "failed",
                InstallStateKind::Cancelled => "cancelled",
            },
            "error": error,
            "error_code": error_code.and_then(|value| serde_json::to_value(value).ok()),
            "ok": ok,
        }));
        self.telemetry.ops_events.emit(event);
    }

    pub async fn is_install_cancelled(&self, install_id: InstallId) -> bool {
        let map = self.providers.installs.lock().await;
        map.get(&install_id)
            .map(|st| matches!(st.state, InstallStateKind::Cancelled))
            .unwrap_or(false)
    }

    pub async fn cancel_install(
        &self,
        install_id: InstallId,
    ) -> Option<ctx_provider_install::install_state::InstallInfo> {
        let mut map = self.providers.installs.lock().await;
        let st = map.get_mut(&install_id)?;
        if !matches!(st.state, InstallStateKind::Running) {
            return Some(st.info(install_id));
        }

        st.state = InstallStateKind::Cancelled;
        st.error = Some("Install canceled by user".to_string());
        st.error_code = Some(InstallErrorCode::Cancelled);
        st.progress_pct_override = None;
        st.info_event_override = None;
        st.info_event_override_until = None;
        st.finished_at = Some(chrono::Utc::now());

        let event = InstallProgressEvent {
            install_id,
            provider_id: st.provider_id.clone(),
            target: st.target,
            at: chrono::Utc::now(),
            stage: "cancelled".to_string(),
            message: "Install canceled by user".to_string(),
            level: InstallEventLevel::Warning,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: Some(InstallErrorCode::Cancelled),
        };
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);
        let provider_id = st.provider_id.clone();
        let target = st.target;
        drop(map);

        let mut ops_event = OpsEvent::new("info", "provider_install_cancel_requested");
        ops_event.provider_id = Some(provider_id);
        ops_event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
        }));
        self.telemetry.ops_events.emit(ops_event);

        self.get_install_info(install_id).await
    }
}
