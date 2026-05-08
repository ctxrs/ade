use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use uuid::Uuid;

use super::errors::ApiErrorResp;
use crate::api::MobileAuthContext;
use crate::daemon::AppState;
use ctx_fs::vcs;
use ctx_observability::logs;

mod clone;
mod destination;
mod init;
mod status;

pub(super) use clone::repo_clone;
pub(super) use destination::{
    repo_staging_path, repo_validate_destination, repo_validate_destination_get,
};
pub(super) use init::repo_init;
pub(super) use status::repo_status;

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

fn reject_mobile_auth(
    mobile_auth: Option<Extension<MobileAuthContext>>,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    if mobile_auth.is_some() {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResp {
                error: "desktop auth required".to_string(),
            }),
        ));
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
