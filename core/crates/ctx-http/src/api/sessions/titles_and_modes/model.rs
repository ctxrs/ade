use super::*;

use ctx_daemon::daemon::sessions::{
    SetSessionModelError, SetSessionModelErrorKind, SetSessionModelRequest,
};

#[derive(Debug, Deserialize)]
pub(crate) struct SetSessionModelReq {
    model_id: String,
    #[serde(default)]
    reasoning_effort: Option<String>,
}

pub(crate) async fn set_session_model(
    State(state): State<SessionsHandle>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionModelReq>,
) -> Result<Json<Session>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(
        uuid::Uuid::parse_str(&id)
            .map_err(|_| model_error(StatusCode::BAD_REQUEST, "invalid session id"))?,
    );

    let updated = state
        .set_session_model_for_request(
            session_id,
            SetSessionModelRequest {
                model_id: req.model_id,
                reasoning_effort: req.reasoning_effort,
            },
        )
        .await
        .map_err(map_set_session_model_error)?;

    Ok(Json(updated))
}

fn map_set_session_model_error(error: SetSessionModelError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        SetSessionModelErrorKind::BadRequest | SetSessionModelErrorKind::LiveSwitchRejected => {
            StatusCode::BAD_REQUEST
        }
        SetSessionModelErrorKind::NotFound => StatusCode::NOT_FOUND,
        SetSessionModelErrorKind::Forbidden => StatusCode::FORBIDDEN,
        SetSessionModelErrorKind::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
        SetSessionModelErrorKind::ProviderUnavailable | SetSessionModelErrorKind::Internal => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    };
    model_error(status, error.message())
}

fn model_error(status: StatusCode, error: impl Into<String>) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}
