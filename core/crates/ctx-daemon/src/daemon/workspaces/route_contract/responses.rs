pub use ctx_route_contracts::workspaces::{
    WorkspaceActiveHeadBatchRouteResponse, WorkspaceActiveSnapshotRouteResponse,
    WorkspaceAttachmentRouteResponse, WorkspaceRouteResponse, WorktreeRouteResponse,
};
use ctx_sandbox_contract::{ContainerMountMode, ContainerNetworkMode};
use ctx_workspace_container::WorkspaceContainerStatus;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceHarnessContainerStatusRouteResponse {
    pub name: String,
    pub running: bool,
    pub known: bool,
    pub mount_mode: Option<ContainerMountMode>,
    pub network_mode: Option<ContainerNetworkMode>,
    pub allowlist: Vec<String>,
    pub egress_guard: Option<bool>,
}

impl From<WorkspaceContainerStatus> for WorkspaceHarnessContainerStatusRouteResponse {
    fn from(status: WorkspaceContainerStatus) -> Self {
        Self {
            name: status.name,
            running: status.running,
            known: status.known,
            mount_mode: status.mount_mode,
            network_mode: status.network_mode,
            allowlist: status.allowlist,
            egress_guard: status.egress_guard,
        }
    }
}
