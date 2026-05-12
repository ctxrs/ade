use std::sync::Arc;

use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::errors::ApiErrorResp;
use crate::api::MobileAuthContext;
use crate::daemon::AppState;
use ctx_fs::vcs;
use ctx_observability::logs;
use ctx_workspace_services::repo_onboarding::{
    ensure_git_usable, expand_tilde, validate_absolute_path, RepoGitCommandError,
    RepoOnboardingPathError,
};

mod auth;
mod clone;
mod destination;
mod init;
mod status;

use auth::reject_mobile_auth;
pub(super) use clone::repo_clone;
pub(super) use destination::{
    repo_staging_path, repo_validate_destination, repo_validate_destination_get,
};
pub(super) use init::repo_init;
pub(super) use status::repo_status;

fn repo_git_command_error_response(error: RepoGitCommandError) -> (StatusCode, Json<ApiErrorResp>) {
    if let Some(message) = error.spawn_message() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiErrorResp {
                error: format!("failed to spawn git: {message}"),
            }),
        );
    }

    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: logs::redact_sensitive(
                &error
                    .failed_message()
                    .unwrap_or_else(|| "git command failed".to_string()),
            ),
        }),
    )
}

fn repo_onboarding_path_error_response(
    error: RepoOnboardingPathError,
) -> (StatusCode, Json<ApiErrorResp>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
