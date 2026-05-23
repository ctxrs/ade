mod active;
mod attachments;
mod common;
mod harness_container;
mod management_route_params;
mod registry;
mod worktrees;

#[cfg(test)]
mod tests;

pub(in crate::daemon::workspaces) use common::file_completions_route_error;
