use std::path::PathBuf;

use ctx_workspace_services::repo_onboarding as service;
use serde::{Deserialize, Serialize};

use crate::daemon::WorkspacesHandle;

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

impl From<service::RepoOnboardingServiceError> for RepoOnboardingRouteError {
    fn from(error: service::RepoOnboardingServiceError) -> Self {
        let kind = match error.kind() {
            service::RepoOnboardingServiceErrorKind::BadRequest => {
                RepoOnboardingRouteErrorKind::BadRequest
            }
            service::RepoOnboardingServiceErrorKind::Internal => {
                RepoOnboardingRouteErrorKind::Internal
            }
        };
        Self::new(kind, error.message())
    }
}

fn repo_path_route_response(path: PathBuf) -> RepoPathRouteResponse {
    RepoPathRouteResponse {
        path: path.to_string_lossy().to_string(),
    }
}

fn repo_status_route_response(status: service::RepoStatusCheck) -> RepoStatusRouteResponse {
    RepoStatusRouteResponse {
        canonical_path: status.canonical_path.to_string_lossy().to_string(),
        is_repo: status.is_repo,
        error: status.error,
    }
}

impl WorkspacesHandle {
    pub async fn initialize_repo_for_route(
        &self,
        req: RepoInitRouteRequest,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        service::initialize_repo_with_service_errors(service::RepoInitRequest {
            path: &req.path,
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
        service::clone_repo_with_service_errors(service::RepoCloneRequest {
            repo_url: &req.repo_url,
            dest_parent: &req.dest_parent,
            branch: req.branch.as_deref(),
            dest_name: req.dest_name.as_deref(),
        })
        .await
        .map(repo_path_route_response)
        .map_err(Into::into)
    }

    pub async fn validate_repo_destination_for_route(
        &self,
        req: RepoValidateDestinationRouteRequest,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        service::validate_repo_destination_with_service_errors(
            service::RepoValidateDestinationRequest {
                path: &req.path,
                must_not_exist: req.must_not_exist,
                require_empty_if_exists: req.require_empty_if_exists,
            },
        )
        .await
        .map(repo_path_route_response)
        .map_err(Into::into)
    }

    pub async fn create_repo_staging_path_for_route(
        &self,
    ) -> Result<RepoPathRouteResponse, RepoOnboardingRouteError> {
        service::create_repo_staging_path_with_service_errors(&self.state.core.data_root)
            .await
            .map(repo_path_route_response)
            .map_err(Into::into)
    }

    pub async fn inspect_repo_status_for_route(
        &self,
        req: RepoStatusRouteRequest,
    ) -> Result<RepoStatusRouteResponse, RepoOnboardingRouteError> {
        service::inspect_repo_status_with_service_errors(&req.path)
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

        let status_without_error = repo_status_route_response(service::RepoStatusCheck {
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

        let status_with_error = repo_status_route_response(service::RepoStatusCheck {
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

    #[tokio::test]
    async fn create_repo_staging_path_uses_daemon_data_root() {
        let (data_root, workspaces) = test_workspaces_handle().await;

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

        let response = workspaces
            .initialize_repo_for_route(RepoInitRouteRequest {
                path: repo_path.to_string_lossy().to_string(),
                allow_existing: false,
                allow_non_empty: false,
            })
            .await
            .expect("initialize repo");
        let value = serde_json::to_value(response).expect("route response json");
        let initialized = value
            .get("path")
            .and_then(|path| path.as_str())
            .expect("path field");
        let status = workspaces
            .inspect_repo_status_for_route(RepoStatusRouteRequest {
                path: initialized.to_string(),
            })
            .await
            .expect("repo status");
        let status = serde_json::to_value(status).expect("status json");

        assert_eq!(
            status.get("is_repo").and_then(|value| value.as_bool()),
            Some(true)
        );
        assert_eq!(status.get("error"), None);
    }

    #[tokio::test]
    async fn inspect_repo_status_missing_path_returns_bad_request() {
        let (_data_root, workspaces) = test_workspaces_handle().await;
        let temp = tempdir().expect("repo parent");
        let missing = temp.path().join("missing");

        let error = workspaces
            .inspect_repo_status_for_route(RepoStatusRouteRequest {
                path: missing.to_string_lossy().to_string(),
            })
            .await
            .expect_err("missing path should fail");

        assert_eq!(error.kind(), RepoOnboardingRouteErrorKind::BadRequest);
        assert!(error.message().starts_with("invalid path '"));
    }
}
