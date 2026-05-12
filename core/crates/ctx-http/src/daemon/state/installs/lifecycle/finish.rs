use super::super::*;

impl AppState {
    pub async fn finish_install(
        &self,
        install_id: InstallId,
        ok: bool,
        error: Option<String>,
        error_code: Option<InstallErrorCode>,
    ) {
        let Some((provider_id, target, state, error, error_code)) = self
            .providers
            .with_provider_installs(|map| {
                let st = map.get_mut(&install_id)?;
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
                Some((
                    st.provider_id.clone(),
                    st.target,
                    st.state,
                    st.error.clone(),
                    st.error_code,
                ))
            })
            .await
        else {
            return;
        };

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
}
