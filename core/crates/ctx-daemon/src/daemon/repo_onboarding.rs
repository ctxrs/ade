use std::path::PathBuf;

use ctx_observability::logs;
use ctx_workspace_services::repo_onboarding as service;

use crate::daemon::WorkspacesHandle;

#[derive(Debug, Clone)]
pub struct DaemonRepoInitRequest {
    pub path: String,
    pub allow_existing: bool,
    pub allow_non_empty: bool,
}

#[derive(Debug, Clone)]
pub struct DaemonRepoCloneRequest {
    pub repo_url: String,
    pub dest_parent: String,
    pub branch: Option<String>,
    pub dest_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DaemonRepoValidateDestinationRequest {
    pub path: String,
    pub must_not_exist: bool,
    pub require_empty_if_exists: bool,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DaemonRepoStatusCheck {
    pub canonical_path: PathBuf,
    pub is_repo: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RepoOnboardingErrorKind {
    BadRequest,
    Internal,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RepoOnboardingError {
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

    pub fn kind(&self) -> RepoOnboardingErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
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
    pub async fn initialize_repo(
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

    pub async fn clone_repo(
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

    pub async fn validate_repo_destination(
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

    pub async fn create_repo_staging_path(&self) -> Result<PathBuf, RepoOnboardingError> {
        service::create_repo_staging_path(&self.state.core.data_root)
            .await
            .map_err(repo_staging_path_error)
    }

    pub async fn inspect_repo_status(
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
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn create_repo_staging_path_uses_daemon_data_root() {
        let (data_root, workspaces) = test_workspaces_handle().await;

        let staging = workspaces
            .create_repo_staging_path()
            .await
            .expect("staging path");

        assert!(staging.exists());
        assert!(staging.starts_with(data_root.path().join("workspaces").join("staging")));
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
