use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use super::errors::ApiErrorResp;
use crate::daemon::AppState;
use crate::logs;
use ctx_fs::vcs;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_repo_name_strips_git_and_handles_scp_style() {
        assert_eq!(
            derive_repo_name("git@github.com:org/repo.git"),
            Some("repo".to_string())
        );
        assert_eq!(
            derive_repo_name("https://github.com/org/repo"),
            Some("repo".to_string())
        );
        assert_eq!(derive_repo_name(""), None);
    }

    #[test]
    fn validate_absolute_path_rejects_relative() {
        let p = PathBuf::from("relative/path");
        assert!(validate_absolute_path(&p, "path").is_err());
    }
}

fn expand_tilde(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    if raw == "~" || raw.starts_with("~/") {
        let base = directories::BaseDirs::new()
            .ok_or_else(|| "could not resolve home directory to expand '~'".to_string())?;
        let home = base.home_dir();
        if raw == "~" {
            return Ok(home.to_path_buf());
        }
        return Ok(home.join(raw.trim_start_matches("~/")));
    }
    Ok(PathBuf::from(raw))
}

async fn ensure_git_usable() -> Result<(), String> {
    let output = Command::new("git")
        .arg("--version")
        .output()
        .await
        .map_err(|e| format!("git is required but could not be executed: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let msg = if !stderr.trim().is_empty() {
        stderr.trim().to_string()
    } else if !stdout.trim().is_empty() {
        stdout.trim().to_string()
    } else {
        format!("git exited with status {}", output.status)
    };
    Err(format!("git is required but appears unusable: {msg}"))
}

fn validate_absolute_path(path: &Path, field: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("{field} must be an absolute path"));
    }
    Ok(())
}

