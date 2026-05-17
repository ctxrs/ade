use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::Json;
use ctx_core::ids::SessionId;
use ctx_core::models::Artifact;
use ctx_daemon::daemon::sessions::{SessionArtifactInput, SessionArtifactRouteError};
use serde::Deserialize;

use super::super::{errors::ApiErrorResp, validate_scoped_mcp_session_context};

mod list;
mod set;

pub(in crate::api) use list::list_session_artifacts;
pub(in crate::api) use set::set_session_artifacts;

fn session_artifact_api_error(
    error: SessionArtifactRouteError,
) -> (StatusCode, Json<ApiErrorResp>) {
    match error {
        SessionArtifactRouteError::NotFound => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ),
        SessionArtifactRouteError::BadRequest(error) => {
            (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error }))
        }
        SessionArtifactRouteError::Internal(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp { error }),
        ),
    }
}

pub(in crate::api::artifacts) fn session_artifact_status(
    error: SessionArtifactRouteError,
) -> StatusCode {
    match error {
        SessionArtifactRouteError::NotFound | SessionArtifactRouteError::BadRequest(_) => {
            StatusCode::NOT_FOUND
        }
        SessionArtifactRouteError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
