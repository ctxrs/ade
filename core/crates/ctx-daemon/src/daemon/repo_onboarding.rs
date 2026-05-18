use std::path::PathBuf;

use ctx_observability::logs;
use ctx_workspace_services::repo_onboarding as service;
use serde::{Deserialize, Serialize};

use crate::daemon::WorkspacesHandle;

#[derive(Debug, Clone)]
struct DaemonRepoInitRequest {
    pub path: String,
    pub allow_existing: bool,
    pub allow_non_empty: bool,
}

#[derive(Debug, Clone)]
struct DaemonRepoCloneRequest {
    pub repo_url: String,
    pub dest_parent: String,
    pub branch: Option<String>,
    pub dest_name: Option<String>,
}

#[derive(Debug, Clone)]
struct DaemonRepoValidateDestinationRequest {
    pub path: String,
    pub must_not_exist: bool,
    pub require_empty_if_exists: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DaemonRepoStatusCheck {
    pub canonical_path: PathBuf,
    pub is_repo: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum RepoOnboardingErrorKind {
    BadRequest,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct RepoOnboardingError {
    kind: RepoOnboardingErrorKind,
    message: String,
}

impl RepoOnboardingError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: RepoOnboardingErrorKind::BadRequest,
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: RepoOnboardingErrorKind::Internal,
            message: message.into(),
        }
    }

    fn kind(&self) -> RepoOnboardingErrorKind {
        self.kind
    }

    fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct RepoInitRouteRequest {
    path: String,
    #[serde(default)]
    allow_existing: bool,
    #[serde(default)]
    allow_non_empty: bool,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct RepoCloneRouteRequest {
    repo_url: String,
    dest_parent: String,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    dest_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct RepoValidateDestinationRouteRequest {
    path: String,
    #[serde(default)]
    must_not_exist: bool,
    #[serde(default)]
    require_empty_if_exists: bool,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct RepoStatusRouteRequest {
    path: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RepoPathRouteResponse {
    path: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RepoStatusRouteResponse {
    canonical_path: String,
    is_repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RepoOnboardingRouteErrorKind {
    BadRequest,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RepoOnboardingRouteError {
    kind: RepoOnboardingRouteErrorKind,
    message: String,
}

impl RepoOnboardingRouteError {
    fn new(kind: RepoOnboardingRouteErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> RepoOnboardingRouteErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<RepoOnboardingError> for RepoOnboardingRouteError {
    fn from(error: RepoOnboardingError) -> Self {
        let kind = match error.kind() {
            RepoOnboardingErrorKind::BadRequest => RepoOnboardingRouteErrorKind::BadRequest,
            RepoOnboardingErrorKind::Internal => RepoOnboardingRouteErrorKind::Internal,
        };
        Self::new(kind, error.message())
    }
}

fn repo_path_route_response(path: PathBuf) -> RepoPathRouteResponse {
    RepoPathRouteResponse {
        path: path.to_string_lossy().to_string(),
    }
}

fn repo_status_route_response(status: DaemonRepoStatusCheck) -> RepoStatusRouteResponse {
    RepoStatusRouteResponse {
        canonical_path: status.canonical_path.to_string_lossy().to_string(),
        is_repo: status.is_repo,
        error: status.error,
    }
}

fn repo_git_command_error(error: service::RepoGitCommandError) -> RepoOnboardingError {
    if let Some(message) = error.spawn_message() {
        return RepoOnboardingError::internal(format!("failed to spawn git: {message}"));
    }
    RepoOnboardingError::bad_request(logs::redact_sensitive(
        &error
            .failed_message()
            .unwrap_or_else(|| "git command failed".to_string()),
    ))
}

fn repo_path_error(error: service::RepoOnboardingPathError) -> RepoOnboardingError {
    RepoOnboardingError::bad_request(error.message().to_string())
}

fn repo_staging_path_error(error: service::RepoOnboardingPathError) -> RepoOnboardingError {
    RepoOnboardingError::internal(error.message().to_string())
}

fn repo_workflow_error(error: service::RepoOnboardingWorkflowError) -> RepoOnboardingError {
    match error {
        service::RepoOnboardingWorkflowError::GitPreflight(error) => {
            RepoOnboardingError::bad_request(error)
        }
        service::RepoOnboardingWorkflowError::GitCommand(error) => repo_git_command_error(error),
        service::RepoOnboardingWorkflowError::Path(error) => repo_path_error(error),
    }
}

impl WorkspacesHandle {
    async fn initialize_repo(
        &self,
        req: DaemonRepoInitRequest,
    ) -> Result<PathBuf, RepoOnboardingError> {
        service::initialize_repo(service::RepoInitRequest {
            path: &req.path,
            allow_existing: req.allow_existing,
            allow_non_empty: req.allow_non_empty,
        })
        .await
        .map_err(repo_workflow_error)
    }

    async fn clone_repo(
        &self,
        req: DaemonRepoCloneRequest,
    ) -> Result<PathBuf, RepoOnboardingError> {
        service::clone_repo(service::RepoCloneRequest {
            repo_url: &req.repo_url,
            dest_parent: &req.dest_parent,
            branch: req.branch.as_deref(),
            dest_name: req.dest_name.as_deref(),
        })
        .await
        .map_err(repo_workflow_error)
    }

    async fn validate_repo_destination(
        &self,
        req: DaemonRepoValidateDestinationRequest,
    ) -> Result<PathBuf, RepoOnboardingError> {
        service::validate_repo_destination(service::RepoValidateDestinationRequest {
            path: &req.path,
            must_not_exist: req.must_not_exist,
            require_empty_if_exists: req.require_empty_if_exists,
        })
        .await
        .map_err(repo_path_error)
    }

    async fn create_repo_staging_path(&self) -> Result<PathBuf, RepoOnboardingError> {
        service::create_repo_staging_path(&self.state.core.data_root)
            .await
            .map_err(repo_staging_path_error)
    }

    async fn inspect_repo_status(
        &self,
        path: &str,
    ) -> Result<DaemonRepoStatusCheck, RepoOnboardingError> {
        let status = service::inspect_repo_status(path)
            .await
            .map_err(repo_workflow_error)?;
        Ok(DaemonRepoStatusCheck {
            canonical_path: status.canonical_path,
            is_repo: status.is_repo,
            error: status.error.map(|error| logs::redact_sensitive(&error)),
        })
    }

    pub async fn initialize_repo_for_route(
        &self,
        req: RepoInitRouteRequest,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        self.initialize_repo(DaemonRepoInitRequest {
            path: req.path,
            allow_existing: req.allow_existing,
            allow_non_empty: req.allow_non_empty,
        })
        .await
        .map(repo_path_route_response)
        .map_err(Into::into)
    }

    pub async fn clone_repo_for_route(
        &self,
        req: RepoCloneRouteRequest,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        self.clone_repo(DaemonRepoCloneRequest {
            repo_url: req.repo_url,
            dest_parent: req.dest_parent,
            branch: req.branch,
            dest_name: req.dest_name,
        })
        .await
        .map(repo_path_route_response)
        .map_err(Into::into)
    }

    pub async fn validate_repo_destination_for_route(
        &self,
        req: RepoValidateDestinationRouteRequest,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        self.validate_repo_destination(DaemonRepoValidateDestinationRequest {
            path: req.path,
            must_not_exist: req.must_not_exist,
            require_empty_if_exists: req.require_empty_if_exists,
        })
        .await
        .map(repo_path_route_response)
        .map_err(Into::into)
    }

    pub async fn create_repo_staging_path_for_route(
        &self,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        self.create_repo_staging_path()
            .await
            .map(repo_path_route_response)
            .map_err(Into::into)
    }

    pub async fn inspect_repo_status_for_route(
        &self,
        req: RepoStatusRouteRequest,
    ) -> Result<RepoStatusRouteResponse, RepoOnboardingRouteError> {
        self.inspect_repo_status(&req.path)
            .await
            .map(repo_status_route_response)
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    use crate::test_support::TestDaemon;

    async fn test_workspaces_handle() -> (tempfile::TempDir, WorkspacesHandle) {
        let data_root = tempdir().expect("data root");
        let daemon = TestDaemon::new_for_test(
            data_root.path().to_path_buf(),
            "http://127.0.0.1:4567".to_string(),
        )
        .await
        .expect("test daemon");
        (data_root, daemon.handle().workspaces())
    }

    #[test]
    fn repo_error_conversions_redact_and_classify_git_failures() {
        let spawn = repo_git_command_error(service::RepoGitCommandError::Spawn {
            message: "permission denied".to_string(),
        });
        assert_eq!(spawn.kind(), RepoOnboardingErrorKind::Internal);
        assert_eq!(spawn.message(), "failed to spawn git: permission denied");

        let failed = repo_git_command_error(service::RepoGitCommandError::Failed {
            action: "git clone",
            stderr: "fatal: token=secret-token\n".to_string(),
        });
        assert_eq!(failed.kind(), RepoOnboardingErrorKind::BadRequest);
        assert!(failed.message().contains("git clone failed"));
        assert!(!failed.message().contains("secret-token"));
    }

    #[test]
    fn repo_error_conversions_preserve_preflight_and_path_messages() {
        let preflight = repo_workflow_error(service::RepoOnboardingWorkflowError::GitPreflight(
            "git is required".to_string(),
        ));
        assert_eq!(preflight.kind(), RepoOnboardingErrorKind::BadRequest);
        assert_eq!(preflight.message(), "git is required");

        let path = repo_path_error(service::RepoOnboardingPathError::from(
            "path is required".to_string(),
        ));
        assert_eq!(path.kind(), RepoOnboardingErrorKind::BadRequest);
        assert_eq!(path.message(), "path is required");

        let staging = repo_staging_path_error(service::RepoOnboardingPathError::from(
            "failed to create staging dir".to_string(),
        ));
        assert_eq!(staging.kind(), RepoOnboardingErrorKind::Internal);
        assert_eq!(staging.message(), "failed to create staging dir");
    }

    #[test]
    fn route_requests_preserve_serde_defaults() {
        let init: RepoInitRouteRequest = serde_json::from_value(json!({
            "path": "/tmp/repo",
        }))
        .expect("init route request");
        assert_eq!(init.path, "/tmp/repo");
        assert!(!init.allow_existing);
        assert!(!init.allow_non_empty);

        let clone: RepoCloneRouteRequest = serde_json::from_value(json!({
            "repo_url": "https://example.invalid/repo.git",
            "dest_parent": "/tmp",
        }))
        .expect("clone route request");
        assert_eq!(clone.repo_url, "https://example.invalid/repo.git");
        assert_eq!(clone.dest_parent, "/tmp");
        assert_eq!(clone.branch, None);
        assert_eq!(clone.dest_name, None);

        let destination: RepoValidateDestinationRouteRequest = serde_json::from_value(json!({
            "path": "/tmp/repo",
        }))
        .expect("destination route request");
        assert_eq!(destination.path, "/tmp/repo");
        assert!(!destination.must_not_exist);
        assert!(!destination.require_empty_if_exists);

        let status: RepoStatusRouteRequest = serde_json::from_value(json!({
            "path": "/tmp/repo",
        }))
        .expect("status route request");
        assert_eq!(status.path, "/tmp/repo");
    }

    #[test]
    fn route_responses_preserve_wire_shapes() {
        let path_response = repo_path_route_response(PathBuf::from("/tmp/repo"));
        assert_eq!(
            serde_json::to_value(path_response).unwrap(),
            json!({
                "path": "/tmp/repo",
            })
        );

        let status_without_error = repo_status_route_response(DaemonRepoStatusCheck {
            canonical_path: PathBuf::from("/tmp/repo"),
            is_repo: true,
            error: None,
        });
        assert_eq!(
            serde_json::to_value(status_without_error).unwrap(),
            json!({
                "canonical_path": "/tmp/repo",
                "is_repo": true,
            })
        );

        let status_with_error = repo_status_route_response(DaemonRepoStatusCheck {
            canonical_path: PathBuf::from("/tmp/repo"),
            is_repo: false,
            error: Some("not a repo".to_string()),
        });
        assert_eq!(
            serde_json::to_value(status_with_error).unwrap(),
            json!({
                "canonical_path": "/tmp/repo",
                "is_repo": false,
                "error": "not a repo",
            })
        );
    }

    #[test]
    fn route_errors_preserve_daemon_categories_and_messages() {
        let bad_request =
            RepoOnboardingRouteError::from(RepoOnboardingError::bad_request("path is required"));
        assert_eq!(bad_request.kind(), RepoOnboardingRouteErrorKind::BadRequest);
        assert_eq!(bad_request.message(), "path is required");

        let internal = RepoOnboardingRouteError::from(RepoOnboardingError::internal(
            "failed to create staging dir",
        ));
        assert_eq!(internal.kind(), RepoOnboardingRouteErrorKind::Internal);
        assert_eq!(internal.message(), "failed to create staging dir");
    }

    #[tokio::test]
    async fn create_repo_staging_path_uses_daemon_data_root() {
        let (data_root, workspaces) = test_workspaces_handle().await;

        let staging = workspaces
            .create_repo_staging_path()
            .await
            .expect("staging path");

        assert!(staging.exists());
        assert!(staging.starts_with(data_root.path().join("workspaces").join("staging")));

        let response = workspaces
            .create_repo_staging_path_for_route()
            .await
            .expect("route staging path");
        let value = serde_json::to_value(response).expect("route response json");
        let route_path = PathBuf::from(
            value
                .get("path")
                .and_then(|path| path.as_str())
                .expect("path field"),
        );
        assert!(route_path.exists());
        assert!(route_path.starts_with(data_root.path().join("workspaces").join("staging")));
    }

    #[tokio::test]
    async fn validate_repo_destination_preserves_path_error_behavior() {
        let (_data_root, workspaces) = test_workspaces_handle().await;

        let error = workspaces
            .validate_repo_destination(DaemonRepoValidateDestinationRequest {
                path: "   ".to_string(),
                must_not_exist: false,
                require_empty_if_exists: false,
            })
            .await
            .expect_err("blank path should fail");

        assert_eq!(error.kind(), RepoOnboardingErrorKind::BadRequest);
        assert_eq!(error.message(), "path is required");

        let route_error = workspaces
            .validate_repo_destination_for_route(RepoValidateDestinationRouteRequest {
                path: "   ".to_string(),
                must_not_exist: false,
                require_empty_if_exists: false,
            })
            .await
            .expect_err("blank route path should fail");

        assert_eq!(route_error.kind(), RepoOnboardingRouteErrorKind::BadRequest);
        assert_eq!(route_error.message(), "path is required");
    }

    #[tokio::test]
    async fn initialize_repo_can_be_inspected_as_repo() {
        let (_data_root, workspaces) = test_workspaces_handle().await;
        let temp = tempdir().expect("repo parent");
        let repo_path = temp.path().join("repo");

        let initialized = workspaces
            .initialize_repo(DaemonRepoInitRequest {
                path: repo_path.to_string_lossy().to_string(),
                allow_existing: false,
                allow_non_empty: false,
            })
            .await
            .expect("initialize repo");
        let status = workspaces
            .inspect_repo_status(initialized.to_str().expect("utf8 path"))
            .await
            .expect("repo status");

        assert!(status.is_repo);
        assert_eq!(status.error, None);
    }

    #[tokio::test]
    async fn inspect_repo_status_missing_path_returns_bad_request() {
        let (_data_root, workspaces) = test_workspaces_handle().await;
        let temp = tempdir().expect("repo parent");
        let missing = temp.path().join("missing");

        let error = workspaces
            .inspect_repo_status(missing.to_str().expect("utf8 path"))
            .await
            .expect_err("missing path should fail");

        assert_eq!(error.kind(), RepoOnboardingErrorKind::BadRequest);
        assert!(error.message().starts_with("invalid path '"));
    }
}
