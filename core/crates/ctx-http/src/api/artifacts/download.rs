use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_session_tools::{
    build_session_artifact_etag, build_session_artifact_last_modified,
    parse_session_artifact_range_header, session_artifact_if_none_match_matches,
    session_artifact_if_range_allows_range_request, SessionArtifactRange,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

use super::access::{
    open_canonical_session_artifact_file, resolve_session_artifact_accessible_path,
};
use crate::daemon::AppState;

fn apply_session_artifact_response_headers(headers: &mut HeaderMap) {
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0, must-revalidate"),
    );
}

fn apply_session_artifact_etag(headers: &mut HeaderMap, etag: Option<&str>) {
    if let Some(etag) = etag {
        if let Ok(value) = HeaderValue::from_str(etag) {
            headers.insert(header::ETAG, value);
        }
    }
}

fn header_to_str(value: Option<&HeaderValue>) -> Option<&str> {
    value.and_then(|header| header.to_str().ok())
}

pub(in crate::api) async fn get_session_artifact(
    State(state): State<Arc<AppState>>,
    Path((session_id, artifact_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let session_id =
        SessionId(uuid::Uuid::parse_str(&session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let artifact_id =
        ArtifactId(uuid::Uuid::parse_str(&artifact_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let artifact = store
        .get_artifact(artifact_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    if artifact.session_id != session.id {
        return Err(StatusCode::NOT_FOUND);
    }

    let path = PathBuf::from(&artifact.absolute_path);
    let canonical_path = resolve_session_artifact_accessible_path(&state, &store, &session, &path)
        .await?
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut file = open_canonical_session_artifact_file(&canonical_path).await?;
    let meta = file.metadata().await.map_err(|_| StatusCode::NOT_FOUND)?;
    if !meta.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    let size = meta.len();
    let modified = meta.modified().ok();
    let etag = modified.and_then(|modified| build_session_artifact_etag(size, modified));
    let last_modified = modified.map(build_session_artifact_last_modified);
    let range_header = headers.get(header::RANGE);
    if etag.as_deref().is_some_and(|current_etag| {
        session_artifact_if_none_match_matches(
            header_to_str(headers.get(header::IF_NONE_MATCH)),
            current_etag,
        )
    }) {
        let mut resp = Response::new(Body::empty());
        *resp.status_mut() = StatusCode::NOT_MODIFIED;
        apply_session_artifact_response_headers(resp.headers_mut());
        apply_session_artifact_etag(resp.headers_mut(), etag.as_deref());
        if let Some(last_modified) = last_modified.as_deref() {
            if let Ok(value) = HeaderValue::from_str(last_modified) {
                resp.headers_mut().insert(header::LAST_MODIFIED, value);
            }
        }
        return Ok(resp);
    }
    let should_ignore_range = range_header.is_some()
        && headers.contains_key(header::IF_RANGE)
        && !session_artifact_if_range_allows_range_request(
            header_to_str(headers.get(header::IF_RANGE)),
            etag.as_deref(),
            last_modified.as_deref(),
        );
    let maybe_range = if should_ignore_range {
        None
    } else {
        match parse_session_artifact_range_header(header_to_str(range_header), size) {
            SessionArtifactRange::Ignore => None,
            SessionArtifactRange::Satisfiable { start, end } => Some((start, end)),
            SessionArtifactRange::Unsatisfiable => {
                let mut resp = Response::new(Body::empty());
                *resp.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                apply_session_artifact_response_headers(resp.headers_mut());
                apply_session_artifact_etag(resp.headers_mut(), etag.as_deref());
                if let Some(last_modified) = last_modified.as_deref() {
                    if let Ok(value) = HeaderValue::from_str(last_modified) {
                        resp.headers_mut().insert(header::LAST_MODIFIED, value);
                    }
                }
                if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                    resp.headers_mut().insert(header::CONTENT_RANGE, value);
                }
                return Ok(resp);
            }
        }
    };

    let (status, body, content_length, content_range) = if let Some((start, end)) = maybe_range {
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let len = end.saturating_sub(start).saturating_add(1);
        let stream = ReaderStream::new(file.take(len));
        (
            StatusCode::PARTIAL_CONTENT,
            Body::from_stream(stream),
            len,
            Some(format!("bytes {start}-{end}/{size}")),
        )
    } else {
        let stream = ReaderStream::new(file);
        (StatusCode::OK, Body::from_stream(stream), size, None)
    };

    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    apply_session_artifact_response_headers(resp.headers_mut());
    apply_session_artifact_etag(resp.headers_mut(), etag.as_deref());
    if let Some(last_modified) = last_modified.as_deref() {
        if let Ok(value) = HeaderValue::from_str(last_modified) {
            resp.headers_mut().insert(header::LAST_MODIFIED, value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        resp.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    if let Some(content_range) = content_range {
        if let Ok(value) = HeaderValue::from_str(&content_range) {
            resp.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(&artifact.mime_type) {
        resp.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    if let Some(name) = artifact.name {
        let value = format!("inline; filename=\"{}\"", name.replace('"', ""));
        if let Ok(v) = value.parse() {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}
