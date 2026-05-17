use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use ctx_core::ids::{ArtifactId, SessionId};

use ctx_daemon::daemon::SessionsHandle;

#[path = "download/response.rs"]
mod response;

pub(in crate::api) async fn get_session_artifact(
    State(state): State<SessionsHandle>,
    Path((session_id, artifact_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let artifact_id =
        ArtifactId(uuid::Uuid::parse_str(&artifact_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let download = state
        .open_session_artifact_for_route(session_id, artifact_id)
        .await
        .map_err(super::session::session_artifact_status)?;
    response::build_session_artifact_download_response(
        headers,
        download.file,
        response::SessionArtifactDownloadMetadata {
            size: download.size,
            etag: download.etag.as_deref(),
            last_modified: download.last_modified.as_deref(),
            mime_type: &download.mime_type,
            name: download.name.as_deref(),
        },
    )
    .await
}
