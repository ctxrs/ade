use super::launch::{WebSessionLaunchError, WebSessionLaunchErrorKind, WebSessionLaunchRequest};
use super::*;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct WebSessionCreatePayload {
    session_id: Option<String>,
    worktree_id: Option<String>,
    url: String,
    viewport: Option<WebSessionViewport>,
    fps: Option<u32>,
}

pub(in crate::api) async fn create_web_session(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<WebSessionCreatePayload>,
) -> Result<Json<WebSessionInfo>, (StatusCode, Json<ApiErrorResp>)> {
    if payload.url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "url is required".to_string(),
            }),
        ));
    }

    let session_id = payload
        .session_id
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid session id".to_string(),
                }),
            )
        })?
        .map(SessionId);
    let worktree_id = payload
        .worktree_id
        .as_deref()
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: "invalid worktree id".to_string(),
                }),
            )
        })?
        .map(WorktreeId);

    let info = super::launch::create_web_session(
        &state,
        WebSessionLaunchRequest {
            session_id,
            worktree_id,
            url: payload.url,
            viewport: payload.viewport,
            fps: payload.fps,
        },
    )
    .await
    .map_err(web_session_launch_error_response)?;
    Ok(Json(info))
}

fn web_session_launch_error_response(
    error: WebSessionLaunchError,
) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        WebSessionLaunchErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        WebSessionLaunchErrorKind::Forbidden => StatusCode::FORBIDDEN,
        WebSessionLaunchErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
