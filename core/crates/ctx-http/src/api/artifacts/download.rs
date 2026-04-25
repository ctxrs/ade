use super::*;

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

fn build_session_artifact_etag(meta: &std::fs::Metadata) -> Option<String> {
    let modified = meta
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?;
    Some(format!("\"{:x}-{:x}\"", meta.len(), modified.as_nanos()))
}

fn build_session_artifact_last_modified(meta: &std::fs::Metadata) -> Option<String> {
    let modified = meta.modified().ok()?;
    let modified = chrono::DateTime::<Utc>::from(modified);
    Some(modified.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
}

fn normalize_entity_tag(tag: &str) -> &str {
    tag.trim().strip_prefix("W/").unwrap_or(tag.trim())
}

fn header_matches_if_none_match(value: Option<&HeaderValue>, etag: &str) -> bool {
    value
        .and_then(|header| header.to_str().ok())
        .is_some_and(|raw| {
            raw.split(',').any(|part| {
                let candidate = part.trim();
                candidate == "*" || normalize_entity_tag(candidate) == normalize_entity_tag(etag)
            })
        })
}

fn if_range_allows_range_request(
    value: Option<&HeaderValue>,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> bool {
    let Some(value) = value else {
        return true;
    };
    let Ok(raw) = value.to_str() else {
        return false;
    };
    let candidate = raw.trim();
    if candidate.starts_with('"') {
        return etag.is_some_and(|current_etag| candidate == current_etag);
    }
    if candidate.starts_with("W/") || candidate == "*" {
        return false;
    }
    let Some(current_last_modified) = last_modified else {
        return false;
    };
    let Ok(if_range_time) = chrono::DateTime::parse_from_rfc2822(candidate) else {
        return false;
    };
    let Ok(last_modified_time) = chrono::DateTime::parse_from_rfc2822(current_last_modified) else {
        return false;
    };
    last_modified_time <= if_range_time
}

enum ParsedRange {
    Ignore,
    Satisfiable(u64, u64),
    Unsatisfiable,
}

fn parse_decimal_u64(raw: &str) -> Option<Result<u64, ()>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(trimmed.parse::<u64>().map_err(|_| ()))
}

fn parse_range_header(range: Option<&HeaderValue>, size: u64) -> ParsedRange {
    let Some(header_value) = range else {
        return ParsedRange::Ignore;
    };
    let Ok(header) = header_value.to_str() else {
        return ParsedRange::Ignore;
    };
    let Some((unit, range)) = header.trim().split_once('=') else {
        return ParsedRange::Ignore;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") {
        return ParsedRange::Ignore;
    }
    let range = range.trim();
    if range.contains(',') {
        return ParsedRange::Ignore;
    }
    let Some((start_raw, end_raw)) = range.split_once('-') else {
        return ParsedRange::Ignore;
    };
    if start_raw.is_empty() {
        let suffix = match parse_decimal_u64(end_raw) {
            Some(Ok(value)) => value,
            Some(Err(())) => {
                if size == 0 {
                    return ParsedRange::Unsatisfiable;
                }
                return ParsedRange::Satisfiable(0, size.saturating_sub(1));
            }
            None => {
                return ParsedRange::Ignore;
            }
        };
        if suffix == 0 || size == 0 {
            return ParsedRange::Unsatisfiable;
        }
        let start = size.saturating_sub(suffix);
        let end = size.saturating_sub(1);
        return ParsedRange::Satisfiable(start, end);
    }
    let start = match parse_decimal_u64(start_raw) {
        Some(Ok(value)) => value,
        Some(Err(())) => {
            return ParsedRange::Unsatisfiable;
        }
        None => {
            return ParsedRange::Ignore;
        }
    };
    if start >= size {
        return ParsedRange::Unsatisfiable;
    }
    let end = if end_raw.is_empty() {
        size.saturating_sub(1)
    } else {
        match parse_decimal_u64(end_raw) {
            Some(Ok(value)) => value.min(size.saturating_sub(1)),
            Some(Err(())) => size.saturating_sub(1),
            None => {
                return ParsedRange::Ignore;
            }
        }
    };
    if start > end {
        return ParsedRange::Unsatisfiable;
    }
    ParsedRange::Satisfiable(start, end)
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
    let etag = build_session_artifact_etag(&meta);
    let last_modified = build_session_artifact_last_modified(&meta);
    let range_header = headers.get(header::RANGE);
    if etag.as_deref().is_some_and(|current_etag| {
        header_matches_if_none_match(headers.get(header::IF_NONE_MATCH), current_etag)
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
        && !if_range_allows_range_request(
            headers.get(header::IF_RANGE),
            etag.as_deref(),
            last_modified.as_deref(),
        );
    let maybe_range = if should_ignore_range {
        None
    } else {
        match parse_range_header(range_header, size) {
            ParsedRange::Ignore => None,
            ParsedRange::Satisfiable(start, end) => Some((start, end)),
            ParsedRange::Unsatisfiable => {
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
