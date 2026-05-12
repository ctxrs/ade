use std::path::PathBuf;

use super::destination::{
    prepare_clone_destination, prepare_repo_init_path, RepoCloneDestinationRequest,
    RepoInitPathRequest, RepoOnboardingPathError,
};
use super::git::{
    canonical_clone_dest, ensure_git_usable, init_git_repo_with_initial_commit, run_git_clone,
    RepoGitCommandError,
};
use super::status::{repo_status, RepoStatusCheck};

pub struct RepoInitRequest<'a> {
    pub path: &'a str,
    pub allow_existing: bool,
    pub allow_non_empty: bool,
}

pub struct RepoCloneRequest<'a> {
    pub repo_url: &'a str,
    pub dest_parent: &'a str,
    pub branch: Option<&'a str>,
    pub dest_name: Option<&'a str>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RepoOnboardingWorkflowError {
    GitPreflight(String),
    GitCommand(RepoGitCommandError),
    Path(RepoOnboardingPathError),
}

impl From<RepoGitCommandError> for RepoOnboardingWorkflowError {
    fn from(value: RepoGitCommandError) -> Self {
        Self::GitCommand(value)
    }
}

impl From<RepoOnboardingPathError> for RepoOnboardingWorkflowError {
    fn from(value: RepoOnboardingPathError) -> Self {
        Self::Path(value)
    }
}

pub async fn initialize_repo(
    req: RepoInitRequest<'_>,
) -> Result<PathBuf, RepoOnboardingWorkflowError> {
    ensure_git_usable()
        .await
        .map_err(RepoOnboardingWorkflowError::GitPreflight)?;

    let path = prepare_repo_init_path(RepoInitPathRequest {
        path: req.path,
        allow_existing: req.allow_existing,
        allow_non_empty: req.allow_non_empty,
    })
    .await?;

    // Worktrees require a base commit to diff against. `git init` alone yields a repo with no
    // commits, which breaks the out-of-the-box wizard path ("New repo").
    //
    // Use inline identity overrides so this workflow does not depend on global git config.
    init_git_repo_with_initial_commit(&path).await?;

    Ok(tokio::fs::canonicalize(&path).await.unwrap_or(path))
}

pub async fn clone_repo(req: RepoCloneRequest<'_>) -> Result<PathBuf, RepoOnboardingWorkflowError> {
    ensure_git_usable()
        .await
        .map_err(RepoOnboardingWorkflowError::GitPreflight)?;

    let repo_url = req.repo_url.trim();
    if repo_url.is_empty() {
        return Err(RepoOnboardingPathError::new("repo_url is required").into());
    }

    let dest = prepare_clone_destination(RepoCloneDestinationRequest {
        repo_url,
        dest_parent: req.dest_parent,
        dest_name: req.dest_name,
    })
    .await?;

    run_git_clone(
        repo_url,
        req.branch.map(str::trim).filter(|value| !value.is_empty()),
        &dest,
    )
    .await?;

    Ok(canonical_clone_dest(dest).await)
}

pub async fn inspect_repo_status(
    path: &str,
) -> Result<RepoStatusCheck, RepoOnboardingWorkflowError> {
    ensure_git_usable()
        .await
        .map_err(RepoOnboardingWorkflowError::GitPreflight)?;
    Ok(repo_status(path).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn initialize_repo_creates_repo_with_initial_commit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("new-repo");

        let initialized = initialize_repo(RepoInitRequest {
            path: path.to_str().expect("utf8 path"),
            allow_existing: false,
            allow_non_empty: false,
        })
        .await
        .expect("initialize repo");

        assert_eq!(initialized, path);
        let head = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&initialized)
            .arg("rev-parse")
            .arg("--verify")
            .arg("HEAD")
            .output()
            .await
            .expect("git rev-parse");
        assert!(
            head.status.success(),
            "expected initial commit, stderr: {}",
            String::from_utf8_lossy(&head.stderr)
        );
    }

    #[tokio::test]
    async fn clone_repo_rejects_empty_repo_url() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let error = clone_repo(RepoCloneRequest {
            repo_url: "   ",
            dest_parent: tmp.path().to_str().expect("utf8 path"),
            branch: None,
            dest_name: None,
        })
        .await
        .expect_err("empty repo url");

        assert_eq!(
            error,
            RepoOnboardingWorkflowError::Path(RepoOnboardingPathError::new("repo_url is required"))
        );
    }

    #[tokio::test]
    async fn inspect_repo_status_rejects_missing_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("missing");

        let error = inspect_repo_status(missing.to_str().expect("utf8 path"))
            .await
            .expect_err("missing path");

        match error {
            RepoOnboardingWorkflowError::Path(path_error) => {
                assert!(
                    path_error.message().starts_with("invalid path '"),
                    "unexpected error: {}",
                    path_error.message()
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
