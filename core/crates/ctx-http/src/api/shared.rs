#[path = "shared/errors.rs"]
mod errors;
#[path = "shared/file_completions.rs"]
mod file_completions;
#[path = "shared/path_guard.rs"]
mod path_guard;

pub(crate) use errors::{
    map_effective_execution_settings_error, status_code_for_internal_error,
    status_code_for_request_or_policy_error,
};
pub(super) use file_completions::{map_file_completions_error, FileCompletionsQuery};
pub(super) use path_guard::path_resolves_within_root;