fn derive_repo_name(repo_url: &str) -> Option<String> {
    let url = repo_url.trim().trim_end_matches('/');
    if url.is_empty() {
        return None;
    }
    // Handle `git@github.com:org/repo.git` style URLs by treating `:` as a path separator.
    let normalized = url.replace(':', "/");
    let last = normalized.split('/').next_back()?.trim();
    if last.is_empty() {
        return None;
    }
    let name = last.strip_suffix(".git").unwrap_or(last);
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn validate_dest_name(name: &str) -> Result<(), String> {
    let s = name.trim();
    if s.is_empty() {
        return Err("dest_name must be non-empty".to_string());
    }
    let p = Path::new(s);
    if p.is_absolute() {
        return Err("dest_name must be a single path segment".to_string());
    }
    let mut components = p.components();
    let first = components.next();
    if components.next().is_some() {
        return Err("dest_name must be a single path segment".to_string());
    }
    match first {
        Some(std::path::Component::Normal(_)) => Ok(()),
        _ => Err("dest_name must be a single path segment".to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct RepoCloneReq {
    repo_url: String,
    dest_parent: String,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    dest_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct RepoCloneResp {
    path: String,
}

pub(super) async fn repo_clone(
    Json(req): Json<RepoCloneReq>,
) -> Result<Json<RepoCloneResp>, (StatusCode, Json<ApiErrorResp>)> {
    ensure_git_usable()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    let repo_url = req.repo_url.trim().to_string();
    if repo_url.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "repo_url is required".to_string(),
            }),
        ));
    }

    let dest_parent = expand_tilde(&req.dest_parent)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    validate_absolute_path(&dest_parent, "dest_parent")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    // Allow cloning into a destination parent that doesn't exist yet by creating it.
    // This keeps the wizard UX simple (users can type a new folder path).
    if !dest_parent.exists() {
        tokio::fs::create_dir_all(&dest_parent).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!(
                        "failed to create dest_parent '{}': {e}",
                        dest_parent.to_string_lossy()
                    ),
                }),
            )
        })?;
    }

    let dest_parent = tokio::fs::canonicalize(&dest_parent).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!(
                    "invalid dest_parent '{}': {}",
                    dest_parent.to_string_lossy(),
                    e
                ),
            }),
        )
    })?;

    let name = req
        .dest_name
        .as_ref()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| derive_repo_name(&repo_url))
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "could not derive repo name".to_string(),
            }),
        ))?;

    if let Err(e) = validate_dest_name(&name) {
        return Err((StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })));
    }

    let dest = dest_parent.join(&name);
    if dest.exists() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("destination already exists: {}", dest.display()),
            }),
        ));
    }

    let mut cmd = Command::new("git");
    cmd.arg("clone");
    if let Some(branch) = req
        .branch
        .as_ref()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
    {
        cmd.arg("--branch").arg(branch).arg("--single-branch");
    }
    cmd.arg("--").arg(&repo_url).arg(&dest);

    let output = cmd.output().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to spawn git: {e}"),
            }),
        )
    })?;
    if !output.status.success() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&format!(
                    "git clone failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }),
        ));
    }

    let canonical_dest = tokio::fs::canonicalize(&dest).await.unwrap_or(dest);

    Ok(Json(RepoCloneResp {
        path: canonical_dest.to_string_lossy().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct RepoInitReq {
    path: String,
    #[serde(default)]
    allow_existing: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct RepoInitResp {
    path: String,
}

pub(super) async fn repo_init(
    Json(req): Json<RepoInitReq>,
) -> Result<Json<RepoInitResp>, (StatusCode, Json<ApiErrorResp>)> {
    ensure_git_usable()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    let raw = req.path.trim();
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "path is required".to_string(),
            }),
        ));
    }

    let path = expand_tilde(raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    validate_absolute_path(&path, "path")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    if path.exists() && !req.allow_existing {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("destination already exists: {}", path.display()),
            }),
        ));
    }
    tokio::fs::create_dir_all(&path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("failed to create directory '{}': {e}", path.display()),
            }),
        )
    })?;

    // Refuse to init into a non-empty directory. Users can still choose "Import folder" for
    // existing projects, or explicitly run `git init` themselves.
    let mut dir = tokio::fs::read_dir(&path).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("failed to read directory '{}': {e}", path.display()),
            }),
        )
    })?;
    if dir
        .next_entry()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResp {
                    error: format!("failed to read directory '{}': {e}", path.display()),
                }),
            )
        })?
        .is_some()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("destination is not empty: {}", path.display()),
            }),
        ));
    }

    let output = Command::new("git")
        .arg("init")
        .arg("--")
        .arg(&path)
        .output()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to spawn git: {e}"),
                }),
            )
        })?;
    if !output.status.success() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&format!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }),
        ));
    }

    // Worktrees require a base commit to diff against. `git init` alone yields a repo with no
    // commits, which breaks the out-of-the-box wizard path ("New repo").
    //
    // We create an empty initial commit using inline identity overrides, so we don't depend on the
    // user's global git config (user.name/user.email).
    let output = Command::new("git")
        .arg("-C")
        .arg(&path)
        .arg("-c")
        .arg("user.name=ctx")
        .arg("-c")
        .arg("user.email=ctx@localhost")
        .arg("commit")
        .arg("--allow-empty")
        .arg("-m")
        .arg("Initial commit")
        .output()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: format!("failed to spawn git: {e}"),
                }),
            )
        })?;
    if !output.status.success() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&format!(
                    "git commit failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )),
            }),
        ));
    }

    let canonical = tokio::fs::canonicalize(&path).await.unwrap_or(path);

    Ok(Json(RepoInitResp {
        path: canonical.to_string_lossy().to_string(),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct RepoStatusReq {
    path: String,
}

#[derive(Debug, Serialize)]
pub(super) struct RepoStatusResp {
    canonical_path: String,
    is_repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(super) async fn repo_status(
    Json(req): Json<RepoStatusReq>,
) -> Result<Json<RepoStatusResp>, (StatusCode, Json<ApiErrorResp>)> {
    ensure_git_usable()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;

    let raw = req.path.trim();
    if raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "path is required".to_string(),
            }),
        ));
    }
    let expanded = expand_tilde(raw)
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;
    validate_absolute_path(&expanded, "path")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(ApiErrorResp { error: e })))?;
    let canonical = tokio::fs::canonicalize(&expanded).await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: format!("invalid path '{}': {e}", expanded.to_string_lossy()),
            }),
        )
    })?;

    let canonical_str = canonical.to_string_lossy().to_string();
    let driver = match vcs::driver_for_path(&canonical).await {
        Ok(d) => d,
        Err(err) => {
            return Ok(Json(RepoStatusResp {
                canonical_path: canonical_str,
                is_repo: false,
                error: Some(logs::redact_sensitive(&err.to_string())),
            }))
        }
    };
    match driver.assert_repo(&canonical).await {
        Ok(()) => Ok(Json(RepoStatusResp {
            canonical_path: canonical_str,
            is_repo: true,
            error: None,
        })),
        Err(err) => Ok(Json(RepoStatusResp {
            canonical_path: canonical_str,
            is_repo: false,
            error: Some(logs::redact_sensitive(&err.to_string())),
        })),
    }
}

#[derive(Debug, Serialize)]
pub(super) struct RepoStagingPathResp {
    path: String,
}

/// Returns a unique staging path under data_root/workspaces/staging/<uuid>.
/// Used for disk-isolated clone/new: the daemon manages the path so the wizard
/// doesn't need to ask the user for a host destination.
pub(super) async fn repo_staging_path(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RepoStagingPathResp>, (StatusCode, Json<ApiErrorResp>)> {
    let staging_dir = state
        .core
        .data_root
        .join("workspaces")
        .join("staging")
        .join(Uuid::new_v4().to_string());

    tokio::fs::create_dir_all(&staging_dir).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!(
                    "failed to create staging dir '{}': {e}",
                    staging_dir.display()
                ),
            }),
        )
    })?;

    let path = tokio::fs::canonicalize(&staging_dir)
        .await
        .unwrap_or(staging_dir)
        .to_string_lossy()
        .to_string();

    Ok(Json(RepoStagingPathResp { path }))
}

// Keep rustfmt from reordering these unused imports in some feature combos.
#[allow(dead_code)]
fn _unused(_r: Response) {}
