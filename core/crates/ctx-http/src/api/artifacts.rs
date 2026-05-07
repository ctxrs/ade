use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use ctx_session_tools::{infer_session_artifact_mime_type, normalize_session_artifact_name};
use serde::Deserialize;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::logs;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_core::models::{Artifact, SessionEventType};

mod blob;
mod download;

pub(super) use blob::{get_blob, persist_blob_bytes, upload_blob, MAX_BLOB_MULTIPART_BODY_BYTES};
pub(super) use download::get_session_artifact;

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
