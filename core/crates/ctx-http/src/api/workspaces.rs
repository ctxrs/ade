use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};

mod active;
mod attachments;
mod context;
mod harness_container;
mod management;
mod registry;
mod worktrees;

pub(super) use active::{get_workspace_active_heads, get_workspace_active_snapshot};
pub(super) use attachments::{list_workspace_attachments, sync_workspace_attachments};
use context::*;
pub(super) use harness_container::{
    ensure_workspace_harness_container, get_workspace_harness_container,
    stop_workspace_harness_container,
};
pub(in crate::api) use management::*;
pub(super) use registry::{create_workspace, delete_workspace, get_workspace, list_workspaces};
pub(super) use worktrees::{get_worktree, get_worktree_bootstrap_logs};

use super::errors::ApiErrorResp;
use super::shared::{
    load_and_cache_workspace_files, map_effective_execution_settings_error,
    path_resolves_within_root, store_for_existing_workspace_status, FileCompletionsQuery,
};
use crate::daemon::workspaces::{vcs_hooks, WorkspaceHydrationError, WorkspaceHydrationErrorKind};
use crate::daemon::AppState;
use crate::execution_effective;
use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    AttachmentMode, AttachmentUpdatePolicy, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, WorkspaceAttachment, WorkspaceAttachmentKind, Worktree,
};
use ctx_fs::git::assert_git_repo;
use ctx_fs::vcs;
use ctx_observability::logs;
use ctx_observability::telemetry::TelemetryEvent;
use ctx_workspace_attachments::AttachmentConfig;
use ctx_workspace_config as workspace_config;
use ctx_workspace_container::WorkspaceContainerStatus as HarnessContainerStatus;
use ctx_workspace_services::file_completions;
use ctx_workspace_services::workspace_registration::detect_workspace_primary_branch;

#[derive(Debug, Deserialize)]
pub(super) struct UpdateMergeQueueConfigReq {
    enabled: bool,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    verify_command: Option<String>,
    #[serde(default)]
    push_on_success: Option<bool>,
    #[serde(default)]
    push_remote: Option<String>,
    #[serde(default)]
    push_branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct UpdateWorkspaceConfigResp {
    ok: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkspaceMergeQueueConfigResp {
    enabled: bool,
    target_branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    verify_command: Option<String>,
    push_on_success: bool,
    push_remote: String,
    push_branch: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateWorkspacePrimaryBranchReq {
    primary_branch: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkspacePrimaryBranchResp {
    primary_branch: String,
}

#[cfg(test)]
mod tests;
