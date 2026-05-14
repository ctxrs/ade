use std::path::{Path as StdPath, PathBuf};

use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use ctx_core::ids::{ArtifactId, SessionId};
use ctx_core::models::Artifact;
use ctx_observability::logs;
use ctx_session_tools::{infer_session_artifact_mime_type, normalize_session_artifact_name};
use serde::Deserialize;

use super::super::{errors::ApiErrorResp, validate_scoped_mcp_session_context};
use super::access::{session_artifact_path_is_accessible, validate_session_artifact_write_path};

mod list;
mod set;

pub(in crate::api) use list::list_session_artifacts;
pub(in crate::api) use set::set_session_artifacts;
