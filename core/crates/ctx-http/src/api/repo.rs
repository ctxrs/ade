use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use super::errors::ApiErrorResp;
use crate::api::MobileAuthContext;
use ctx_daemon::daemon::repo_onboarding::{RepoOnboardingError, RepoOnboardingErrorKind};
use ctx_daemon::daemon::WorkspacesHandle;

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

fn repo_onboarding_error_response(error: RepoOnboardingError) -> (StatusCode, Json<ApiErrorResp>) {
    let status = match error.kind() {
        RepoOnboardingErrorKind::BadRequest => StatusCode::BAD_REQUEST,
        RepoOnboardingErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (
        status,
        Json(ApiErrorResp {
            error: error.message().to_string(),
        }),
    )
}
