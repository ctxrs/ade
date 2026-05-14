use std::path::PathBuf;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_session_tools::{build_session_artifact_etag, build_session_artifact_last_modified};

use super::access::{
    open_canonical_session_artifact_file, resolve_session_artifact_accessible_path,
};
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
    let (session, artifact) = state
        .get_session_artifact_for_download(session_id, artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let path = PathBuf::from(&artifact.absolute_path);
    let canonical_path = resolve_session_artifact_accessible_path(&state, &session, &path)
        .await?
        .ok_or(StatusCode::NOT_FOUND)?;
    let file = open_canonical_session_artifact_file(&canonical_path).await?;
    let meta = file.metadata().await.map_err(|_| StatusCode::NOT_FOUND)?;
    if !meta.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    let size = meta.len();
    let modified = meta.modified().ok();
    let etag = modified.and_then(|modified| build_session_artifact_etag(size, modified));
    let last_modified = modified.map(build_session_artifact_last_modified);
    response::build_session_artifact_download_response(
        headers,
        file,
        response::SessionArtifactDownloadMetadata {
            size,
            etag: etag.as_deref(),
            last_modified: last_modified.as_deref(),
            mime_type: &artifact.mime_type,
            name: artifact.name.as_deref(),
        },
    )
    .await
}
