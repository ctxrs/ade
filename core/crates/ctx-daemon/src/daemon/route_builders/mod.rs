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
        ExecutionLaunchHandle, LinuxSandboxRuntimeHandle, ProviderWorkspaceLaunchRuntime,
        TerminalRouteHandle, WebSessionRouteHandle,
    },
    maintenance_route_handles::{DaemonShutdownHandle, UpdateActivityHandle, UpdateDrainHandle},
    merge_queue_route_handles::{
        MergeQueueApiHandle, MergeQueueNoticePublicationEffect, MergeQueueNoticePublicationFuture,
        MergeQueueNoticeSessionEvent,
    },
    mobile_route_handles::{MobileRuntimeHandle, MobileSecureProxyHandle},
    provider_route_handles::{
        ProviderAccountsHandle, ProviderAdminHandle, ProviderAuthImportHandle,
        ProviderBootstrapHandle, ProviderHarnessConfigHandle, ProviderInstallHandle,
        ProviderOptionsHandle, ProviderStatusHandle, ProviderUsageHandle,
        ProviderWorkspaceAuthHandle,
    },
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
        DaemonState, ProtectedWorkspaceStoreLookup, SessionStoreLookup, TaskStoreLookup,
        WeakSessionStoreLookup,
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
        TaskWorktreeHost, TaskWorktreeHostParts, WorkspaceActiveCacheRuntime,
        WorkspaceActiveHydrationRuntime, WorkspaceDeletionRuntimeDeps,
    },
    DaemonShutdownHost, DaemonShutdownHostParts,
};

mod core;
mod execution;
mod maintenance;
mod providers;
mod sessions;
mod tasks;
mod transport;
mod workspace;

#[cfg(test)]
use super::workspace_route_handles::WorkspacePrimaryBranchRefreshEffect;
#[cfg(test)]
use super::workspace_stream_route_handles::WorkspaceVcsStreamRefreshEffect;

#[derive(Clone)]
pub(crate) struct RouteBuilder {
    state: Arc<DaemonState>,
}

pub(crate) fn route_handles_from_state(state: &Arc<DaemonState>) -> DaemonRouteHandles {
    let handle = RouteBuilder::new(Arc::clone(state));
    DaemonRouteHandles {
        auth: handle.auth(),
        health: handle.health(),
        diagnostics: handle.diagnostics(),
        blob: handle.blob(),
        request_base: handle.request_base(),
        repo_onboarding: handle.repo_onboarding(),
        logs: handle.logs(),
        org_policy: handle.org_policy(),
        workspace_org_policy: handle.workspace_org_policy(),
        workspace_prompt_bootstrap_config: handle.workspace_prompt_bootstrap_config(),
        workspace_execution_config: handle.workspace_execution_config(),
        workspace_file_completions: handle.workspace_file_completions(),
        workspace_harness_container: handle.workspace_harness_container(),
        workspace_provider_model_preferences: handle.workspace_provider_model_preferences(),
        workspace_worktree: handle.workspace_worktree(),
        workspace_registry: handle.workspace_registry(),
        workspace_merge_queue_config: handle.workspace_merge_queue_config(),
        merge_queue_api: handle.merge_queue_api(),
        workspace_attachments: handle.workspace_attachments(),
        workspace_primary_branch: handle.workspace_primary_branch(),
        dictation: handle.dictation(),
        update_release: handle.update_release(),
        update_activity: handle.update_activity(),
        settings: handle.settings(),
        mobile_store: handle.mobile_store(),
        mobile_runtime: handle.mobile_runtime(),
        mobile_secure_proxy: handle.mobile_secure_proxy(),
        resource_utilization: handle.resource_utilization(),
        run_archive: handle.run_archive(),
        session_artifacts: handle.session_artifacts(),
        session_control: handle.session_control(),
        session_file_completions: handle.session_file_completions(),
        session_message_command: handle.session_message_command(),
        session_read_models: handle.session_read_models(),
        session_subagent_mcp_read: handle.session_subagent_mcp_read(),
        session_subagent_mcp_control: handle.session_subagent_mcp_control(),
        session_subagent_read: handle.session_subagent_read(),
        session_title_model_mode: handle.session_title_model_mode(),
        session_vcs: handle.session_vcs(),
        demo_seed_transcript: handle.demo_seed_transcript(),
        title_generation_local: handle.title_generation_local(),
        task_creation: handle.task_creation(),
        task_lifecycle: handle.task_lifecycle(),
        task_listing: handle.task_listing(),
        task_read_state: handle.task_read_state(),
        task_session_admission: handle.task_session_admission(),
        task_session_listing: handle.task_session_listing(),
        task_title: handle.task_title(),
        workspace_deletion: handle.workspace_deletion(),
        workspace_active: handle.workspace_active(),
        workspace_stream: handle.workspace_stream(),
        workspace_vcs_stream: handle.workspace_vcs_stream(),
        provider_accounts: handle.provider_accounts(),
        provider_auth_import: handle.provider_auth_import(),
        provider_status: handle.provider_status(),
        provider_admin: handle.provider_admin(),
        provider_install: handle.provider_install(),
        provider_usage: handle.provider_usage(),
        provider_harness_config: handle.provider_harness_config(),
        provider_bootstrap: handle.provider_bootstrap(),
        provider_options: handle.provider_options(),
        provider_workspace_auth: handle.provider_workspace_auth(),
        telemetry: handle.telemetry(),
        terminal_route: handle.terminal_route(),
        web_session_route: handle.web_session_route(),
        execution_launch: handle.execution_launch(),
        linux_sandbox_runtime: handle.linux_sandbox_runtime(),
        update_drain: handle.update_drain(),
        daemon_shutdown: handle.daemon_shutdown(),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn workspace_attachments_runtime_from_state(
    state: &Arc<DaemonState>,
) -> Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime> {
    RouteBuilder::new(Arc::clone(state)).workspace_attachments_runtime()
}

#[cfg(test)]
pub(crate) fn workspace_primary_branch_with_refresh_effect_from_state(
    state: &Arc<DaemonState>,
    refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
) -> WorkspacePrimaryBranchHandle {
    RouteBuilder::new(Arc::clone(state))
        .workspace_primary_branch_with_refresh_effect(refresh_vcs_snapshot)
}

#[cfg(test)]
pub(crate) fn workspace_vcs_stream_with_refresh_effect_from_state(
    state: &Arc<DaemonState>,
    refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
) -> WorkspaceVcsStreamHandle {
    RouteBuilder::new(Arc::clone(state))
        .workspace_vcs_stream_with_refresh_effect(refresh_worktree_vcs)
}

impl RouteBuilder {
    pub(crate) fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    fn protected_workspace_store_lookup(&self) -> ProtectedWorkspaceStoreLookup {
        ProtectedWorkspaceStoreLookup::new(
            self.state.core.stores.clone(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }
}
