use std::time::Duration;

use super::*;

const INSTALL_TIMEOUT_GRACE_SECS: u64 = 90;
const INSTALL_TIMEOUT_VENV_SECS: u64 = (5 * 60) + INSTALL_TIMEOUT_GRACE_SECS;
const INSTALL_TIMEOUT_DOWNLOAD_SECS: u64 = (15 * 60) + INSTALL_TIMEOUT_GRACE_SECS;
const INSTALL_TIMEOUT_PACKAGE_MANAGER_SECS: u64 = (12 * 60) + INSTALL_TIMEOUT_GRACE_SECS;
const INSTALL_TIMEOUT_PREPARE_SECS: u64 = 5 * 60;
const INSTALL_TIMEOUT_REGISTRY_SECS: u64 = 2 * 60;
const INSTALL_TIMEOUT_DEFAULT_SECS: u64 = 20 * 60;

impl AppState {
    fn install_running_timeout_for_stage(stage: &str) -> Duration {
        match stage {
            "download" | "node_download" | "python_download" | "model_download"
            | "runtime_download" => Duration::from_secs(INSTALL_TIMEOUT_DOWNLOAD_SECS),
            "npm_install" | "dependency_npm_install" | "pip_install" => {
                Duration::from_secs(INSTALL_TIMEOUT_PACKAGE_MANAGER_SECS)
            }
            "venv" => Duration::from_secs(INSTALL_TIMEOUT_VENV_SECS),
            "registry" | "registry_load" | "registry_save" => {
                Duration::from_secs(INSTALL_TIMEOUT_REGISTRY_SECS)
            }
            "prepare" | "extract" | "node_extract" | "python_extract" | "runtime_extract" => {
                Duration::from_secs(INSTALL_TIMEOUT_PREPARE_SECS)
            }
            _ => Duration::from_secs(INSTALL_TIMEOUT_DEFAULT_SECS),
        }
    }

    fn format_install_timeout(duration: Duration) -> String {
        let secs = duration.as_secs();
        if secs >= 60 {
            let mins = secs / 60;
            let rem = secs % 60;
            if rem == 0 {
                format!("{mins}m")
            } else {
                format!("{mins}m {rem}s")
            }
        } else {
            format!("{secs}s")
        }
    }

    pub(super) fn reconcile_stale_running_install_locked(
        &self,
        install_id: InstallId,
        st: &mut InstallState,
    ) -> bool {
        if !matches!(st.state, InstallStateKind::Running) {
            return false;
        }
        let now = chrono::Utc::now();
        let last_event = st.events.back().cloned();
        let stage = last_event
            .as_ref()
            .map(|event| event.stage.trim())
            .filter(|stage| !stage.is_empty())
            .unwrap_or("prepare");
        let anchor = last_event
            .as_ref()
            .map(|event| event.at)
            .unwrap_or(st.started_at);
        let Ok(inactive_for) = now.signed_duration_since(anchor).to_std() else {
            return false;
        };
        let timeout_after = Self::install_running_timeout_for_stage(stage);
        if inactive_for <= timeout_after {
            return false;
        }

        let message = format!(
            "Install timed out during {stage} after {} without progress. Retry the install.",
            Self::format_install_timeout(inactive_for)
        );
        st.state = InstallStateKind::Failed;
        st.finished_at = Some(now);
        st.error = Some(message.clone());
        st.error_code = Some(InstallErrorCode::Timeout);
        let event = InstallProgressEvent {
            install_id,
            provider_id: st.provider_id.clone(),
            target: st.target,
            at: now,
            stage: stage.to_string(),
            message: message.clone(),
            level: InstallEventLevel::Error,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: Some(InstallErrorCode::Timeout),
        };
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);

        let mut ops_event = OpsEvent::new("warn", "provider_install_failed");
        ops_event.provider_id = Some(st.provider_id.clone());
        ops_event.meta = Some(serde_json::json!({
            "install_id": install_id.to_string(),
            "target": st.target.map(|value| value.as_str()),
            "state": "failed",
            "error": message,
            "error_code": "timeout",
            "ok": false,
        }));
        self.telemetry.ops_events.emit(ops_event);
        true
    }

    pub(super) fn find_running_install_locked(
        &self,
        installs: &mut HashMap<InstallId, InstallState>,
        provider_id: &str,
        target: Option<InstallTarget>,
    ) -> Option<InstallId> {
        installs.iter_mut().find_map(|(id, st)| {
            let _ = self.reconcile_stale_running_install_locked(*id, st);
            if st.provider_id == provider_id
                && st.target == target
                && matches!(st.state, InstallStateKind::Running)
            {
                Some(*id)
            } else {
                None
            }
        })
    }
}
