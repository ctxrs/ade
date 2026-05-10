use std::io::SeekFrom;

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use ctx_session_tools::{
    parse_session_artifact_range_header, session_artifact_if_none_match_matches,
    session_artifact_if_range_allows_range_request, SessionArtifactRange,
};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

pub(super) struct SessionArtifactDownloadMetadata<'a> {
    pub(super) size: u64,
    pub(super) etag: Option<&'a str>,
    pub(super) last_modified: Option<&'a str>,
    pub(super) mime_type: &'a str,
    pub(super) name: Option<&'a str>,
}

pub(super) async fn build_session_artifact_download_response(
    request_headers: HeaderMap,
    mut file: File,
    metadata: SessionArtifactDownloadMetadata<'_>,
) -> Result<Response, StatusCode> {
    let range_header = request_headers.get(header::RANGE);
    if metadata.etag.is_some_and(|current_etag| {
        session_artifact_if_none_match_matches(
            header_to_str(request_headers.get(header::IF_NONE_MATCH)),
            current_etag,
        )
    }) {
        return Ok(not_modified_response(&metadata));
    }
    let should_ignore_range = range_header.is_some()
        && request_headers.contains_key(header::IF_RANGE)
        && !session_artifact_if_range_allows_range_request(
            header_to_str(request_headers.get(header::IF_RANGE)),
            metadata.etag,
            metadata.last_modified,
        );
    let maybe_range = if should_ignore_range {
        None
    } else {
        match parse_session_artifact_range_header(header_to_str(range_header), metadata.size) {
            SessionArtifactRange::Ignore => None,
            SessionArtifactRange::Satisfiable { start, end } => Some((start, end)),
            SessionArtifactRange::Unsatisfiable => {
                return Ok(range_not_satisfiable_response(&metadata));
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
            Some(format!("bytes {start}-{end}/{}", metadata.size)),
        )
    } else {
        let stream = ReaderStream::new(file);
        (
            StatusCode::OK,
            Body::from_stream(stream),
            metadata.size,
            None,
        )
    };

    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    apply_session_artifact_response_headers(resp.headers_mut());
    apply_session_artifact_metadata_headers(resp.headers_mut(), &metadata);
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        resp.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    if let Some(content_range) = content_range {
        if let Ok(value) = HeaderValue::from_str(&content_range) {
            resp.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    if let Ok(value) = HeaderValue::from_str(metadata.mime_type) {
        resp.headers_mut().insert(header::CONTENT_TYPE, value);
    }
    if let Some(name) = metadata.name {
        let value = format!("inline; filename=\"{}\"", name.replace('"', ""));
        if let Ok(v) = value.parse() {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}

fn not_modified_response(metadata: &SessionArtifactDownloadMetadata<'_>) -> Response {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::NOT_MODIFIED;
    apply_session_artifact_response_headers(resp.headers_mut());
    apply_session_artifact_metadata_headers(resp.headers_mut(), metadata);
    resp
}

fn range_not_satisfiable_response(metadata: &SessionArtifactDownloadMetadata<'_>) -> Response {
    let mut resp = Response::new(Body::empty());
    *resp.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
    apply_session_artifact_response_headers(resp.headers_mut());
    apply_session_artifact_metadata_headers(resp.headers_mut(), metadata);
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{}", metadata.size)) {
        resp.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    resp
}

fn apply_session_artifact_response_headers(headers: &mut HeaderMap) {
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0, must-revalidate"),
    );
}

fn apply_session_artifact_metadata_headers(
    headers: &mut HeaderMap,
    metadata: &SessionArtifactDownloadMetadata<'_>,
) {
    if let Some(etag) = metadata.etag {
        if let Ok(value) = HeaderValue::from_str(etag) {
            headers.insert(header::ETAG, value);
        }
    }
    if let Some(last_modified) = metadata.last_modified {
        if let Ok(value) = HeaderValue::from_str(last_modified) {
            headers.insert(header::LAST_MODIFIED, value);
        }
    }
}

fn header_to_str(value: Option<&HeaderValue>) -> Option<&str> {
    value.and_then(|header| header.to_str().ok())
}
