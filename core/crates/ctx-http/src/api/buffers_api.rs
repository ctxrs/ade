use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use sha2::Digest;

use crate::buffers::{
    BufferCloseReq, BufferConflictResp, BufferId, BufferOpenReq, BufferOpenResp, BufferUpdateReq,
    BufferUpdateResp,
};
use crate::daemon::AppState;
use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};

fn sha256_hex(text: &str) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) async fn resolve_session_root_and_file(
    state: &Arc<AppState>,
    session_id: &str,
    path: &str,
) -> Result<(SessionId, WorkspaceId, WorktreeId, PathBuf, PathBuf, bool), StatusCode> {
    let (sid, workspace_id, worktree_id, root, is_container_file) =
        resolve_session_root(state, session_id).await?;
    if is_container_file {
        let file = crate::buffers::BufferStore::resolve_path_lexical(&root, path)
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok((sid, workspace_id, worktree_id, root, file, true))
    } else {
        let file = crate::buffers::BufferStore::resolve_path(&root, path)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        Ok((sid, workspace_id, worktree_id, root, file, false))
    }
}

pub(super) async fn resolve_session_root(
    state: &Arc<AppState>,
    session_id: &str,
) -> Result<(SessionId, WorkspaceId, WorktreeId, PathBuf, bool), StatusCode> {
    let sid = SessionId(uuid::Uuid::parse_str(session_id).map_err(|_| StatusCode::BAD_REQUEST)?);

    let store = state
        .store_for_session(sid)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let session = store
        .get_session(sid)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let wt = store
        .get_worktree(session.worktree_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(state, &wt)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let root = data_plane.live_worktree_root;
    let is_container_file = matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    );
    let root = if is_container_file {
        root
    } else {
        root.canonicalize().map_err(|_| StatusCode::BAD_REQUEST)?
    };

    Ok((
        sid,
        session.workspace_id,
        session.worktree_id,
        root,
        is_container_file,
    ))
}

pub(super) async fn open_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferOpenReq>,
) -> Result<Json<BufferOpenResp>, StatusCode> {
    let (sid, workspace_id, worktree_id, root, file, is_container_file) =
        resolve_session_root_and_file(&state, &req.session_id, &req.path).await?;
    let text = if is_container_file {
        let fs = crate::container_fs::ContainerFs::for_worktree(&state, workspace_id, worktree_id)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        fs.read_to_string(&file)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
    } else {
        tokio::fs::read_to_string(&file)
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
    };
    let disk_sha = sha256_hex(&text);
    let st = state
        .core
        .buffers
        .open_or_reuse(sid, worktree_id, root, file, text.clone(), disk_sha.clone())
        .await;
    Ok(Json(BufferOpenResp {
        buffer_id: st.id.0.to_string(),
        path: req.path,
        version: st.version,
        text: st.text,
        last_disk_sha256: st.last_disk_sha256,
    }))
}

pub(super) async fn update_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferUpdateReq>,
) -> Result<impl IntoResponse, (StatusCode, Json<BufferConflictResp>)> {
    let bid = BufferId(uuid::Uuid::parse_str(&req.buffer_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(BufferConflictResp {
                error: "invalid buffer_id".to_string(),
                disk_sha256: "".to_string(),
                disk_text: "".to_string(),
            }),
        )
    })?);
    let current = state.core.buffers.get(bid).await.ok_or((
        StatusCode::NOT_FOUND,
        Json(BufferConflictResp {
            error: "buffer not found".to_string(),
            disk_sha256: "".to_string(),
            disk_text: "".to_string(),
        }),
    ))?;
    let store = state
        .store_for_worktree(current.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                Json(BufferConflictResp {
                    error: "worktree not found".to_string(),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?;
    let wt = store
        .get_worktree(current.worktree_id)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BufferConflictResp {
                    error: "failed to load worktree".to_string(),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(BufferConflictResp {
                error: "worktree not found".to_string(),
                disk_sha256: "".to_string(),
                disk_text: "".to_string(),
            }),
        ))?;
    let data_plane = crate::worktree_data_plane::resolve_worktree_data_plane(&state, &wt)
        .await
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(BufferConflictResp {
                    error: "failed to resolve worktree data plane".to_string(),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?;
    let is_container_file = matches!(
        data_plane.execution_mode,
        crate::settings::ExecutionMode::Sandbox
    );

    let new_sha = if req.persist {
        let disk_text = if is_container_file {
            let fs = crate::container_fs::ContainerFs::for_worktree(&state, wt.workspace_id, wt.id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(BufferConflictResp {
                            error: "failed to ensure sandbox filesystem".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
            fs.read_to_string(&current.path).await.map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(BufferConflictResp {
                        error: "failed to read file".to_string(),
                        disk_sha256: "".to_string(),
                        disk_text: "".to_string(),
                    }),
                )
            })?
        } else {
            tokio::fs::read_to_string(&current.path)
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(BufferConflictResp {
                            error: "failed to read file".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?
        };
        let disk_sha = sha256_hex(&disk_text);
        if !req.force && disk_sha != current.last_disk_sha256 {
            return Err((
                StatusCode::CONFLICT,
                Json(BufferConflictResp {
                    error: "file changed on disk while buffer was open".to_string(),
                    disk_sha256: disk_sha,
                    disk_text,
                }),
            ));
        }

        if is_container_file {
            let fs = crate::container_fs::ContainerFs::for_worktree(&state, wt.workspace_id, wt.id)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(BufferConflictResp {
                            error: "failed to ensure sandbox filesystem".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
            fs.write_string(&current.path, &req.text)
                .await
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(BufferConflictResp {
                            error: crate::logs::redact_sensitive(&e.to_string()),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
        } else {
            tokio::fs::write(&current.path, req.text.as_bytes())
                .await
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(BufferConflictResp {
                            error: "failed to write file".to_string(),
                            disk_sha256: "".to_string(),
                            disk_text: "".to_string(),
                        }),
                    )
                })?;
        }
        Some(sha256_hex(&req.text))
    } else {
        None
    };
    let st = state
        .core
        .buffers
        .update(bid, req.version, req.text, new_sha)
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(BufferConflictResp {
                    error: crate::logs::redact_sensitive(&e.to_string()),
                    disk_sha256: "".to_string(),
                    disk_text: "".to_string(),
                }),
            )
        })?;

    Ok(Json(BufferUpdateResp {
        buffer_id: st.id.0.to_string(),
        version: st.version,
        last_disk_sha256: st.last_disk_sha256,
    }))
}

pub(super) async fn close_buffer(
    State(state): State<Arc<AppState>>,
    Json(req): Json<BufferCloseReq>,
) -> Result<StatusCode, StatusCode> {
    let bid = BufferId(uuid::Uuid::parse_str(&req.buffer_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    let sid =
        SessionId(uuid::Uuid::parse_str(&req.session_id).map_err(|_| StatusCode::BAD_REQUEST)?);
    if state.core.buffers.close(bid, sid).await.is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::OK)
}
