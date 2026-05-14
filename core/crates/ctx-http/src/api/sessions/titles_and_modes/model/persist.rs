use super::*;
use crate::api::sessions::titles_and_modes::model::error::{
    internal_session_model_error, session_model_error, SessionModelResult,
};
use crate::api::sessions::titles_and_modes::model::resolve::ResolvedSessionModelUpdate;

pub(super) async fn persist_session_model_update(
    state: &SessionsHandle,
    session_id: SessionId,
    resolved_model: &ResolvedSessionModelUpdate,
) -> SessionModelResult<Session> {
    state
        .persist_session_model_update_for_request(
            session_id,
            resolved_model.model_id.clone(),
            resolved_model.reasoning_effort.clone(),
            resolved_model.full_model_id.clone(),
        )
        .await
        .map_err(internal_session_model_error)?
        .ok_or_else(|| session_model_error(StatusCode::NOT_FOUND, "session not found"))
}
