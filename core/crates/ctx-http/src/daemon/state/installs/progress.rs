use super::*;

const PREREQUISITE_PROGRESS_STAGE_FLOOR: &str = "start";
const PREREQUISITE_PROGRESS_VISIBILITY_MS: i64 = 1_200;

impl AppState {
    pub(super) fn push_install_event_locked(st: &mut InstallState, event: InstallProgressEvent) {
        st.progress_pct = ctx_provider_install::install_state::heuristic_progress_pct_from_event(
            &event,
            st.progress_pct,
        );
        if st.events.len() >= 256 {
            st.events.pop_front();
        }
        st.events.push_back(event.clone());
        let _ = st.tx.send(event);
    }

    pub(super) fn set_install_info_event_override_locked(
        st: &mut InstallState,
        event: &InstallProgressEvent,
    ) {
        st.info_event_override = Some(event.clone());
        st.info_event_override_until =
            Some(event.at + chrono::Duration::milliseconds(PREREQUISITE_PROGRESS_VISIBILITY_MS));
    }

    pub(super) fn mirrored_install_event(
        source_install_id: InstallId,
        source_provider_id: &str,
        source_event: &InstallProgressEvent,
        mirror_install_id: InstallId,
        mirror_state: &InstallState,
    ) -> InstallProgressEvent {
        InstallProgressEvent {
            install_id: mirror_install_id,
            provider_id: mirror_state.provider_id.clone(),
            target: mirror_state.target,
            at: chrono::Utc::now(),
            stage: PREREQUISITE_PROGRESS_STAGE_FLOOR.to_string(),
            message: format!(
                "Prerequisite {source_provider_id} (install {source_install_id}, stage {}): {}",
                source_event.stage, source_event.message
            ),
            level: source_event.level,
            bytes: None,
            total_bytes: None,
            attempt: None,
            error_code: source_event.error_code,
        }
    }
}
