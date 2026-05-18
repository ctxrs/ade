use super::*;

type ApiErr = (StatusCode, Json<ApiErrorResp>);

mod delete;
mod post;

pub(crate) use delete::delete_session_message;
pub(crate) use post::post_message;
