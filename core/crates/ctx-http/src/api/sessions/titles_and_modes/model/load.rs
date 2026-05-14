use ctx_core::models::{ExecutionEnvironment, Workspace};
use ctx_provider_install::install_state::InstallTarget;

use super::*;
use crate::api::sessions::titles_and_modes::model::error::{
    internal_session_model_error, session_model_error, SessionModelHttpError, SessionModelResult,
};
use ctx_daemon::daemon::sessions::SessionModelTargetLoadError;

pub(super) struct SessionModelTarget {
    pub(super) session: Session,
    pub(super) workspace: Workspace,
    pub(super) execution_environment: ExecutionEnvironment,
    pub(super) install_target: InstallTarget,
}

pub(super) async fn load_session_model_target(
    state: &SessionsHandle,
    session_id: SessionId,
) -> SessionModelResult<SessionModelTarget> {
    let (session, workspace, execution_environment, install_target) = state
        .load_session_model_target_parts(session_id)
        .await
        .map_err(session_model_target_load_error)?;

    Ok(SessionModelTarget {
        session,
        workspace,
        execution_environment,
        install_target,
    })
}

fn session_model_target_load_error(error: SessionModelTargetLoadError) -> SessionModelHttpError {
    match error {
        SessionModelTargetLoadError::NotFound(kind) => {
            session_model_error(StatusCode::NOT_FOUND, format!("{kind} not found"))
        }
        SessionModelTargetLoadError::ExecutionSettings(error) => session_model_error(
            crate::api::shared::status_code_for_internal_error(&error),
            "failed to load execution settings",
        ),
        SessionModelTargetLoadError::Internal(error) => internal_session_model_error(error),
    }
}
