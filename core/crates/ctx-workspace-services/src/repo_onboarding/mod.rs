mod destination;
mod git;
mod path_policy;
mod staging;
mod status;
mod workflow;

pub use destination::{
    validate_repo_destination, RepoOnboardingPathError, RepoValidateDestinationRequest,
};
pub use git::RepoGitCommandError;
pub use path_policy::{derive_repo_name, expand_tilde, validate_absolute_path, validate_dest_name};
pub use staging::create_repo_staging_path;
pub use status::RepoStatusCheck;
pub use workflow::{
    clone_repo, initialize_repo, inspect_repo_status, RepoCloneRequest, RepoInitRequest,
    RepoOnboardingWorkflowError,
};
