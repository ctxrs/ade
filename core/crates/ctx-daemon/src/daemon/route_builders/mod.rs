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
mod provider_deps;
mod session_deps;
mod sessions;
mod tasks;
#[cfg(any(test, feature = "test-support"))]
mod test_helpers;
mod transport;
mod workspace;

#[cfg(any(test, feature = "test-support"))]
pub(crate) use test_helpers::*;

#[derive(Clone)]
pub(crate) struct RouteBuilder {
    state: Arc<DaemonState>,
}

pub(crate) fn route_handles_from_state(state: &Arc<DaemonState>) -> DaemonRouteHandles {
    let handle = RouteBuilder::new(Arc::clone(state));
    let provider_routes = handle.provider_route_deps();
    let session_routes = handle.session_route_deps();
    let session_title_model_mode = session_routes.session_title_model_mode();
    let task_session_admission = handle.task_session_admission_with_route_deps(
        &provider_routes,
        &session_routes,
        session_title_model_mode.clone(),
    );
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
        workspace_provider_model_preferences: handle
            .workspace_provider_model_preferences_with_provider_routes(&provider_routes),
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
        session_artifacts: session_routes.session_artifacts(),
        session_control: session_routes.session_control_with_provider_routes(&provider_routes),
        session_file_completions: session_routes.session_file_completions(),
        session_message_command: session_routes.session_message_command(),
        session_read_models: session_routes.session_read_models(),
        session_subagent_mcp_read: session_routes.session_subagent_mcp_read(),
        session_subagent_mcp_control: session_routes
            .session_subagent_mcp_control_with_provider_routes(&provider_routes),
        session_subagent_read: session_routes.session_subagent_read(),
        session_title_model_mode,
        session_vcs: session_routes.session_vcs(),
        demo_seed_transcript: session_routes.demo_seed_transcript(),
        title_generation_local: session_routes.title_generation_local(),
        task_creation: handle
            .task_creation_with_session_admission(task_session_admission.clone(), &session_routes),
        task_lifecycle: handle.task_lifecycle_with_session_routes(&session_routes),
        task_listing: handle.task_listing(),
        task_read_state: handle.task_read_state_with_session_routes(&session_routes),
        task_session_admission,
        task_session_listing: handle.task_session_listing(),
        task_title: handle.task_title_with_session_routes(&session_routes),
        workspace_deletion: handle.workspace_deletion(),
        workspace_active: handle.workspace_active(),
        workspace_stream: handle.workspace_stream(),
        workspace_vcs_stream: handle.workspace_vcs_stream(),
        provider_accounts: provider_routes.provider_accounts(),
        provider_auth_import: provider_routes.provider_auth_import(),
        provider_status: provider_routes.provider_status(),
        provider_admin: provider_routes.provider_admin(),
        provider_install: provider_routes.provider_install(),
        provider_usage: provider_routes.provider_usage(),
        provider_harness_config: provider_routes.provider_harness_config(),
        provider_bootstrap: provider_routes.provider_bootstrap(),
        provider_options: provider_routes.provider_options(),
        provider_workspace_auth: provider_routes.provider_workspace_auth(),
        telemetry: handle.telemetry(),
        terminal_route: handle.terminal_route(),
        web_session_route: handle.web_session_route(),
        execution_launch: handle.execution_launch(),
        linux_sandbox_runtime: handle.linux_sandbox_runtime(),
        update_drain: handle.update_drain(),
        daemon_shutdown: handle.daemon_shutdown_with_session_routes(&session_routes),
    }
}

impl RouteBuilder {
    pub(crate) fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    fn provider_route_deps(&self) -> provider_deps::ProviderRouteDeps {
        provider_deps::ProviderRouteDeps::new(provider_deps::ProviderRouteDepsParts {
            data_root: self.state.core.data_root.clone(),
            daemon_url: self.state.core.daemon_url.clone(),
            auth_token: self.state.core.auth_token.clone(),
            workspace_stores: self.protected_workspace_store_lookup(),
            providers: Arc::clone(&self.state.providers),
            ops_events: self.state.telemetry.ops_events.clone(),
            shutdown_tx: self.state.core.shutdown_tx.clone(),
            harness: Arc::clone(&self.state.execution.harness),
        })
    }

    fn protected_workspace_store_lookup(&self) -> ProtectedWorkspaceStoreLookup {
        ProtectedWorkspaceStoreLookup::new(
            self.state.core.stores.clone(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }

    fn session_route_deps(&self) -> session_deps::SessionRouteDeps {
        let workspace_stores = self.protected_workspace_store_lookup();
        let session_stores =
            SessionStoreLookup::new(self.state.global_store().clone(), workspace_stores.clone());
        let weak_session_stores = WeakSessionStoreLookup::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::downgrade(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        );
        session_deps::SessionRouteDeps::new(session_deps::SessionRouteDepsParts {
            data_root: self.state.core.data_root.clone(),
            tool_output_spool_dir: self.state.core.tool_output_spool_dir.clone(),
            daemon_url: self.state.core.daemon_url.clone(),
            auth_token: self.state.core.auth_token.clone(),
            global_store: self.state.global_store().clone(),
            stores: self.state.core.stores.clone(),
            workspace_stores,
            session_stores,
            weak_session_stores,
            sessions: Arc::clone(&self.state.sessions),
            scheduler_worker_host: self.state.session_scheduler_worker_host.worker_host(),
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            worktree_file_completions_cache: Arc::clone(
                &self.state.workspaces.file_completions_cache,
            ),
            providers: Arc::clone(&self.state.providers),
            ops_events: self.state.telemetry.ops_events.clone(),
            perf_telemetry: self.state.telemetry.perf_telemetry.clone(),
            provider_unknown_events: self.state.telemetry.provider_unknown_events.clone(),
            ask_user_question: Arc::clone(&self.state.core.ask_user_question),
            update_drain: Arc::clone(&self.state.core.update_drain),
            harness: Arc::clone(&self.state.execution.harness),
            task_publication: Arc::clone(&self.state.task_publication),
            task_worktree_host: self.task_worktree_host(),
            worktree_vcs_runtime: self.worktree_vcs_runtime_host(),
            worktree_vcs_execution: self.worktree_vcs_execution_host(),
        })
    }
}
