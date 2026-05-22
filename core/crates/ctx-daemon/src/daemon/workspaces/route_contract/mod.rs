mod active;
mod attachments;
mod common;
mod harness_container;
mod management_route_params;
mod registry;
mod responses;
mod worktrees;

pub use attachments::{
    CreateWorkspaceAttachmentRouteRequest, DeleteWorkspaceAttachmentRouteRequest,
    SyncWorkspaceAttachmentsRouteRequest,
};
pub use common::{WorkspaceRouteParams, WorktreeRouteParams};
pub use responses::{
    WorkspaceActiveHeadBatchRouteResponse, WorkspaceActiveSnapshotRouteResponse,
    WorkspaceAttachmentRouteResponse, WorkspaceRouteResponse, WorktreeRouteResponse,
};
pub use worktrees::WorkspaceFileCompletionsRouteQuery;

#[cfg(test)]
mod tests;
