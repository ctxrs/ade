use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use super::super::errors::ApiErrorResp;
use super::{
    store_for_existing_session_api_error, store_for_existing_session_api_error_for_write,
    store_for_existing_session_status_for_write,
};
use crate::daemon::execution_effective;
use crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment;
use crate::daemon::AppState;
use ctx_core::ids::SessionId;
use ctx_core::models::{Session, SessionEventType};
use ctx_observability::logs;
use ctx_session_tools::model_resolution::{
    compose_model_id, normalize_effort_id, resolve_model_id,
};

mod mode;
mod model;
mod title;

pub(crate) use mode::set_session_mode;
pub(crate) use model::set_session_model;
pub(crate) use title::generate_session_title;
