use super::*;

type ApiErr = (StatusCode, Json<ApiErrorResp>);

mod attachments;
mod delete;
mod post;
mod turns;

pub(crate) use delete::delete_session_message;
pub(crate) use post::post_message;
pub(crate) use turns::ensure_session_turn_for_message;

fn api_error(status: StatusCode, error: impl Into<String>) -> ApiErr {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn session_store_api_error(status: StatusCode) -> ApiErr {
    match status {
        StatusCode::NOT_FOUND => api_error(StatusCode::NOT_FOUND, "Session not found."),
        _ => api_error(status, "Failed to open session."),
    }
}
