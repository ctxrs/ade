use super::*;
use crate::daemon::sessions::subagents::{SubagentError, SubagentErrorKind};

mod context;
mod handlers;
mod init;
mod listings;
mod types;

pub(crate) use context::{context_window_for_run, worktree_path_for_child};
pub(crate) use handlers::*;
pub(crate) use init::*;
pub(crate) use listings::{
    get_session_subagent_invocation, list_session_subagent_invocations, list_session_subagents,
};
pub(crate) use types::*;

fn subagent_error_response(error: SubagentError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        SubagentErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        SubagentErrorKind::NotFound => StatusCode::NOT_FOUND,
        SubagentErrorKind::Forbidden => StatusCode::FORBIDDEN,
        SubagentErrorKind::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
        SubagentErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
