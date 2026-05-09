use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use ctx_session_tools::{
    infer_session_upload_blob_mime_type, SESSION_IMAGE_BLOB_MAX_BYTES,
    SESSION_IMAGE_BLOB_MULTIPART_MAX_BYTES, SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE,
};
use serde::Serialize;
use sha2::Digest;
use tokio_util::io::ReaderStream;

use super::super::errors::ApiErrorResp;
use crate::daemon::AppState;

#[derive(Debug, Serialize)]
pub(in crate::api) struct BlobUploadResp {
    pub(in crate::api) blob_id: String,
    pub(in crate::api) sha256: String,
    pub(in crate::api) bytes: i64,
    pub(in crate::api) mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::api) name: Option<String>,
}

pub(super) const MAX_BLOB_BYTES: usize = SESSION_IMAGE_BLOB_MAX_BYTES;
pub(in crate::api) const MAX_BLOB_MULTIPART_BODY_BYTES: usize =
    SESSION_IMAGE_BLOB_MULTIPART_MAX_BYTES;

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

pub(in crate::api) async fn persist_blob_bytes(
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

pub(in crate::api) async fn upload_blob(
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

pub(in crate::api) async fn get_blob(
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
