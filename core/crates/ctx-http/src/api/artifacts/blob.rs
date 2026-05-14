use std::path::{Path as StdPath, PathBuf};

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use ctx_session_tools::{
    SESSION_IMAGE_BLOB_MAX_BYTES, SESSION_IMAGE_BLOB_MULTIPART_MAX_BYTES,
    SESSION_IMAGE_BLOB_TOO_LARGE_MESSAGE,
};
use serde::Serialize;
use sha2::Digest;
use tokio_util::io::ReaderStream;

use super::super::errors::ApiErrorResp;
use ctx_daemon::daemon::CoreHandle;

#[path = "blob/errors.rs"]
mod errors;
#[path = "blob/storage.rs"]
mod storage;
#[path = "blob/upload.rs"]
mod upload;

use errors::{
    blob_upload_api_error, blob_upload_multipart_rejection_error, blob_upload_status_error,
};
use storage::blobs_dir;
pub(in crate::api) use storage::persist_blob_bytes;
use upload::parse_blob_upload_file;

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

pub(in crate::api) async fn upload_blob(
    State(state): State<CoreHandle>,
    req: Request,
) -> Result<Json<BlobUploadResp>, (StatusCode, Json<ApiErrorResp>)> {
    let file = parse_blob_upload_file(req, &state).await?;
    let resp = persist_blob_bytes(&state, &file.bytes, &file.mime_type, file.name.as_deref())
        .await
        .map_err(blob_upload_status_error)?;
    Ok(Json(resp))
}

pub(in crate::api) async fn get_blob(
    State(state): State<CoreHandle>,
    Path(id): Path<String>,
) -> Result<Response, StatusCode> {
    let Some((_sha256, mime_type, _bytes, name, _created_at)) = state
        .get_blob(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    let path = blobs_dir(state.data_root()).join(&id);
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
