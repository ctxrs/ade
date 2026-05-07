use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use ctx_session_tools::{
    infer_session_artifact_mime_type, infer_session_upload_blob_mime_type,
    normalize_session_artifact_name, SESSION_IMAGE_BLOB_MAX_BYTES,
    SESSION_IMAGE_BLOB_MULTIPART_MAX_BYTES, SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE,
};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::logs;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_core::models::{Artifact, SessionEventType};

mod download;

pub(super) use download::get_session_artifact;

#[derive(Debug, Serialize)]
pub(super) struct BlobUploadResp {
    pub(super) blob_id: String,
    pub(super) sha256: String,
    pub(super) bytes: i64,
    pub(super) mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
}

pub(super) const MAX_BLOB_BYTES: usize = SESSION_IMAGE_BLOB_MAX_BYTES;
pub(super) const MAX_BLOB_MULTIPART_BODY_BYTES: usize = SESSION_IMAGE_BLOB_MULTIPART_MAX_BYTES;

fn blob_upload_api_error(
    status: StatusCode,
    error: impl Into<String>,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        status,
        Json(ApiErrorResp {
            error: error.into(),
        }),
    )
}

fn blob_upload_status_error(status: StatusCode) -> (StatusCode, Json<ApiErrorResp>) {
    match status {
        StatusCode::PAYLOAD_TOO_LARGE => {
            blob_upload_api_error(status, SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE)
        }
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            blob_upload_api_error(status, "Only image attachments are supported.")
        }
        StatusCode::INTERNAL_SERVER_ERROR => {
            blob_upload_api_error(status, "Failed to store image attachment.")
        }
        _ => blob_upload_api_error(status, "Image attachment upload failed."),
    }
}

fn blob_upload_multipart_rejection_error(status: StatusCode) -> (StatusCode, Json<ApiErrorResp>) {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return blob_upload_api_error(status, SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE);
    }
    blob_upload_api_error(
        StatusCode::BAD_REQUEST,
        "Image attachment upload was not valid multipart form data.",
    )
}

fn blobs_dir(data_root: &StdPath) -> PathBuf {
    data_root.join("blobs")
}

async fn canonicalize_existing_or_raw(path: &StdPath) -> PathBuf {
    tokio::fs::canonicalize(path)
        .await
        .unwrap_or_else(|_| path.to_path_buf())
}

async fn session_artifact_allowed_roots(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &ctx_core::models::Session,
) -> Result<Vec<PathBuf>, StatusCode> {
    let mut roots = Vec::with_capacity(2);
    if let Some(worktree) = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        roots.push(canonicalize_existing_or_raw(&PathBuf::from(worktree.root_path)).await);
    }
    roots.push(
        canonicalize_existing_or_raw(
            &state
                .core
                .tool_output_spool_dir
                .join(session.id.0.to_string()),
        )
        .await,
    );
    Ok(roots)
}

async fn resolve_session_artifact_accessible_path(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &ctx_core::models::Session,
    path: &StdPath,
) -> Result<Option<PathBuf>, StatusCode> {
    let roots = session_artifact_allowed_roots(state, store, session).await?;
    let canonical = match tokio::fs::canonicalize(path).await {
        Ok(canonical) => canonical,
        Err(_) => return Ok(None),
    };
    Ok(roots
        .iter()
        .any(|root| canonical.starts_with(root))
        .then_some(canonical))
}

pub(super) async fn session_artifact_path_is_accessible(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &ctx_core::models::Session,
    path: &StdPath,
) -> Result<bool, StatusCode> {
    Ok(
        resolve_session_artifact_accessible_path(state, store, session, path)
            .await?
            .is_some(),
    )
}

pub(crate) async fn open_canonical_session_artifact_file(
    path: &StdPath,
) -> Result<tokio::fs::File, StatusCode> {
    #[cfg(unix)]
    {
        let canonical = path.to_path_buf();
        let std_file = tokio::task::spawn_blocking(move || {
            use std::os::unix::fs::OpenOptionsExt;

            let mut options = std::fs::OpenOptions::new();
            options.read(true).custom_flags(libc::O_NOFOLLOW);
            options.open(canonical)
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::NOT_FOUND)?;
        Ok(tokio::fs::File::from_std(std_file))
    }
    #[cfg(not(unix))]
    {
        tokio::fs::File::open(path)
            .await
            .map_err(|_| StatusCode::NOT_FOUND)
    }
}

async fn validate_session_artifact_write_path(
    state: &Arc<AppState>,
    store: &ctx_store::Store,
    session: &ctx_core::models::Session,
    path: &StdPath,
) -> Result<PathBuf, String> {
    let roots = session_artifact_allowed_roots(state, store, session)
        .await
        .map_err(|status| format!("failed to resolve session artifact roots: {status}"))?;
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|e| logs::redact_sensitive(&e.to_string()))?;
    if roots.iter().any(|root| canonical.starts_with(root)) {
        return Ok(canonical);
    }
    Err("absolute_file_path must stay inside the session worktree or tool-output spool".into())
}

pub(super) async fn persist_blob_bytes(
    state: &AppState,
    bytes: &[u8],
    mime_type: &str,
    name: Option<&str>,
) -> Result<BlobUploadResp, StatusCode> {
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
    req: Request,
) -> Result<Json<BlobUploadResp>, (StatusCode, Json<ApiErrorResp>)> {
    let mut multipart = Multipart::from_request(req, &state)
        .await
        .map_err(|rejection| blob_upload_multipart_rejection_error(rejection.status()))?;
    let mut file_name: Option<String> = None;
    let mut mime_type: Option<String> = None;
    let mut bytes: Option<Bytes> = None;

    while let Some(field) = multipart.next_field().await.map_err(|_| {
        blob_upload_api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE,
        )
    })? {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        if name != "file" {
            continue;
        }
        let mut field = field;
        file_name = field.file_name().map(|s| s.to_string());
        mime_type = field.content_type().map(|s| s.to_string());
        let mut field_bytes = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(|_| {
            blob_upload_api_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE,
            )
        })? {
            if field_bytes.len().saturating_add(chunk.len()) > MAX_BLOB_BYTES {
                return Err(blob_upload_api_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE,
                ));
            }
            field_bytes.extend_from_slice(&chunk);
        }
        bytes = Some(Bytes::from(field_bytes));
        break;
    }

    let Some(bytes) = bytes else {
        return Err(blob_upload_api_error(
            StatusCode::BAD_REQUEST,
            "Image attachment upload requires a file field.",
        ));
    };
    let mime_type = infer_session_upload_blob_mime_type(file_name.as_deref(), mime_type);
    let resp = persist_blob_bytes(&state, &bytes, &mime_type, file_name.as_deref())
        .await
        .map_err(blob_upload_status_error)?;
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
        if !session_artifact_path_is_accessible(
            &state,
            &store,
            &session,
            StdPath::new(&artifact.absolute_path),
        )
        .await?
        {
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
        let path = validate_session_artifact_write_path(&state, &store, &session, &path)
            .await
            .map_err(|error| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResp {
                        error: format!("artifact {} {error}", idx + 1),
                    }),
                )
            })?;

        let name = normalize_session_artifact_name(artifact.name, &path);
        let mime_type = infer_session_artifact_mime_type(&path, artifact.mime_type);
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
