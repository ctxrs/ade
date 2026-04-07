use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Multipart, Path, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::logs;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_core::models::{Artifact, SessionEventType};

#[derive(Debug, Serialize)]
pub(super) struct BlobUploadResp {
    pub(super) blob_id: String,
    pub(super) sha256: String,
    pub(super) bytes: i64,
    pub(super) mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
}

fn blobs_dir(data_root: &StdPath) -> PathBuf {
    data_root.join("blobs")
}

fn infer_upload_blob_mime_type(file_name: Option<&str>, override_value: Option<String>) -> String {
    match file_name {
        Some(name) => infer_artifact_mime_type(StdPath::new(name), override_value),
        None => override_value.unwrap_or_else(|| "application/octet-stream".to_string()),
    }
}

pub(super) async fn persist_blob_bytes(
    state: &AppState,
    bytes: &[u8],
    mime_type: &str,
    name: Option<&str>,
) -> Result<BlobUploadResp, StatusCode> {
    const MAX_BLOB_BYTES: usize = 25 * 1024 * 1024;
    if bytes.len() > MAX_BLOB_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if !mime_type.starts_with("image/") {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    let sha256 = hex::encode(hasher.finalize());

    let blob_id = uuid::Uuid::new_v4().to_string();

    let dir = blobs_dir(&state.core.data_root);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let path = dir.join(&blob_id);
    let tmp = dir.join(format!("{blob_id}.tmp"));

    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    state
        .global_store()
        .insert_blob(
            &blob_id,
            &sha256,
            bytes.len() as i64,
            mime_type,
            name,
            Utc::now(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(BlobUploadResp {
        blob_id,
        sha256,
        bytes: bytes.len() as i64,
        mime_type: mime_type.to_string(),
        name: name.map(|s| s.to_string()),
    })
}

pub(super) async fn upload_blob(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<BlobUploadResp>, StatusCode> {
    let mut file_name: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut bytes: Option<Bytes> = None;

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        if name != "file" {
            continue;
        }
        file_name = field.file_name().map(|s| s.to_string());
        mime_type = field.content_type().map(|s| s.to_string());
        let b = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
        bytes = Some(b);
        break;
    }

    let Some(bytes) = bytes else {
        return Err(StatusCode::BAD_REQUEST);
    };
    let mime_type = infer_upload_blob_mime_type(file_name.as_deref(), mime_type);
    let resp = persist_blob_bytes(&state, &bytes, &mime_type, file_name.as_deref()).await?;
    Ok(Json(resp))
}

pub(super) async fn get_blob(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let Some((_sha256, mime_type, _bytes, name, _created_at)) = state
        .global_store()
        .get_blob(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    let path = blobs_dir(&state.core.data_root).join(&id);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

    let stream = ReaderStream::new(file);
    let mut resp = Response::new(Body::from_stream(stream));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        mime_type
            .parse()
            .unwrap_or_else(|_| header::HeaderValue::from_static("application/octet-stream")),
    );
    if let Some(name) = name {
        let value = format!("inline; filename=\"{}\"", name.replace('"', ""));
        if let Ok(v) = value.parse() {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}

fn normalize_artifact_name(name: Option<String>, path: &StdPath) -> Option<String> {
    if let Some(name) = name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

fn infer_artifact_mime_type(path: &StdPath, override_value: Option<String>) -> String {
    if let Some(value) = override_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("mdx"))
    {
        return "text/markdown".to_string();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

fn parse_range_header(range: Option<&HeaderValue>, size: u64) -> Option<(u64, u64)> {
    let header = range?.to_str().ok()?.trim().to_string();
    let range = header.strip_prefix("bytes=")?;
    let (start_raw, end_raw) = range.split_once('-')?;
    if start_raw.is_empty() {
        let suffix: u64 = end_raw.parse().ok()?;
        if suffix == 0 || size == 0 {
            return None;
        }
        let start = size.saturating_sub(suffix);
        let end = size.saturating_sub(1);
        return Some((start, end));
    }
    let start: u64 = start_raw.parse().ok()?;
    if start >= size {
        return None;
    }
    let end = if end_raw.is_empty() {
        size.saturating_sub(1)
    } else {
        end_raw.parse::<u64>().ok()?.min(size.saturating_sub(1))
    };
    if start > end {
        return None;
    }
    Some((start, end))
}

pub(super) async fn get_session_artifact(
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
    let meta = tokio::fs::metadata(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !meta.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    let size = meta.len();
    let maybe_range = parse_range_header(headers.get(header::RANGE), size);

    let mut file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;

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
            Some(format!("bytes {}-{}/{}", start, end, size)),
        )
    } else {
        let stream = ReaderStream::new(file);
        (StatusCode::OK, Body::from_stream(stream), size, None)
    };

    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    resp.headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        resp.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    if let Some(content_range) = content_range {
        if let Ok(value) = HeaderValue::from_str(&content_range) {
            resp.headers_mut().insert(header::CONTENT_RANGE, value);
        }
    }
    if let Some(name) = artifact.name {
        let value = format!("inline; filename=\"{}\"", name.replace('"', ""));
        if let Ok(v) = value.parse() {
            resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
        }
    }
    Ok(resp)
}

#[derive(Debug, Deserialize)]
struct ArtifactInput {
    absolute_file_path: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetSessionArtifactsReq {
    #[serde(default)]
    artifacts: Vec<ArtifactInput>,
}

pub(super) async fn list_session_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Artifact>>, StatusCode> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let store = state
        .store_for_session(session_id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let mut artifacts = store
        .list_session_artifacts(session.id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for artifact in artifacts.iter_mut() {
        if tokio::fs::metadata(&artifact.absolute_path).await.is_err() {
            artifact.missing = Some(true);
        }
    }

    Ok(Json(artifacts))
}

pub(super) async fn set_session_artifacts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionArtifactsReq>,
) -> Result<Json<Vec<Artifact>>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);
    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let mut artifacts = Vec::with_capacity(req.artifacts.len());
    for (idx, artifact) in req.artifacts.into_iter().enumerate() {
        let raw = artifact.absolute_file_path.trim();
        if raw.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("artifact {} missing absolute_file_path", idx + 1),
                }),
            ));
        }
        let path = PathBuf::from(raw);
        if !path.is_absolute() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("artifact {} absolute_file_path must be absolute", idx + 1),
                }),
            ));
        }
        let meta = tokio::fs::metadata(&path).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "artifact {} path not accessible: {}",
                        idx + 1,
                        logs::redact_sensitive(&e.to_string())
                    ),
                }),
            )
        })?;
        if !meta.is_file() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("artifact {} path is not a file", idx + 1),
                }),
            ));
        }

        let name = normalize_artifact_name(artifact.name, &path);
        let mime_type = infer_artifact_mime_type(&path, artifact.mime_type);
        let bytes = meta.len() as i64;
        let created_at = Utc::now();

        artifacts.push(Artifact {
            id: ArtifactId::new(),
            session_id: session.id,
            task_id: session.task_id,
            workspace_id: session.workspace_id,
            worktree_id: session.worktree_id,
            name,
            absolute_path: path.to_string_lossy().to_string(),
            mime_type,
            bytes,
            created_at,
            missing: None,
        });
    }

    store
        .replace_session_artifacts(session.id, &artifacts)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let event = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::ArtifactsSet,
            serde_json::json!({ "artifacts": artifacts }),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state.publish_event(event).await;

    Ok(Json(artifacts))
}

#[cfg(test)]
mod tests {
    use super::infer_artifact_mime_type;
    use std::path::Path;

    #[test]
    fn infer_artifact_mime_type_treats_mdx_as_markdown() {
        assert_eq!(
            infer_artifact_mime_type(Path::new("/tmp/merge-queue-for-agents.mdx"), None),
            "text/markdown"
        );
    }

    #[test]
    fn infer_artifact_mime_type_preserves_explicit_override() {
        assert_eq!(
            infer_artifact_mime_type(
                Path::new("/tmp/merge-queue-for-agents.mdx"),
                Some("application/mdx".to_string())
            ),
            "application/mdx"
        );
    }

    #[test]
    fn infer_artifact_mime_type_keeps_existing_markdown_inference() {
        assert_eq!(
            infer_artifact_mime_type(Path::new("/tmp/notes.md"), None),
            "text/markdown"
        );
    }
}
