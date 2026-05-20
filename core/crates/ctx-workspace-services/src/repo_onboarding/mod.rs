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
    clone_repo, clone_repo_with_service_errors, create_repo_staging_path_with_service_errors,
    initialize_repo, initialize_repo_with_service_errors, inspect_repo_status,
    inspect_repo_status_with_service_errors, validate_repo_destination_with_service_errors,
    RepoCloneRequest, RepoInitRequest, RepoOnboardingServiceError, RepoOnboardingServiceErrorKind,
    RepoOnboardingWorkflowError,
};
