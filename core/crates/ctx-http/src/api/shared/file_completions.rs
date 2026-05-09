use serde::Deserialize;

#[path = "file_completions/container.rs"]
mod container;
#[path = "file_completions/listing.rs"]
mod listing;

pub(crate) use listing::{load_and_cache_workspace_files, load_and_cache_worktree_files};

#[derive(Debug, Deserialize, Default)]
pub(crate) struct FileCompletionsQuery {
    pub(crate) query: Option<String>,
    pub(crate) limit: Option<u32>,
}
