use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;

use axum::http::StatusCode;
use ctx_observability::logs;

use crate::daemon::AppState;

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

pub(in crate::api::artifacts) async fn resolve_session_artifact_accessible_path(
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

pub(in crate::api) async fn session_artifact_path_is_accessible(
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

pub(in crate::api::artifacts) async fn validate_session_artifact_write_path(
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
