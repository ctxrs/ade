use super::*;
use ctx_core::models::{SandboxGuestIdentity, SandboxSubstrate, VcsKind};
use ctx_workspace_services::worktree_vcs::{branch_exists, managed_worktree_path};

#[path = "lifecycle_tests/archive.rs"]
mod archive;
#[path = "lifecycle_tests/delete.rs"]
mod delete;
#[path = "lifecycle_tests/delete_cleanup.rs"]
mod delete_cleanup;
#[path = "lifecycle_tests/fixtures.rs"]
mod fixtures;
#[path = "lifecycle_tests/unarchive.rs"]
mod unarchive;
use fixtures::*;
