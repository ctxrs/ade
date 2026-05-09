use super::*;

pub(super) type SessionModelHttpError = (StatusCode, Json<ApiErrorResp>);
pub(super) type SessionModelResult<T> = Result<T, SessionModelHttpError>;

pub(super) fn session_model_error(
    status: StatusCode,
    error: impl Into<String>,
) -> SessionModelHttpError {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

pub(super) fn internal_session_model_error(error: impl std::fmt::Display) -> SessionModelHttpError {
    session_model_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        logs::redact_sensitive(&error.to_string()),
    )
}
