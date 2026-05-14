use super::*;

type ApiErr = (StatusCode, Json<ApiErrorResp>);

mod attachments;
mod delete;
mod post;

pub(crate) use delete::delete_session_message;
pub(crate) use post::post_message;

fn api_error(status: StatusCode, error: impl Into<String>) -> ApiErr {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}
