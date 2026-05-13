use axum::http::StatusCode;
use serde::Deserialize;

use crate::daemon::workspaces::{FileCompletionsError, FileCompletionsErrorKind};

#[derive(Debug, Deserialize, Default)]
pub(crate) struct FileCompletionsQuery {
    pub(crate) query: Option<String>,
    pub(crate) limit: Option<u32>,
}

pub(crate) fn map_file_completions_error(error: FileCompletionsError) -> StatusCode {
    match error.kind() {
        FileCompletionsErrorKind::NotFound => StatusCode::NOT_FOUND,
        FileCompletionsErrorKind::Forbidden => StatusCode::FORBIDDEN,
        FileCompletionsErrorKind::InsufficientStorage => StatusCode::INSUFFICIENT_STORAGE,
        FileCompletionsErrorKind::Internal => {
            tracing::warn!(error = error.message(), "file completions request failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}
