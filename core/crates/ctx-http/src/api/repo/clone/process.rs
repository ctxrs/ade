use std::path::{Path, PathBuf};

use super::*;

pub(super) async fn run_git_clone(
    repo_url: &str,
    branch: Option<&str>,
    dest: &Path,
) -> Result<(), (StatusCode, Json<ApiErrorResp>)> {
    let mut cmd = Command::new("git");
    cmd.arg("clone");
    if let Some(branch) = branch {
        cmd.arg("--branch").arg(branch).arg("--single-branch");
    }
    cmd.arg("--").arg(repo_url).arg(dest);

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

    Ok(())
}

pub(super) async fn canonical_clone_dest(dest: PathBuf) -> PathBuf {
    tokio::fs::canonicalize(&dest).await.unwrap_or(dest)
}
