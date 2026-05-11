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

mod auth;
mod clone;
mod destination;
mod git;
mod init;
mod path;
mod status;

use auth::reject_mobile_auth;
pub(super) use clone::repo_clone;
pub(super) use destination::{
    repo_staging_path, repo_validate_destination, repo_validate_destination_get,
};
use git::ensure_git_usable;
pub(super) use init::repo_init;
use path::{derive_repo_name, expand_tilde, validate_absolute_path, validate_dest_name};
pub(super) use status::repo_status;
