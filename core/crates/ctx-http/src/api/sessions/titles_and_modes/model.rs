use super::*;

mod error;
mod load;
mod persist;
mod resolve;

use error::session_model_error;
use load::load_session_model_target;
use persist::persist_session_model_update;
use resolve::{
    ensure_session_model_adapter, resolve_session_model_update, switch_live_session_model,
};

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModelReq {
    model_id: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

pub(crate) async fn set_session_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModelReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| session_model_error(StatusCode::BAD_REQUEST, "invalid session id"))?,
    );

    let target = load_session_model_target(&state, session_id).await?;
    let adapter =
        ensure_session_model_adapter(&state, &target.session, target.install_target).await?;
    let resolved_model = resolve_session_model_update(
        &state,
        &target.workspace,
        &target.session,
        target.execution_environment,
        req,
    )
    .await?;
    switch_live_session_model(
        adapter.as_ref(),
        &target.session,
        &resolved_model.full_model_id,
    )
    .await?;

    let updated =
        persist_session_model_update(&state, &target.store, session_id, &resolved_model).await?;

    Ok(Json(updated))
}
