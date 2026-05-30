use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use ctx_core::ids::{WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Workspace, WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, Worktree,
};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind};
use ctx_session_vcs_service::vcs::SessionVcsDiffBaseQuery;
use ctx_worktree_vcs_service::{WorktreeVcsCommitLookupSource, WorktreeVcsDiffBaseQuery};

use crate::daemon::sessions::title_generation::{
    TitleGenerationLocalHandle, TitleGenerationLocalInstallEffect,
};
use crate::daemon::sessions::{
    subagents::{
        SessionEventHeadSubscriber, SessionSubagentMcpControlFuture,
        SessionSubagentMcpControlHandle, SessionSubagentMcpControlHandleParts,
        SessionSubagentMcpControlLifecycleHost, SessionSubagentMcpControlPublicationHost,
        SessionSubagentMcpControlSchedulerSpawner, SubagentChildRunHost, SubagentSpawnHost,
        SubagentSpawnHostParts,
    },
    DemoSeedTranscriptHandle,
};

use super::{
    blobs::BlobHandle,
    git_status::{WorktreeVcsExecutionHost, WorktreeVcsRuntimeHost},
    launch_route_handles::{
        ExecutionLaunchHandle, LinuxSandboxRuntimeHandle, TerminalRouteHandle,
        WebSessionRouteHandle,
    },
    maintenance_route_handles::{DaemonShutdownHandle, UpdateActivityHandle, UpdateDrainHandle},
    merge_queue_route_handles::MergeQueueApiHandle,
    mobile_route_handles::{MobileRuntimeHandle, MobileSecureProxyHandle},
    provider_route_handles::ProviderStatusHandle,
    resource_utilization_route_handles::ResourceUtilizationHandle,
    route_capabilities::DaemonRouteHandles,
    route_handles::{
        AuthHandle, DiagnosticsHandle, DictationHandle, HealthHandle, LogsHandle,
        MobileStoreHandle, OrgPolicyHandle, RepoOnboardingHandle, RequestBaseHandle,
        TelemetryHandle, UpdateReleaseHandle,
    },
    run_archive_route_handles::RunArchiveHandle,
    session_control_effects::{SessionControlHandle, SessionControlHandleParts},
    session_route_handles::{
        SessionArtifactEffects, SessionArtifactsHandle, SessionFileCompletionsHandle,
        SessionFileCompletionsHandleParts, SessionMessageCommandHandle,
        SessionMessageSchedulerSpawner, SessionReadModelsHandle, SessionSubagentMcpReadFuture,
        SessionSubagentMcpReadHandle, SessionSubagentReadHandle, SessionTitleModelModeHandle,
        SessionTitleModelModeHandleParts, SessionVcsEffects, SessionVcsEffectsParts,
        SessionVcsFuture, SessionVcsHandle,
    },
    settings_route_handles::SettingsHandle,
    state::{
        ProtectedWorkspaceStoreLookup, SessionStoreLookup, TaskStoreLookup, WeakSessionStoreLookup,
    },
    task_route_handles::{
        TaskAdmissionFuture, TaskAdmissionModelCatalogLoader, TaskAdmissionSessionEffects,
        TaskArchivedRevLoader, TaskCloseWebSessionsForTask, TaskCreationHandle,
        TaskLifecycleEffects, TaskLifecycleHandle, TaskListingHandle, TaskMetadataEffects,
        TaskReadStateHandle, TaskSessionAdmissionHandle, TaskSessionListingHandle, TaskTitleHandle,
    },
    terminals::TerminalLaunchHost,
    web_sessions::{WebSessionLaunchHost, WebSessionWorkerRuntimeHost},
    workspace_route_handles::{
        WorkspaceAttachmentsHandle, WorkspaceDeletionHandle, WorkspaceExecutionConfigHandle,
        WorkspaceFileCompletionsHandle, WorkspaceHarnessContainerHandle,
        WorkspaceMergeQueueConfigHandle, WorkspaceOrgPolicyHandle, WorkspacePrimaryBranchHandle,
        WorkspacePrimaryBranchRefreshFuture, WorkspacePromptBootstrapConfigHandle,
        WorkspaceProviderModelPreferenceHandle, WorkspaceRegistryHandle, WorkspaceWorktreeHandle,
    },
    workspace_stream_route_handles::{
        WorkspaceActiveEffects, WorkspaceActiveEffectsParts, WorkspaceActiveFuture,
        WorkspaceActiveHandle, WorkspaceActiveHandleParts, WorkspaceStreamEffects,
        WorkspaceStreamEffectsParts, WorkspaceStreamFuture, WorkspaceStreamHandle,
        WorkspaceStreamHandleParts, WorkspaceStreamSessionLifecycleHost, WorkspaceVcsStreamHandle,
        WorkspaceVcsStreamRefreshFuture, WorkspaceVcsStreamWatcherFuture,
    },
    workspaces::{
        WorkspaceActiveCacheRuntime, WorkspaceActiveHydrationRuntime, WorkspaceDeletionRuntimeDeps,
    },
    DaemonShutdownHost, DaemonShutdownHostParts,
};

mod core;
mod core_deps;
mod execution;
mod execution_deps;
mod maintenance;
mod maintenance_deps;
mod provider_deps;
mod session_deps;
mod sessions;
mod state_deps;
mod task_deps;
mod tasks;
#[cfg(any(test, feature = "test-support"))]
mod test_helpers;
mod transport;
mod transport_deps;
mod workspace;
mod workspace_deps;

pub(crate) use state_deps::route_handles_from_state;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_helpers::*;
