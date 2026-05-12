use super::*;

mod finish;

impl AppState {
    pub async fn find_running_install(
        &self,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        self.providers
            .with_provider_installs(|installs| {
                self.find_running_install_locked(installs, provider_id, target)
            })
            .await
    }

    pub async fn start_install(
        &self,
        provider_id: String,
        target: Option<InstallTarget>,
    ) -> (InstallId, bool) {
        let _start_gate = self.providers.acquire_install_start_gate().await;
        let existing = self
            .providers
            .with_provider_installs(|installs| {
                self.find_running_install_locked(installs, &provider_id, target)
            })
            .await;
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
            .with_provider_installs(|installs| {
                installs.insert(install_id, state);
            })
            .await;
        let mut event = OpsEvent::new("info", "provider_install_started");
        event.provider_id = Some(provider_id);
        event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": target.map(|value| value.as_str()),
        }));
        self.telemetry.ops_events.emit(event);
        (install_id, true)
    }

    pub async fn cancel_install(
        &self,
        install_id: InstallId,
    ) -> Option<ctx_provider_install::install_state::InstallInfo> {
        enum CancelInstallOutcome {
            AlreadyFinished(ctx_provider_install::install_state::InstallInfo),
            Cancelled {
                provider_id: String,
                target: Option<InstallTarget>,
            },
        }

        let outcome = self
            .providers
            .with_provider_installs(|map| {
                let st = map.get_mut(&install_id)?;
                if !matches!(st.state, InstallStateKind::Running) {
                    return Some(CancelInstallOutcome::AlreadyFinished(st.info(install_id)));
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
                Some(CancelInstallOutcome::Cancelled {
                    provider_id: st.provider_id.clone(),
                    target: st.target,
                })
            })
            .await?;

        let (provider_id, target) = match outcome {
            CancelInstallOutcome::AlreadyFinished(info) => return Some(info),
            CancelInstallOutcome::Cancelled {
                provider_id,
                target,
            } => (provider_id, target),
        };

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
