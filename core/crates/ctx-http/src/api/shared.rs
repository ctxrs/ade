#[path = "shared/errors.rs"]
mod errors;
#[path = "shared/file_completions.rs"]
mod file_completions;
#[path = "shared/path_guard.rs"]
mod path_guard;
#[path = "shared/session_root.rs"]
mod session_root;
#[path = "shared/store_lookup.rs"]
mod store_lookup;

pub(crate) use errors::{
    map_effective_execution_settings_error, status_code_for_internal_error,
    status_code_for_request_or_policy_error,
};
pub(super) use file_completions::{map_file_completions_error, FileCompletionsQuery};
pub(super) use path_guard::path_resolves_within_root;
pub(super) use session_root::session_root_kind_for_worktree;
pub(super) use store_lookup::store_for_existing_workspace_status;
