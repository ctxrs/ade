use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context;
use ctx_core::ids::{SessionId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, SessionEvent, SessionEventType, TerminalSession, Workspace,
    WorkspaceActiveHeadBatch, WorkspaceActiveSnapshot, Worktree,
};
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_observability::ops_events::OpsEvents;
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use ctx_observability::telemetry::Telemetry;
use ctx_provider_install::install_state::InstallTarget;
use ctx_provider_runtime::ProviderRuntime;
use ctx_resource_utilization::resource_governance::ResourceGovernanceRuntime;
use ctx_resource_utilization::ResourceSampler;
use ctx_session_runtime::runtime::{SessionLifecycleHost, SessionRuntime};
use ctx_session_vcs_service::vcs::SessionVcsDiffBaseQuery;
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use ctx_transport_runtime::terminal_launch::TerminalLaunchError;
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_transport_runtime::web_sessions::{WebSessionInfo, WebSessionManager};
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;
use ctx_worktree_vcs_service::{WorktreeVcsCommitLookupSource, WorktreeVcsDiffBaseQuery};
use tokio::sync::{broadcast, Mutex};

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
    provider_route_handles::{
        ProviderAccountsHandle, ProviderAdminHandle, ProviderAuthImportHandle,
        ProviderBootstrapHandle, ProviderHarnessConfigHandle, ProviderInstallHandle,
        ProviderOptionsHandle, ProviderStatusHandle, ProviderUsageHandle,
        ProviderWorkspaceAuthHandle,
    },
    route_capabilities::{DaemonRouteHandles, DaemonShutdownSignal},
    route_handles::{
        AuthHandle, DiagnosticsHandle, DictationHandle, HealthHandle, LogsHandle,
        MobileStoreHandle, OrgPolicyHandle, RepoOnboardingHandle, RequestBaseHandle,
        TelemetryHandle, UpdateReleaseHandle,
    },
    session_control_effects::{SessionControlHandle, SessionControlHandleParts},
    session_route_handles::{
        SessionArtifactEffects, SessionArtifactsHandle, SessionFileCompletionsHandle,
        SessionFileCompletionsHandleParts, SessionMessageCommandHandle,
        SessionMessageSchedulerSpawner, SessionReadModelsHandle, SessionSubagentMcpReadFuture,
        SessionSubagentMcpReadHandle, SessionSubagentReadHandle, SessionTitleModelModeHandle,
        SessionTitleModelModeHandleParts, SessionVcsEffects, SessionVcsEffectsParts,
        SessionVcsFuture, SessionVcsHandle,
    },
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
    terminals::{CreateTerminalLaunchRequest, TerminalLaunchHost},
    web_sessions::{
        WebSessionLaunchError, WebSessionLaunchHost, WebSessionLaunchRequest,
        WebSessionWorkerRuntimeHost,
    },
    workspace_route_handles::{
        WorkspaceAttachmentsHandle, WorkspaceDeletionHandle, WorkspaceExecutionConfigHandle,
        WorkspaceFileCompletionsHandle, WorkspaceHarnessContainerHandle,
        WorkspaceMergeQueueConfigHandle, WorkspaceOrgPolicyHandle, WorkspacePrimaryBranchHandle,
        WorkspacePrimaryBranchRefreshFuture, WorkspacePromptBootstrapConfigHandle,
        WorkspaceProviderModelPreferenceHandle, WorkspaceRegistryHandle, WorkspaceWorktreeHandle,
    },
    workspaces::{
        TaskWorktreeHost, TaskWorktreeHostParts, WorkspaceActiveCacheRuntime,
        WorkspaceActiveHydrationRuntime, WorkspaceDeletionRuntimeDeps,
    },
    DaemonShutdownHost, DaemonShutdownHostParts,
};

#[cfg(test)]
use super::workspace_route_handles::WorkspacePrimaryBranchRefreshEffect;

#[derive(Clone)]
pub struct DaemonHandle {
    state: Arc<DaemonState>,
}

pub(crate) fn route_handles_from_state(state: &Arc<DaemonState>) -> DaemonRouteHandles {
    let handle = DaemonHandle::new(Arc::clone(state));
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

impl DaemonHandle {
    pub fn new(state: Arc<DaemonState>) -> Self {
        Self { state }
    }

    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.state.core.shutdown_tx.subscribe()
    }

    pub fn shutdown_signal(&self) -> DaemonShutdownSignal {
        DaemonShutdownSignal::new(self.state.core.shutdown_tx.clone())
    }

    pub fn auth(&self) -> AuthHandle {
        AuthHandle::new(
            self.state.core.auth_token.clone(),
            Arc::clone(&self.state.core.mcp_auth),
            self.state.global_store().clone(),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn health(&self) -> HealthHandle {
        HealthHandle::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.clone(),
            Arc::clone(&self.state.core.storage_guard),
        )
    }

    pub fn diagnostics(&self) -> DiagnosticsHandle {
        DiagnosticsHandle::new(
            self.health(),
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.execution.setup),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn blob(&self) -> BlobHandle {
        BlobHandle::new(
            self.state.core.data_root.clone(),
            self.state.global_store().clone(),
        )
    }

    pub fn request_base(&self) -> RequestBaseHandle {
        RequestBaseHandle::new(
            self.state.core.daemon_url.clone(),
            self.state.core.public_base_url.clone(),
        )
    }

    pub fn repo_onboarding(&self) -> RepoOnboardingHandle {
        RepoOnboardingHandle::new(self.state.core.data_root.clone())
    }

    pub fn run_archive(&self) -> RunArchiveHandle {
        RunArchiveHandle::new(self.protected_workspace_store_lookup())
    }

    pub fn logs(&self) -> LogsHandle {
        LogsHandle::new(self.state.core.data_root.clone())
    }

    pub fn org_policy(&self) -> OrgPolicyHandle {
        OrgPolicyHandle::new(self.state.global_store().clone())
    }

    pub fn workspace_registry(&self) -> WorkspaceRegistryHandle {
        WorkspaceRegistryHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.telemetry.telemetry.clone(),
        )
    }

    pub fn workspace_deletion(&self) -> WorkspaceDeletionHandle {
        WorkspaceDeletionHandle::new(Arc::new(
            crate::daemon::workspaces::WorkspaceDeletionRuntime::new(
                WorkspaceDeletionRuntimeDeps {
                    data_root: self.state.core.data_root.clone(),
                    daemon_url: self.state.core.daemon_url.clone(),
                    stores: self.state.core.stores.clone(),
                    global_store: self.state.global_store().clone(),
                    sessions: Arc::clone(&self.state.sessions),
                    active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
                    workspace_active_snapshot_cache: Arc::clone(
                        &self.state.workspaces.workspace_active_snapshot_cache,
                    ),
                    workspace_active_heads_cache: Arc::clone(
                        &self.state.workspaces.workspace_active_heads_cache,
                    ),
                    workspace_file_completions_cache: Arc::clone(
                        &self.state.workspaces.workspace_file_completions_cache,
                    ),
                    harness: Arc::clone(&self.state.execution.harness),
                    providers: Arc::clone(&self.state.providers),
                    merge_queue: Arc::clone(&self.state.transport.merge_queue),
                },
            ),
        ))
    }

    pub fn workspace_merge_queue_config(&self) -> WorkspaceMergeQueueConfigHandle {
        WorkspaceMergeQueueConfigHandle::new(
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }

    pub fn merge_queue_api(&self) -> MergeQueueApiHandle {
        MergeQueueApiHandle::new(self.merge_queue_route_host())
    }

    fn merge_queue_route_host(&self) -> Arc<crate::daemon::merge_queue::MergeQueueRouteHost> {
        let workspace_stores = self.protected_workspace_store_lookup();
        let session_stores =
            SessionStoreLookup::new(self.state.global_store().clone(), workspace_stores.clone());
        let publisher = self.session_publication_effects();
        let publish_merge_queue_notice =
            Arc::new(move |notice_event: MergeQueueNoticeSessionEvent| {
                let publisher = publisher.clone();
                Box::pin(async move { publisher.publish_merge_queue_notice(notice_event).await })
                    as MergeQueueNoticePublicationFuture
            });
        Arc::new(crate::daemon::merge_queue::MergeQueueRouteHost::new(
            self.state.core.stores.clone(),
            self.state.global_store().clone(),
            workspace_stores,
            session_stores,
            Arc::clone(&self.state.transport.merge_queue),
            self.state.telemetry.ops_events.clone(),
            publish_merge_queue_notice,
        ))
    }

    fn worktree_vcs_execution_host(&self) -> WorktreeVcsExecutionHost {
        WorktreeVcsExecutionHost::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.execution.harness),
        )
    }

    fn worktree_vcs_runtime_host(&self) -> WorktreeVcsRuntimeHost {
        WorktreeVcsRuntimeHost::from_workspace_runtime(&self.state.workspaces)
    }

    pub fn workspace_attachments(&self) -> WorkspaceAttachmentsHandle {
        let workspace_stores = self.protected_workspace_store_lookup();
        WorkspaceAttachmentsHandle::new(
            self.state.global_store().clone(),
            workspace_stores,
            self.workspace_attachments_runtime(),
        )
    }

    pub fn workspace_primary_branch(&self) -> WorkspacePrimaryBranchHandle {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let refresh_vcs_snapshot = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    crate::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
                        &vcs_runtime,
                        &vcs_execution,
                        &worktree,
                        true,
                    )
                    .await
                }) as WorkspacePrimaryBranchRefreshFuture
            }
        });
        WorkspacePrimaryBranchHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            refresh_vcs_snapshot,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon) fn workspace_primary_branch_with_refresh_effect(
        &self,
        refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
    ) -> WorkspacePrimaryBranchHandle {
        WorkspacePrimaryBranchHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            refresh_vcs_snapshot,
        )
    }

    pub fn workspace_org_policy(&self) -> WorkspaceOrgPolicyHandle {
        WorkspaceOrgPolicyHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
        )
    }

    pub fn workspace_prompt_bootstrap_config(&self) -> WorkspacePromptBootstrapConfigHandle {
        WorkspacePromptBootstrapConfigHandle::new(self.protected_workspace_store_lookup())
    }

    pub fn workspace_file_completions(&self) -> WorkspaceFileCompletionsHandle {
        WorkspaceFileCompletionsHandle::new(
            self.state.global_store().clone(),
            Arc::clone(&self.state.workspaces.workspace_file_completions_cache),
            self.state.telemetry.perf_telemetry.clone(),
        )
    }

    pub fn workspace_execution_config(&self) -> WorkspaceExecutionConfigHandle {
        WorkspaceExecutionConfigHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
        )
    }

    pub fn workspace_harness_container(&self) -> WorkspaceHarnessContainerHandle {
        WorkspaceHarnessContainerHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.daemon_url.clone(),
            Arc::clone(&self.state.execution.harness),
        )
    }

    pub fn workspace_worktree(&self) -> WorkspaceWorktreeHandle {
        WorkspaceWorktreeHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
        )
    }

    pub fn workspace_provider_model_preferences(&self) -> WorkspaceProviderModelPreferenceHandle {
        WorkspaceProviderModelPreferenceHandle::new(self.provider_workspace_launch_runtime())
    }

    pub fn dictation(&self) -> DictationHandle {
        DictationHandle::new(self.state.global_store().clone())
    }

    pub fn update_release(&self) -> UpdateReleaseHandle {
        UpdateReleaseHandle::new(self.state.core.data_root.clone())
    }

    pub fn update_activity(&self) -> UpdateActivityHandle {
        UpdateActivityHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            self.state.core.data_root.clone(),
        )
    }

    pub fn settings(&self) -> SettingsHandle {
        SettingsHandle::new(
            self.state.global_store().clone(),
            self.state.telemetry.telemetry.clone(),
            self.state.telemetry.perf_telemetry.clone(),
            Arc::clone(&self.state.telemetry.resource_sampler),
            Arc::clone(&self.state.telemetry.resource_governance),
            Arc::clone(&self.state.providers),
            Arc::clone(&self.state.transport.terminals),
        )
    }

    pub fn mobile_store(&self) -> MobileStoreHandle {
        MobileStoreHandle::new(self.state.global_store().clone())
    }

    pub fn mobile_runtime(&self) -> MobileRuntimeHandle {
        MobileRuntimeHandle::new(
            self.state.global_store().clone(),
            self.state.transport.mobile_tunnel.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.is_some(),
        )
    }

    pub fn mobile_secure_proxy(&self) -> MobileSecureProxyHandle {
        MobileSecureProxyHandle::new(
            self.state.global_store().clone(),
            self.health(),
            self.state.telemetry.telemetry.clone(),
        )
    }

    pub fn resource_utilization(&self) -> ResourceUtilizationHandle {
        ResourceUtilizationHandle::new(
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            Arc::clone(&self.state.telemetry.resource_sampler),
        )
    }

    pub fn title_generation_local(&self) -> TitleGenerationLocalHandle {
        TitleGenerationLocalHandle::new(
            self.state.core.data_root.clone(),
            TitleGenerationLocalInstallEffect::new(
                self.state.core.data_root.clone(),
                Arc::clone(&self.state.providers),
                self.state.telemetry.ops_events.clone(),
            ),
        )
    }

    pub fn demo_seed_transcript(&self) -> DemoSeedTranscriptHandle {
        DemoSeedTranscriptHandle::new(
            self.session_store_lookup(),
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        )
    }

    pub fn session_control(&self) -> SessionControlHandle {
        SessionControlHandle::new(SessionControlHandleParts {
            session_stores: self.session_store_lookup(),
            session_runtime: Arc::clone(&self.state.sessions),
            scheduler_spawner: SessionMessageSchedulerSpawner::new(Arc::downgrade(
                &self.state.session_scheduler_worker_host(),
            )),
            perf_telemetry: self.state.telemetry.perf_telemetry.clone(),
            provider_launch: self.provider_workspace_launch_runtime(),
            session_publication: self.session_publication_effects(),
            ask_user_question: Arc::clone(&self.state.core.ask_user_question),
            provider_unknown_events: self.state.telemetry.provider_unknown_events.clone(),
        })
    }

    pub fn session_file_completions(&self) -> SessionFileCompletionsHandle {
        SessionFileCompletionsHandle::new(SessionFileCompletionsHandleParts {
            global_store: self.state.global_store().clone(),
            session_stores: self.session_store_lookup(),
            workspace_stores: self.protected_workspace_store_lookup(),
            worktree_file_completions_cache: Arc::clone(
                &self.state.workspaces.file_completions_cache,
            ),
            perf_telemetry: self.state.telemetry.perf_telemetry.clone(),
            data_root: self.state.core.data_root.clone(),
            daemon_url: self.state.core.daemon_url.clone(),
            harness: Arc::clone(&self.state.execution.harness),
        })
    }

    pub fn session_title_model_mode(&self) -> SessionTitleModelModeHandle {
        SessionTitleModelModeHandle::new(SessionTitleModelModeHandleParts {
            global_store: self.state.global_store().clone(),
            session_stores: self.session_store_lookup(),
            workspace_stores: self.protected_workspace_store_lookup(),
            session_runtime: Arc::clone(&self.state.sessions),
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            provider_runtime: Arc::clone(&self.state.providers),
            ops_events: self.state.telemetry.ops_events.clone(),
            data_root: self.state.core.data_root.clone(),
            daemon_url: self.state.core.daemon_url.clone(),
            auth_token: self.state.core.auth_token.clone(),
            harness: Arc::clone(&self.state.execution.harness),
        })
    }

    pub fn session_message_command(&self) -> SessionMessageCommandHandle {
        SessionMessageCommandHandle::new(
            self.state.global_store().clone(),
            self.session_store_lookup(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.core.update_drain),
            self.state.core.data_root.clone(),
            self.session_title_model_mode(),
            SessionMessageSchedulerSpawner::new(Arc::downgrade(
                &self.state.session_scheduler_worker_host(),
            )),
        )
    }

    pub fn session_subagent_read(&self) -> SessionSubagentReadHandle {
        SessionSubagentReadHandle::new(self.session_store_lookup())
    }

    pub fn session_subagent_mcp_read(&self) -> SessionSubagentMcpReadHandle {
        let provider_inactivity_timeout = Arc::new({
            let sessions = Arc::clone(&self.state.sessions);
            move || {
                let sessions = Arc::clone(&sessions);
                Box::pin(async move { sessions.provider_inactivity_timeout().await })
                    as SessionSubagentMcpReadFuture<_>
            }
        });
        let emit_legacy_context_window_key_reject = Arc::new({
            let perf_telemetry = self.state.telemetry.perf_telemetry.clone();
            move |legacy_key: String| {
                let perf_telemetry = perf_telemetry.clone();
                Box::pin(async move {
                    let mut labels = HashMap::new();
                    labels.insert("source".to_string(), "daemon".to_string());
                    labels.insert(
                        "surface".to_string(),
                        "sessions.context_window_summary".to_string(),
                    );
                    labels.insert("issue".to_string(), "legacy_context_window_key".to_string());
                    labels.insert("legacy_key".to_string(), legacy_key);
                    perf_telemetry
                        .record_metric(
                            PerfMetric {
                                name: "compat.payload_reject_count".to_string(),
                                kind: PerfMetricKind::Counter,
                                unit: "count".to_string(),
                                value: 1.0,
                                labels,
                            },
                            None,
                            None,
                            None,
                        )
                        .await;
                }) as SessionSubagentMcpReadFuture<_>
            }
        });
        SessionSubagentMcpReadHandle::new(
            self.session_store_lookup(),
            provider_inactivity_timeout,
            emit_legacy_context_window_key_reject,
        )
    }

    pub fn session_subagent_mcp_control(&self) -> SessionSubagentMcpControlHandle {
        let provider_inactivity_timeout = Arc::new({
            let sessions = Arc::clone(&self.state.sessions);
            move || {
                let sessions = Arc::clone(&sessions);
                Box::pin(async move { sessions.provider_inactivity_timeout().await })
                    as SessionSubagentMcpControlFuture<_>
            }
        });
        let emit_legacy_context_window_key_reject = Arc::new({
            let perf_telemetry = self.state.telemetry.perf_telemetry.clone();
            move |legacy_key: String| {
                let perf_telemetry = perf_telemetry.clone();
                Box::pin(async move {
                    let mut labels = HashMap::new();
                    labels.insert("source".to_string(), "daemon".to_string());
                    labels.insert(
                        "surface".to_string(),
                        "sessions.context_window_summary".to_string(),
                    );
                    labels.insert("issue".to_string(), "legacy_context_window_key".to_string());
                    labels.insert("legacy_key".to_string(), legacy_key);
                    perf_telemetry
                        .record_metric(
                            PerfMetric {
                                name: "compat.payload_reject_count".to_string(),
                                kind: PerfMetricKind::Counter,
                                unit: "count".to_string(),
                                value: 1.0,
                                labels,
                            },
                            None,
                            None,
                            None,
                        )
                        .await;
                }) as SessionSubagentMcpControlFuture<_>
            }
        });
        let session_stores = self.session_store_lookup();
        let scheduler_spawner = SessionSubagentMcpControlSchedulerSpawner::new(Arc::downgrade(
            &self.state.session_scheduler_worker_host(),
        ));
        let publish_host = SessionSubagentMcpControlPublicationHost::new(
            session_stores.clone(),
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        );
        let child_run_host = SubagentChildRunHost::new(
            self.weak_session_store_lookup(),
            SessionEventHeadSubscriber::new(Arc::downgrade(&self.state.sessions)),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        );
        let worktree_host = self.task_worktree_host();
        let spawn_host = Arc::new(SubagentSpawnHost::new(SubagentSpawnHostParts {
            session_stores: session_stores.clone(),
            session_runtime: Arc::clone(&self.state.sessions),
            scheduler_spawner: scheduler_spawner.clone(),
            publish_host: publish_host.clone(),
            child_run_host,
            session_vcs: self.session_vcs(),
            worktrees: worktree_host,
            provider_launch: self.provider_workspace_launch_runtime(),
            global_store: self.state.global_store().clone(),
            perf_telemetry: self.state.telemetry.perf_telemetry.clone(),
            data_root: self.state.core.data_root.clone(),
        }));
        let archive_worktree_cleanup = Arc::new(
            crate::daemon::sessions::subagents::SubagentArchiveWorktreeCleanupHost::new(
                self.state.core.data_root.clone(),
                self.state.global_store().clone(),
                crate::daemon::workspaces::vcs_hooks::WorkspaceVcsHookHost::new(
                    self.state.core.data_root.clone(),
                    self.state.core.daemon_url.clone(),
                    self.state.global_store().clone(),
                    self.protected_workspace_store_lookup(),
                    Arc::clone(&self.state.execution.harness),
                ),
            ),
        );
        SessionSubagentMcpControlHandle::new(SessionSubagentMcpControlHandleParts {
            session_stores,
            session_runtime: Arc::clone(&self.state.sessions),
            scheduler_spawner,
            publish_host,
            lifecycle_host: SessionSubagentMcpControlLifecycleHost::new(
                self.state.global_store().clone(),
                Arc::clone(&self.state.workspaces.workspace_active_snapshot),
                Arc::clone(&self.state.providers),
            ),
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            spawn_host,
            archive_worktree_cleanup,
            provider_inactivity_timeout,
            emit_legacy_context_window_key_reject,
        })
    }

    pub fn session_read_models(&self) -> SessionReadModelsHandle {
        SessionReadModelsHandle::new(
            self.state.global_store().clone(),
            self.session_store_lookup(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            self.state.core.tool_output_spool_dir.clone(),
            self.state.telemetry.perf_telemetry.clone(),
        )
    }

    fn session_store_lookup(&self) -> SessionStoreLookup {
        SessionStoreLookup::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
        )
    }

    fn task_publication_host(
        &self,
    ) -> Arc<crate::daemon::task_session_effects::TaskPublicationHost> {
        Arc::new(
            crate::daemon::task_session_effects::TaskPublicationHost::new(
                self.protected_workspace_store_lookup(),
                Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            ),
        )
    }

    fn task_session_cleanup_host(
        &self,
    ) -> crate::daemon::task_session_effects::TaskSessionCleanupHost {
        crate::daemon::task_session_effects::TaskSessionCleanupHost::new(
            self.state.global_store().clone(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.providers),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            self.protected_workspace_store_lookup(),
        )
    }

    fn session_publication_effects(
        &self,
    ) -> crate::daemon::task_session_effects::SessionPublicationEffects {
        crate::daemon::task_session_effects::SessionPublicationEffects::new(
            Arc::clone(&self.state.sessions),
            self.session_store_lookup(),
            self.task_publication_host(),
        )
    }

    fn weak_session_store_lookup(&self) -> WeakSessionStoreLookup {
        WeakSessionStoreLookup::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::downgrade(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }

    fn session_artifact_effects(&self) -> Arc<SessionArtifactEffects> {
        self.session_publication_effects()
            .session_artifact_effects()
    }

    pub fn session_artifacts(&self) -> SessionArtifactsHandle {
        SessionArtifactsHandle::new(
            self.session_store_lookup(),
            self.state.core.tool_output_spool_dir.clone(),
            self.session_artifact_effects(),
        )
    }

    fn session_vcs_effects(&self) -> Arc<SessionVcsEffects> {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let worktree_has_vcs_repo = Arc::new({
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    crate::daemon::git_status::worktree_has_vcs_repo(&vcs_execution, &worktree)
                        .await
                }) as SessionVcsFuture<_>
            }
        });
        let load_git_status_snapshot = Arc::new({
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, include_untracked_files: bool, include_entries: bool| {
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    crate::daemon::git_status::load_git_status_snapshot(
                        &vcs_execution,
                        &worktree,
                        include_untracked_files,
                        include_entries,
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let resolve_worktree_commit = Arc::new({
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, revision: String| {
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    let source = crate::daemon::git_status::HttpWorktreeVcsSource::new(
                        &vcs_execution,
                        &worktree,
                    );
                    source.resolve_commit(&revision).await
                }) as SessionVcsFuture<_>
            }
        });
        let diff_worktree_for_session = Arc::new({
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, base_commit_sha: String| {
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    crate::daemon::workspaces::diff_worktree_for_session(
                        &vcs_execution,
                        &worktree,
                        &base_commit_sha,
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let diff_worktree_summary_for_session = Arc::new({
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, base_commit_sha: String| {
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    crate::daemon::workspaces::diff_worktree_summary_for_session(
                        &vcs_execution,
                        &worktree,
                        &base_commit_sha,
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let resolve_worktree_diff_base = Arc::new({
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, query: SessionVcsDiffBaseQuery| {
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    let source = crate::daemon::git_status::HttpWorktreeVcsSource::new(
                        &vcs_execution,
                        &worktree,
                    );
                    ctx_worktree_vcs_service::resolve_worktree_diff_base_from_source(
                        &source,
                        &worktree,
                        WorktreeVcsDiffBaseQuery {
                            base_commit_sha: query.base_commit_sha,
                            target_branch: query.target_branch,
                        },
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let apply_worktree_vcs_session_patch =
            Arc::new(|worktree: Worktree, patch: String, reverse_patch: bool| {
                Box::pin(async move {
                    ctx_worktree_vcs_service::apply_worktree_vcs_session_patch(
                        Path::new(&worktree.root_path),
                        &patch,
                        reverse_patch,
                    )
                    .await
                }) as SessionVcsFuture<_>
            });
        let cached_worktree_vcs_snapshot = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree_id: WorktreeId| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .get_worktree_vcs_snapshot(&vcs_execution, worktree_id)
                        .await
                }) as SessionVcsFuture<_>
            }
        });
        let emit_compat_payload_reject_counter = Arc::new({
            let perf_telemetry = self.state.telemetry.perf_telemetry.clone();
            move |surface: &'static str, issue: &'static str| {
                let perf_telemetry = perf_telemetry.clone();
                Box::pin(async move {
                    let mut labels = HashMap::new();
                    labels.insert("source".to_string(), "daemon".to_string());
                    labels.insert("surface".to_string(), surface.to_string());
                    labels.insert("issue".to_string(), issue.to_string());
                    let metric = PerfMetric {
                        name: "compat.payload_reject_count".to_string(),
                        kind: PerfMetricKind::Counter,
                        unit: "count".to_string(),
                        value: 1.0,
                        labels,
                    };
                    perf_telemetry.record_metric(metric, None, None, None).await;
                }) as SessionVcsFuture<_>
            }
        });
        SessionVcsEffects::new(SessionVcsEffectsParts {
            worktree_has_vcs_repo,
            load_git_status_snapshot,
            resolve_worktree_commit,
            diff_worktree_for_session,
            diff_worktree_summary_for_session,
            resolve_worktree_diff_base,
            apply_worktree_vcs_session_patch,
            cached_worktree_vcs_snapshot,
            emit_compat_payload_reject_counter,
            is_no_vcs_repo_error: Arc::new(ctx_worktree_vcs_service::is_no_vcs_repo_error),
        })
    }

    pub fn session_vcs(&self) -> SessionVcsHandle {
        SessionVcsHandle::new(self.session_store_lookup(), self.session_vcs_effects())
    }

    fn task_worktree_host(&self) -> Arc<TaskWorktreeHost> {
        let workspace_stores = self.protected_workspace_store_lookup();
        let attachments = self.workspace_attachments_runtime();
        let vcs_hooks = Arc::new(
            crate::daemon::workspaces::vcs_hooks::WorkspaceVcsHookHost::new(
                self.state.core.data_root.clone(),
                self.state.core.daemon_url.clone(),
                self.state.global_store().clone(),
                workspace_stores.clone(),
                Arc::clone(&self.state.execution.harness),
            ),
        );
        Arc::new(TaskWorktreeHost::new(TaskWorktreeHostParts {
            data_root: self.state.core.data_root.clone(),
            daemon_url: self.state.core.daemon_url.clone(),
            global_store: self.state.global_store().clone(),
            workspace_stores,
            harness: Arc::clone(&self.state.execution.harness),
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            bootstrap_gates: Arc::clone(&self.state.workspaces.worktree_bootstrap_gates),
            attachments,
            vcs_hooks,
        }))
    }

    pub(crate) fn workspace_attachments_runtime(
        &self,
    ) -> Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime> {
        Arc::new(
            crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime::new(
                self.state.core.data_root.clone(),
                self.state.core.daemon_url.clone(),
                self.state.global_store().clone(),
                self.protected_workspace_store_lookup(),
                Arc::clone(&self.state.execution.harness),
                Arc::clone(&self.state.workspaces.attachment_materialization),
            ),
        )
    }

    fn task_lifecycle_effects(&self) -> Arc<TaskLifecycleEffects> {
        crate::daemon::task_session_effects::task_lifecycle_effects(
            self.task_publication_host(),
            self.task_session_cleanup_host(),
        )
    }

    pub fn task_lifecycle(&self) -> TaskLifecycleHandle {
        TaskLifecycleHandle::new(
            self.state.global_store().clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            self.task_worktree_host(),
            self.task_lifecycle_effects(),
        )
    }

    fn protected_workspace_store_lookup(&self) -> ProtectedWorkspaceStoreLookup {
        ProtectedWorkspaceStoreLookup::new(
            self.state.core.stores.clone(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }

    fn terminal_launch_host(&self) -> TerminalLaunchHost {
        TerminalLaunchHost::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            Arc::clone(&self.state.execution.harness),
            Arc::clone(&self.state.transport.terminals),
        )
    }

    fn web_session_worker_runtime_host(&self) -> WebSessionWorkerRuntimeHost {
        WebSessionWorkerRuntimeHost::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    fn web_session_launch_host(&self) -> WebSessionLaunchHost {
        WebSessionLaunchHost::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            self.state.core.data_root.clone(),
            self.web_session_worker_runtime_host(),
            Arc::clone(&self.state.transport.web_sessions),
        )
    }

    fn task_store_lookup(&self) -> TaskStoreLookup {
        TaskStoreLookup::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
        )
    }

    fn task_metadata_effects(&self) -> Arc<TaskMetadataEffects> {
        crate::daemon::task_session_effects::task_metadata_effects(self.task_publication_host())
    }

    pub fn task_listing(&self) -> TaskListingHandle {
        let snapshot = Arc::clone(&self.state.workspaces.workspace_active_snapshot);
        let archived_rev_loader: TaskArchivedRevLoader = Arc::new(move |workspace_id| {
            let snapshot = Arc::clone(&snapshot);
            Box::pin(async move {
                let (_, archived_rev) = snapshot.snapshot_state(workspace_id).await;
                archived_rev
            })
        });
        TaskListingHandle::new(self.protected_workspace_store_lookup(), archived_rev_loader)
    }

    pub fn task_session_listing(&self) -> TaskSessionListingHandle {
        TaskSessionListingHandle::new(self.task_store_lookup())
    }

    pub fn task_read_state(&self) -> TaskReadStateHandle {
        TaskReadStateHandle::new(self.task_store_lookup(), self.task_metadata_effects())
    }

    pub fn task_title(&self) -> TaskTitleHandle {
        let web_sessions = Arc::clone(&self.state.transport.web_sessions);
        let close_web_sessions_for_task: TaskCloseWebSessionsForTask =
            Arc::new(move |session_ids, worktree_ids| {
                let web_sessions = Arc::clone(&web_sessions);
                Box::pin(async move {
                    web_sessions
                        .close_for_task(&session_ids, &worktree_ids)
                        .await
                })
            });
        TaskTitleHandle::new(
            self.task_store_lookup(),
            self.task_metadata_effects(),
            close_web_sessions_for_task,
        )
    }

    fn task_session_admission_effects(&self) -> Arc<TaskAdmissionSessionEffects> {
        crate::daemon::task_session_effects::task_admission_session_effects(
            self.session_publication_effects(),
            Arc::clone(&self.state.sessions),
            SessionMessageSchedulerSpawner::new(Arc::downgrade(
                &self.state.session_scheduler_worker_host(),
            )),
            self.session_title_model_mode(),
            self.task_publication_host(),
        )
    }

    fn task_session_admission_provider_status(&self) -> ProviderStatusHandle {
        ProviderStatusHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    fn task_session_admission_model_catalog_loader(&self) -> TaskAdmissionModelCatalogLoader {
        let launch = self.provider_workspace_launch_runtime();
        Arc::new(
            move |workspace: Workspace,
                  provider_id: String,
                  execution_environment: ExecutionEnvironment| {
                let launch = Arc::clone(&launch);
                Box::pin(async move {
                    crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment(
                        launch.as_ref(),
                        &workspace,
                        &provider_id,
                        execution_environment,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            },
        )
    }

    pub fn task_session_admission(&self) -> TaskSessionAdmissionHandle {
        TaskSessionAdmissionHandle::new(
            self.state.global_store().clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.providers),
            self.task_session_admission_provider_status(),
            self.task_worktree_host(),
            self.task_session_admission_effects(),
            self.task_session_admission_model_catalog_loader(),
            self.state.telemetry.telemetry.clone(),
            self.state.telemetry.ops_events.clone(),
            self.state.telemetry.perf_telemetry.clone(),
        )
    }

    pub fn task_creation(&self) -> TaskCreationHandle {
        TaskCreationHandle::new(
            self.state.global_store().clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            self.task_session_admission(),
            self.task_lifecycle(),
        )
    }

    pub fn workspace_active(&self) -> WorkspaceActiveHandle {
        let hydration = WorkspaceActiveHydrationRuntime::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        );
        let cache = WorkspaceActiveCacheRuntime::new(
            Arc::clone(&self.state.workspaces.workspace_active_snapshot_cache),
            Arc::clone(&self.state.workspaces.workspace_active_heads_cache),
        );
        let merge_queue = self.merge_queue_route_host();
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let hydration = hydration.clone();
            move |workspace_id: WorkspaceId| {
                let hydration = hydration.clone();
                Box::pin(async move {
                    hydration
                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                        .await
                }) as WorkspaceActiveFuture<_>
            }
        });
        let activate_workspace_merge_queue = Arc::new({
            let merge_queue = Arc::clone(&merge_queue);
            move |workspace_id: WorkspaceId| {
                let merge_queue = Arc::clone(&merge_queue);
                Box::pin(async move {
                    ctx_merge_queue::activate_workspace_merge_queue(&merge_queue, workspace_id)
                        .await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        let cache_workspace_active_snapshot = Arc::new({
            let cache = cache.clone();
            move |snapshot: WorkspaceActiveSnapshot| {
                let cache = cache.clone();
                Box::pin(async move {
                    cache.cache_workspace_active_snapshot(snapshot).await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        let cache_workspace_active_heads = Arc::new({
            let cache = cache.clone();
            move |heads: WorkspaceActiveHeadBatch| {
                let cache = cache.clone();
                Box::pin(async move {
                    cache.cache_workspace_active_heads(heads).await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        WorkspaceActiveHandle::new(WorkspaceActiveHandleParts {
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            effects: WorkspaceActiveEffects::new(WorkspaceActiveEffectsParts {
                ensure_workspace_active_snapshot_hydrated,
                activate_workspace_merge_queue,
                cache_workspace_active_snapshot,
                cache_workspace_active_heads,
            }),
        })
    }

    pub fn workspace_stream(&self) -> WorkspaceStreamHandle {
        let workspace_stores = ProtectedWorkspaceStoreLookup::new(
            self.state.core.stores.clone(),
            Arc::clone(&self.state.sessions),
            Arc::clone(&self.state.transport.merge_queue),
        );
        let session_stores =
            SessionStoreLookup::new(self.state.global_store().clone(), workspace_stores.clone());
        let hydration = WorkspaceActiveHydrationRuntime::new(
            self.state.global_store().clone(),
            workspace_stores.clone(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
        );
        let merge_queue = self.merge_queue_route_host();
        let lifecycle_host = Arc::new(WorkspaceStreamSessionLifecycleHost::new(
            self.state.global_store().clone(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            Arc::clone(&self.state.providers),
        ));
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let hydration = hydration.clone();
            move |workspace_id: WorkspaceId| {
                let hydration = hydration.clone();
                Box::pin(async move {
                    hydration
                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                        .await
                }) as WorkspaceStreamFuture<_>
            }
        });
        let activate_workspace_merge_queue = Arc::new({
            let merge_queue = Arc::clone(&merge_queue);
            move |workspace_id: WorkspaceId| {
                let merge_queue = Arc::clone(&merge_queue);
                Box::pin(async move {
                    ctx_merge_queue::activate_workspace_merge_queue(&merge_queue, workspace_id)
                        .await;
                }) as WorkspaceStreamFuture<_>
            }
        });
        WorkspaceStreamHandle::new(WorkspaceStreamHandleParts {
            global_store: self.state.global_store().clone(),
            workspace_stores,
            session_stores,
            active_snapshot: Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            sessions: Arc::clone(&self.state.sessions),
            lifecycle_host,
            telemetry: self.state.telemetry.telemetry.clone(),
            perf_telemetry: self.state.telemetry.perf_telemetry.clone(),
            effects: WorkspaceStreamEffects::new(WorkspaceStreamEffectsParts {
                ensure_workspace_active_snapshot_hydrated,
                activate_workspace_merge_queue,
            }),
        })
    }

    pub fn workspace_vcs_stream(&self) -> WorkspaceVcsStreamHandle {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let ensure_worktree_vcs_watcher = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .ensure_git_status_watcher(vcs_execution, worktree)
                        .await;
                }) as WorkspaceVcsStreamWatcherFuture
            }
        });
        let refresh_worktree_vcs = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree, summary: bool, touched_files: bool| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .ensure_git_status_watcher(vcs_execution.clone(), worktree.clone())
                        .await;
                    crate::daemon::git_status::request_worktree_vcs_refresh_without_transient(
                        &vcs_runtime,
                        &vcs_execution,
                        &worktree,
                        summary,
                        touched_files,
                    )
                    .await
                }) as WorkspaceVcsStreamRefreshFuture
            }
        });
        WorkspaceVcsStreamHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime::from_workspace_runtime(
                &self.state.workspaces,
            ),
            self.state.telemetry.perf_telemetry.clone(),
            ensure_worktree_vcs_watcher,
            refresh_worktree_vcs,
        )
    }

    #[cfg(test)]
    pub(in crate::daemon) fn workspace_vcs_stream_with_refresh_effect(
        &self,
        refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
    ) -> WorkspaceVcsStreamHandle {
        let vcs_runtime = self.worktree_vcs_runtime_host();
        let vcs_execution = self.worktree_vcs_execution_host();
        let ensure_worktree_vcs_watcher = Arc::new({
            let vcs_runtime = vcs_runtime.clone();
            let vcs_execution = vcs_execution.clone();
            move |worktree: Worktree| {
                let vcs_runtime = vcs_runtime.clone();
                let vcs_execution = vcs_execution.clone();
                Box::pin(async move {
                    vcs_runtime
                        .ensure_git_status_watcher(vcs_execution, worktree)
                        .await;
                }) as WorkspaceVcsStreamWatcherFuture
            }
        });
        WorkspaceVcsStreamHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime::from_workspace_runtime(
                &self.state.workspaces,
            ),
            self.state.telemetry.perf_telemetry.clone(),
            ensure_worktree_vcs_watcher,
            refresh_worktree_vcs,
        )
    }

    pub fn provider_accounts(&self) -> ProviderAccountsHandle {
        ProviderAccountsHandle::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn provider_bootstrap(&self) -> ProviderBootstrapHandle {
        ProviderBootstrapHandle::new(
            self.state.core.data_root.clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    fn provider_workspace_launch_runtime(&self) -> Arc<ProviderWorkspaceLaunchRuntime> {
        Arc::new(ProviderWorkspaceLaunchRuntime::new(
            self.state.core.data_root.clone(),
            self.state.core.daemon_url.clone(),
            self.state.core.auth_token.clone(),
            ProtectedWorkspaceStoreLookup::new(
                self.state.core.stores.clone(),
                Arc::clone(&self.state.sessions),
                Arc::clone(&self.state.transport.merge_queue),
            ),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
            Arc::clone(&self.state.execution.harness),
        ))
    }

    pub fn provider_options(&self) -> ProviderOptionsHandle {
        ProviderOptionsHandle::new(self.provider_workspace_launch_runtime())
    }

    pub fn provider_workspace_auth(&self) -> ProviderWorkspaceAuthHandle {
        ProviderWorkspaceAuthHandle::new(self.provider_workspace_launch_runtime())
    }

    pub fn provider_status(&self) -> ProviderStatusHandle {
        ProviderStatusHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn provider_admin(&self) -> ProviderAdminHandle {
        ProviderAdminHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn provider_install(&self) -> ProviderInstallHandle {
        ProviderInstallHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    pub fn provider_auth_import(&self) -> ProviderAuthImportHandle {
        ProviderAuthImportHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn provider_usage(&self) -> ProviderUsageHandle {
        ProviderUsageHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.core.shutdown_tx.clone(),
        )
    }

    pub fn provider_harness_config(&self) -> ProviderHarnessConfigHandle {
        ProviderHarnessConfigHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
        )
    }

    pub fn telemetry(&self) -> TelemetryHandle {
        TelemetryHandle::new(self.state.core.data_root.clone(), &self.state.telemetry)
    }

    pub fn terminal_route(&self) -> TerminalRouteHandle {
        TerminalRouteHandle::new(
            Arc::clone(&self.state.transport.terminals),
            self.terminal_launch_host(),
        )
    }

    pub fn web_session_route(&self) -> WebSessionRouteHandle {
        WebSessionRouteHandle::new(
            Arc::clone(&self.state.transport.web_sessions),
            self.web_session_launch_host(),
        )
    }

    pub fn execution_launch(&self) -> ExecutionLaunchHandle {
        ExecutionLaunchHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            Arc::clone(&self.state.execution.setup),
            self.state.core.daemon_url.clone(),
        )
    }

    pub fn linux_sandbox_runtime(&self) -> LinuxSandboxRuntimeHandle {
        LinuxSandboxRuntimeHandle::new(
            self.state.core.data_root.clone(),
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
            Arc::clone(&self.state.transport.terminals),
            Arc::clone(&self.state.execution.harness),
        )
    }

    pub fn update_drain(&self) -> UpdateDrainHandle {
        UpdateDrainHandle::new(
            self.state.global_store().clone(),
            self.state.core.stores.clone(),
            Arc::clone(&self.state.core.update_drain),
        )
    }

    pub fn daemon_shutdown(&self) -> DaemonShutdownHandle {
        let shutdown_host = DaemonShutdownHost::new(DaemonShutdownHostParts {
            global_store: self.state.global_store().clone(),
            stores: self.state.core.stores.clone(),
            session_stores: self.session_store_lookup(),
            session_lifecycle: Arc::clone(&self.state.sessions),
            session_publication: self.session_publication_effects(),
            provider_lifecycle: Arc::clone(&self.state.providers),
            update_drain: Arc::clone(&self.state.core.update_drain),
            substrate_lifecycle: Arc::clone(&self.state.execution.harness),
            shutdown_signal: self.state.core.shutdown_tx.clone(),
        });
        DaemonShutdownHandle::new(self.state.core.local_shutdown_token.clone(), shutdown_host)
    }
}

pub(in crate::daemon) type MergeQueueNoticePublicationFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
pub(in crate::daemon) type MergeQueueNoticePublicationEffect =
    Arc<dyn Fn(MergeQueueNoticeSessionEvent) -> MergeQueueNoticePublicationFuture + Send + Sync>;

pub(in crate::daemon) struct MergeQueueNoticeSessionEvent {
    event: SessionEvent,
}

impl MergeQueueNoticeSessionEvent {
    pub(in crate::daemon) fn new(event: SessionEvent) -> anyhow::Result<Self> {
        if !matches!(&event.event_type, SessionEventType::Notice) {
            anyhow::bail!("merge queue notice publication requires a notice event");
        }
        let kind = event
            .payload_json
            .get("kind")
            .and_then(serde_json::Value::as_str);
        if !matches!(
            kind,
            Some("merge_queue_sync" | "merge_queue_canonical_sync")
        ) {
            anyhow::bail!("merge queue notice publication requires a merge queue payload");
        }
        Ok(Self { event })
    }

    pub(in crate::daemon) fn into_event(self) -> SessionEvent {
        self.event
    }
}

#[derive(Clone)]
pub struct MergeQueueApiHandle {
    host: Arc<crate::daemon::merge_queue::MergeQueueRouteHost>,
}

impl MergeQueueApiHandle {
    pub(in crate::daemon) fn new(
        host: Arc<crate::daemon::merge_queue::MergeQueueRouteHost>,
    ) -> Self {
        Self { host }
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.host.existing_workspace_store(workspace_id).await
    }

    pub(in crate::daemon) async fn submit_merge_queue_entry(
        &self,
        params: ctx_merge_queue::MergeQueueSubmitParams,
    ) -> anyhow::Result<ctx_core::models::MergeQueueEntry> {
        ctx_merge_queue::submit_merge_queue_entry::<crate::daemon::merge_queue::MergeQueueRouteHost>(
            &self.host,
            params,
        )
        .await
    }

    pub(in crate::daemon) async fn cancel_merge_queue_entry(
        &self,
        workspace_id: WorkspaceId,
        entry_id: ctx_core::ids::MergeQueueEntryId,
    ) -> anyhow::Result<ctx_core::models::MergeQueueEntry> {
        ctx_merge_queue::cancel_merge_queue_entry::<crate::daemon::merge_queue::MergeQueueRouteHost>(
            &self.host,
            workspace_id,
            entry_id,
        )
        .await
    }

    pub(in crate::daemon) async fn retry_merge_queue_entry(
        &self,
        workspace_id: WorkspaceId,
        entry_id: ctx_core::ids::MergeQueueEntryId,
    ) -> anyhow::Result<ctx_core::models::MergeQueueEntry> {
        ctx_merge_queue::retry_merge_queue_entry::<crate::daemon::merge_queue::MergeQueueRouteHost>(
            &self.host,
            workspace_id,
            entry_id,
        )
        .await
    }

    pub(in crate::daemon) async fn get_workspace_merge_queue_entry(
        &self,
        workspace_id: WorkspaceId,
        entry_id: ctx_core::ids::MergeQueueEntryId,
    ) -> anyhow::Result<ctx_core::models::MergeQueueEntry> {
        ctx_merge_queue::get_workspace_merge_queue_entry::<
            crate::daemon::merge_queue::MergeQueueRouteHost,
        >(self.host.as_ref(), workspace_id, entry_id)
        .await
    }

    pub(in crate::daemon) async fn require_scoped_mcp_session_context(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::ScopedMcpSessionAccessError> {
        self.host
            .require_scoped_mcp_session_context(mcp_auth, session_id)
            .await
    }
}

#[derive(Clone)]
pub struct UpdateActivityHandle {
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
    data_root: PathBuf,
}

impl UpdateActivityHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
        data_root: PathBuf,
    ) -> Self {
        Self {
            global_store,
            stores,
            update_drain,
            data_root,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> &UpdateDrainCoordinator {
        self.update_drain.as_ref()
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Clone)]
pub struct SettingsHandle {
    store: Store,
    telemetry: Telemetry,
    perf_telemetry: PerfTelemetry,
    resource_sampler: Arc<Mutex<ResourceSampler>>,
    resource_governance: Arc<Mutex<ResourceGovernanceRuntime>>,
    providers: Arc<ProviderRuntime>,
    terminals: Arc<TerminalManager>,
}

impl SettingsHandle {
    pub(in crate::daemon) fn new(
        store: Store,
        telemetry: Telemetry,
        perf_telemetry: PerfTelemetry,
        resource_sampler: Arc<Mutex<ResourceSampler>>,
        resource_governance: Arc<Mutex<ResourceGovernanceRuntime>>,
        providers: Arc<ProviderRuntime>,
        terminals: Arc<TerminalManager>,
    ) -> Self {
        Self {
            store,
            telemetry,
            perf_telemetry,
            resource_sampler,
            resource_governance,
            providers,
            terminals,
        }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) fn resource_sampler(&self) -> &Mutex<ResourceSampler> {
        self.resource_sampler.as_ref()
    }

    pub(in crate::daemon) fn resource_governance(&self) -> &Mutex<ResourceGovernanceRuntime> {
        self.resource_governance.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn terminals(&self) -> &TerminalManager {
        self.terminals.as_ref()
    }
}

#[derive(Clone)]
pub struct MobileRuntimeHandle {
    store: Store,
    mobile_tunnel: MobileTunnelManager,
    daemon_url: String,
    auth_token_configured: bool,
}

impl MobileRuntimeHandle {
    pub(in crate::daemon) fn new(
        store: Store,
        mobile_tunnel: MobileTunnelManager,
        daemon_url: String,
        auth_token_configured: bool,
    ) -> Self {
        Self {
            store,
            mobile_tunnel,
            daemon_url,
            auth_token_configured,
        }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }

    pub(in crate::daemon) fn mobile_tunnel(&self) -> &MobileTunnelManager {
        &self.mobile_tunnel
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn auth_token_configured(&self) -> bool {
        self.auth_token_configured
    }
}

#[derive(Clone)]
pub struct MobileSecureProxyHandle {
    store: Store,
    health: HealthHandle,
    telemetry: Telemetry,
}

impl MobileSecureProxyHandle {
    pub(in crate::daemon) fn new(store: Store, health: HealthHandle, telemetry: Telemetry) -> Self {
        Self {
            store,
            health,
            telemetry,
        }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }

    pub(in crate::daemon) fn health(&self) -> &HealthHandle {
        &self.health
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }
}

#[derive(Clone)]
pub struct RunArchiveHandle {
    workspace_stores: ProtectedWorkspaceStoreLookup,
}

impl RunArchiveHandle {
    pub(in crate::daemon) fn new(workspace_stores: ProtectedWorkspaceStoreLookup) -> Self {
        Self { workspace_stores }
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }
}

#[derive(Clone)]
pub struct ResourceUtilizationHandle {
    workspace_stores: ProtectedWorkspaceStoreLookup,
    providers: Arc<ProviderRuntime>,
    resource_sampler: Arc<Mutex<ResourceSampler>>,
}

impl ResourceUtilizationHandle {
    pub(in crate::daemon) fn new(
        workspace_stores: ProtectedWorkspaceStoreLookup,
        providers: Arc<ProviderRuntime>,
        resource_sampler: Arc<Mutex<ResourceSampler>>,
    ) -> Self {
        Self {
            workspace_stores,
            providers,
            resource_sampler,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.workspace_stores.global_store()
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn resource_sampler(&self) -> &Mutex<ResourceSampler> {
        self.resource_sampler.as_ref()
    }
}

#[derive(Clone)]
pub(in crate::daemon) struct ProviderWorkspaceLaunchRuntime {
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
    harness: Arc<HarnessRuntimeManager>,
}

impl ProviderWorkspaceLaunchRuntime {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        daemon_url: String,
        auth_token: Option<String>,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
        harness: Arc<HarnessRuntimeManager>,
    ) -> Self {
        Self {
            data_root,
            daemon_url,
            auth_token,
            workspace_stores,
            providers,
            ops_events,
            harness,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn auth_token(&self) -> Option<&String> {
        self.auth_token.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.workspace_stores.global_store()
    }

    pub(in crate::daemon) async fn load_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<ctx_core::models::Workspace>> {
        self.global_store().get_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<crate::daemon::workspaces::ResolvedExistingWorktreeExecution> {
        let worktree = store
            .get_worktree(worktree_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("worktree not found"))?;
        let base_effective =
            ctx_settings_service::effective_execution_settings(self.global_store(), store)
                .await
                .context("loading workspace execution settings")?;
        let data_plane =
            ctx_worktree_data_plane::resolve_worktree_data_plane_with_host(self, &worktree)
                .await
                .context("resolving worktree data plane")?;
        let effective = ctx_worktree_data_plane::apply_data_plane_to_execution_settings(
            &base_effective,
            &data_plane,
        )
        .context("applying worktree data plane to execution settings")?;
        Ok(
            crate::daemon::workspaces::ResolvedExistingWorktreeExecution {
                worktree,
                effective,
            },
        )
    }

    pub(in crate::daemon) async fn effective_install_target_for_environment(
        &self,
        workspace_id: WorkspaceId,
        execution_environment: ExecutionEnvironment,
    ) -> anyhow::Result<InstallTarget> {
        let store = self.store_for_workspace(workspace_id).await?;
        ctx_settings_service::effective_install_target_for_environment(
            self.global_store(),
            &store,
            execution_environment,
        )
        .await
    }

    pub(in crate::daemon) async fn install_target_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<InstallTarget> {
        let store = self.store_for_workspace(workspace_id).await?;
        let effective =
            ctx_settings_service::effective_execution_settings(self.global_store(), &store)
                .await
                .with_context(|| {
                    format!(
                        "loading execution settings for workspace {}",
                        workspace_id.0
                    )
                })?;
        Ok(ctx_settings_service::install_target_for_settings(
            &effective,
        ))
    }
}

#[derive(Clone)]
pub struct ExecutionLaunchHandle {
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
    execution_setup: Arc<ExecutionSetupCoordinator>,
    daemon_url: String,
}

impl ExecutionLaunchHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
        execution_setup: Arc<ExecutionSetupCoordinator>,
        daemon_url: String,
    ) -> Self {
        Self {
            global_store,
            stores,
            update_drain,
            execution_setup,
            daemon_url,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> &UpdateDrainCoordinator {
        self.update_drain.as_ref()
    }

    pub(in crate::daemon) fn execution_setup(&self) -> &Arc<ExecutionSetupCoordinator> {
        &self.execution_setup
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }
}

#[derive(Clone)]
pub struct LinuxSandboxRuntimeHandle {
    data_root: PathBuf,
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
    terminals: Arc<TerminalManager>,
    harness: Arc<HarnessRuntimeManager>,
}

impl LinuxSandboxRuntimeHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
        terminals: Arc<TerminalManager>,
        harness: Arc<HarnessRuntimeManager>,
    ) -> Self {
        Self {
            data_root,
            global_store,
            stores,
            update_drain,
            terminals,
            harness,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> Arc<UpdateDrainCoordinator> {
        Arc::clone(&self.update_drain)
    }

    pub(in crate::daemon) fn terminals(&self) -> &TerminalManager {
        self.terminals.as_ref()
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }
}

#[cfg(test)]
pub(in crate::daemon) type CreateTerminalFuture =
    Pin<Box<dyn Future<Output = Result<TerminalSession, TerminalLaunchError>> + Send + 'static>>;
#[cfg(test)]
pub(in crate::daemon) type CreateTerminalEffect =
    Arc<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;

#[derive(Clone)]
enum TerminalRouteLaunch {
    Host(TerminalLaunchHost),
    #[cfg(test)]
    Override(CreateTerminalEffect),
}

#[derive(Clone)]
pub struct TerminalRouteHandle {
    terminals: Arc<TerminalManager>,
    launch: TerminalRouteLaunch,
}

impl TerminalRouteHandle {
    pub(in crate::daemon) fn new(
        terminals: Arc<TerminalManager>,
        launch: TerminalLaunchHost,
    ) -> Self {
        Self {
            terminals,
            launch: TerminalRouteLaunch::Host(launch),
        }
    }

    #[cfg(test)]
    pub(in crate::daemon) fn new_for_test(
        terminals: Arc<TerminalManager>,
        create_terminal: CreateTerminalEffect,
    ) -> Self {
        Self {
            terminals,
            launch: TerminalRouteLaunch::Override(create_terminal),
        }
    }

    pub(in crate::daemon) fn terminals(&self) -> &TerminalManager {
        self.terminals.as_ref()
    }

    pub(in crate::daemon) async fn create_terminal(
        &self,
        req: CreateTerminalLaunchRequest,
    ) -> Result<TerminalSession, TerminalLaunchError> {
        match &self.launch {
            TerminalRouteLaunch::Host(host) => {
                crate::daemon::terminals::create_workspace_terminal(host, req).await
            }
            #[cfg(test)]
            TerminalRouteLaunch::Override(create_terminal) => create_terminal(req).await,
        }
    }
}

#[cfg(test)]
pub(in crate::daemon) type CreateWebSessionFuture =
    Pin<Box<dyn Future<Output = Result<WebSessionInfo, WebSessionLaunchError>> + Send + 'static>>;
#[cfg(test)]
pub(in crate::daemon) type CreateWebSessionEffect =
    Arc<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;

#[derive(Clone)]
enum WebSessionRouteLaunch {
    Host(WebSessionLaunchHost),
    #[cfg(test)]
    Override(CreateWebSessionEffect),
}

#[derive(Clone)]
pub struct WebSessionRouteHandle {
    web_sessions: Arc<WebSessionManager>,
    launch: WebSessionRouteLaunch,
}

impl WebSessionRouteHandle {
    pub(in crate::daemon) fn new(
        web_sessions: Arc<WebSessionManager>,
        launch: WebSessionLaunchHost,
    ) -> Self {
        Self {
            web_sessions,
            launch: WebSessionRouteLaunch::Host(launch),
        }
    }

    #[cfg(test)]
    pub(in crate::daemon) fn new_for_test(
        web_sessions: Arc<WebSessionManager>,
        create_web_session: CreateWebSessionEffect,
    ) -> Self {
        Self {
            web_sessions,
            launch: WebSessionRouteLaunch::Override(create_web_session),
        }
    }

    pub(in crate::daemon) fn web_sessions(&self) -> &WebSessionManager {
        self.web_sessions.as_ref()
    }

    pub(in crate::daemon) fn web_sessions_arc(&self) -> Arc<WebSessionManager> {
        Arc::clone(&self.web_sessions)
    }

    pub(in crate::daemon) async fn create_web_session(
        &self,
        req: WebSessionLaunchRequest,
    ) -> Result<WebSessionInfo, WebSessionLaunchError> {
        match &self.launch {
            WebSessionRouteLaunch::Host(host) => {
                crate::daemon::web_sessions::create_web_session(host, req).await
            }
            #[cfg(test)]
            WebSessionRouteLaunch::Override(create_web_session) => create_web_session(req).await,
        }
    }
}

#[derive(Clone)]
pub struct UpdateDrainHandle {
    global_store: Store,
    stores: StoreManager,
    update_drain: Arc<UpdateDrainCoordinator>,
}

impl UpdateDrainHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        stores: StoreManager,
        update_drain: Arc<UpdateDrainCoordinator>,
    ) -> Self {
        Self {
            global_store,
            stores,
            update_drain,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn update_drain(&self) -> Arc<UpdateDrainCoordinator> {
        Arc::clone(&self.update_drain)
    }
}

#[derive(Clone)]
pub struct DaemonShutdownHandle {
    local_shutdown_token: Option<String>,
    shutdown_host: DaemonShutdownHost,
}

impl DaemonShutdownHandle {
    pub(in crate::daemon) fn new(
        local_shutdown_token: Option<String>,
        shutdown_host: DaemonShutdownHost,
    ) -> Self {
        Self {
            local_shutdown_token,
            shutdown_host,
        }
    }

    pub(in crate::daemon) fn local_shutdown_token_authorized(
        &self,
        supplied: Option<&str>,
    ) -> bool {
        let Some(expected) = self.local_shutdown_token.as_deref() else {
            return false;
        };
        supplied.is_some_and(|value| value == expected)
    }

    pub(in crate::daemon) async fn request_shutdown(
        &self,
        reason: String,
    ) -> Result<
        crate::daemon::DaemonTurnActivitySummary,
        crate::daemon::maintenance::DaemonShutdownError,
    > {
        crate::daemon::maintenance::request_daemon_shutdown(&self.shutdown_host, reason).await
    }
}

type WorkspaceActiveFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type WorkspaceActiveHydrationEffect = Arc<
    dyn Fn(
            WorkspaceId,
        )
            -> WorkspaceActiveFuture<Result<(), crate::daemon::workspaces::WorkspaceHydrationError>>
        + Send
        + Sync,
>;
type WorkspaceActiveUnitEffect =
    Arc<dyn Fn(WorkspaceId) -> WorkspaceActiveFuture<()> + Send + Sync>;
type WorkspaceActiveSnapshotCacheEffect =
    Arc<dyn Fn(WorkspaceActiveSnapshot) -> WorkspaceActiveFuture<()> + Send + Sync>;
type WorkspaceActiveHeadsCacheEffect =
    Arc<dyn Fn(WorkspaceActiveHeadBatch) -> WorkspaceActiveFuture<()> + Send + Sync>;

pub(in crate::daemon) struct WorkspaceActiveEffectsParts {
    ensure_workspace_active_snapshot_hydrated: WorkspaceActiveHydrationEffect,
    activate_workspace_merge_queue: WorkspaceActiveUnitEffect,
    cache_workspace_active_snapshot: WorkspaceActiveSnapshotCacheEffect,
    cache_workspace_active_heads: WorkspaceActiveHeadsCacheEffect,
}

pub(in crate::daemon) struct WorkspaceActiveEffects {
    ensure_workspace_active_snapshot_hydrated: WorkspaceActiveHydrationEffect,
    activate_workspace_merge_queue: WorkspaceActiveUnitEffect,
    cache_workspace_active_snapshot: WorkspaceActiveSnapshotCacheEffect,
    cache_workspace_active_heads: WorkspaceActiveHeadsCacheEffect,
}

impl WorkspaceActiveEffects {
    pub(in crate::daemon) fn new(parts: WorkspaceActiveEffectsParts) -> Arc<Self> {
        Arc::new(Self {
            ensure_workspace_active_snapshot_hydrated: parts
                .ensure_workspace_active_snapshot_hydrated,
            activate_workspace_merge_queue: parts.activate_workspace_merge_queue,
            cache_workspace_active_snapshot: parts.cache_workspace_active_snapshot,
            cache_workspace_active_heads: parts.cache_workspace_active_heads,
        })
    }
}

#[derive(Clone)]
pub struct WorkspaceActiveHandle {
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    effects: Arc<WorkspaceActiveEffects>,
}

pub(in crate::daemon) struct WorkspaceActiveHandleParts {
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    effects: Arc<WorkspaceActiveEffects>,
}

impl WorkspaceActiveHandle {
    pub(in crate::daemon) fn new(parts: WorkspaceActiveHandleParts) -> Self {
        Self {
            active_snapshot: parts.active_snapshot,
            effects: parts.effects,
        }
    }

    pub(in crate::daemon) async fn load_workspace_active_snapshot(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveSnapshot, crate::daemon::workspaces::WorkspaceHydrationError> {
        (self.effects.ensure_workspace_active_snapshot_hydrated)(workspace_id).await?;
        (self.effects.activate_workspace_merge_queue)(workspace_id).await;
        let snapshot = self
            .active_snapshot
            .active_snapshot(workspace_id, i64::MAX)
            .await;
        (self.effects.cache_workspace_active_snapshot)(snapshot.clone()).await;
        Ok(snapshot)
    }

    pub(in crate::daemon) async fn load_workspace_active_heads(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceActiveHeadBatch, crate::daemon::workspaces::WorkspaceHydrationError> {
        (self.effects.ensure_workspace_active_snapshot_hydrated)(workspace_id).await?;
        (self.effects.activate_workspace_merge_queue)(workspace_id).await;
        let heads = self.active_snapshot.active_heads(workspace_id).await;
        (self.effects.cache_workspace_active_heads)(heads.clone()).await;
        Ok(heads)
    }
}

#[cfg(test)]
mod workspace_active_tests {
    use super::*;
    use ctx_route_contracts::workspaces::{WorkspaceRouteErrorKind, WorkspaceRouteParams};

    fn workspace_active_test_handle(
        events: Arc<Mutex<Vec<&'static str>>>,
        hydrate_succeeds: bool,
    ) -> WorkspaceActiveHandle {
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let events = Arc::clone(&events);
            move |_workspace_id: WorkspaceId| {
                let events = Arc::clone(&events);
                Box::pin(async move {
                    events.lock().await.push("hydrate");
                    if hydrate_succeeds {
                        Ok(())
                    } else {
                        Err(crate::daemon::workspaces::WorkspaceHydrationError::NotFound)
                    }
                }) as WorkspaceActiveFuture<_>
            }
        })
            as WorkspaceActiveHydrationEffect;
        let activate_workspace_merge_queue = Arc::new({
            let events = Arc::clone(&events);
            move |_workspace_id: WorkspaceId| {
                let events = Arc::clone(&events);
                Box::pin(async move {
                    events.lock().await.push("activate_merge_queue");
                }) as WorkspaceActiveFuture<_>
            }
        }) as WorkspaceActiveUnitEffect;
        let cache_workspace_active_snapshot = Arc::new({
            let events = Arc::clone(&events);
            move |_snapshot: WorkspaceActiveSnapshot| {
                let events = Arc::clone(&events);
                Box::pin(async move {
                    events.lock().await.push("cache_snapshot");
                }) as WorkspaceActiveFuture<_>
            }
        }) as WorkspaceActiveSnapshotCacheEffect;
        let cache_workspace_active_heads = Arc::new({
            let events = Arc::clone(&events);
            move |_heads: WorkspaceActiveHeadBatch| {
                let events = Arc::clone(&events);
                Box::pin(async move {
                    events.lock().await.push("cache_heads");
                }) as WorkspaceActiveFuture<_>
            }
        }) as WorkspaceActiveHeadsCacheEffect;

        WorkspaceActiveHandle::new(WorkspaceActiveHandleParts {
            active_snapshot: Arc::new(WorkspaceActiveSnapshotHub::new()),
            effects: WorkspaceActiveEffects::new(WorkspaceActiveEffectsParts {
                ensure_workspace_active_snapshot_hydrated,
                activate_workspace_merge_queue,
                cache_workspace_active_snapshot,
                cache_workspace_active_heads,
            }),
        })
    }

    #[tokio::test]
    async fn workspace_active_snapshot_for_route_rejects_invalid_workspace_id() {
        let handle = workspace_active_test_handle(Arc::new(Mutex::new(Vec::new())), true);
        let error = handle
            .workspace_active_snapshot_for_route(WorkspaceRouteParams::new("not-a-workspace"))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");
    }

    #[tokio::test]
    async fn workspace_active_heads_for_route_rejects_invalid_workspace_id() {
        let handle = workspace_active_test_handle(Arc::new(Mutex::new(Vec::new())), true);
        let error = handle
            .workspace_active_heads_for_route(WorkspaceRouteParams::new("not-a-workspace"))
            .await
            .unwrap_err();

        assert_eq!(error.kind(), WorkspaceRouteErrorKind::BadRequest);
        assert_eq!(error.message(), "invalid workspace id");
    }

    #[tokio::test]
    async fn workspace_active_snapshot_handle_preserves_effect_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = workspace_active_test_handle(Arc::clone(&events), true);
        let workspace_id = WorkspaceId::new();

        let snapshot = handle
            .load_workspace_active_snapshot(workspace_id)
            .await
            .expect("active snapshot");

        assert_eq!(snapshot.workspace_id, workspace_id);
        assert_eq!(
            *events.lock().await,
            ["hydrate", "activate_merge_queue", "cache_snapshot"]
        );
    }

    #[tokio::test]
    async fn workspace_active_heads_handle_preserves_effect_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = workspace_active_test_handle(Arc::clone(&events), true);
        let workspace_id = WorkspaceId::new();

        let heads = handle
            .load_workspace_active_heads(workspace_id)
            .await
            .expect("active heads");

        assert_eq!(heads.workspace_id, workspace_id);
        assert_eq!(
            *events.lock().await,
            ["hydrate", "activate_merge_queue", "cache_heads"]
        );
    }

    #[tokio::test]
    async fn workspace_active_snapshot_handle_short_circuits_after_hydration_failure() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = workspace_active_test_handle(Arc::clone(&events), false);
        let error = handle
            .load_workspace_active_snapshot(WorkspaceId::new())
            .await
            .unwrap_err();

        assert_eq!(
            error.kind(),
            crate::daemon::workspaces::WorkspaceHydrationErrorKind::NotFound
        );
        assert_eq!(*events.lock().await, ["hydrate"]);
    }

    #[tokio::test]
    async fn workspace_active_heads_handle_short_circuits_after_hydration_failure() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handle = workspace_active_test_handle(Arc::clone(&events), false);
        let error = handle
            .load_workspace_active_heads(WorkspaceId::new())
            .await
            .unwrap_err();

        assert_eq!(
            error.kind(),
            crate::daemon::workspaces::WorkspaceHydrationErrorKind::NotFound
        );
        assert_eq!(*events.lock().await, ["hydrate"]);
    }
}

type WorkspaceStreamFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type WorkspaceStreamHydrationEffect = Arc<
    dyn Fn(
            WorkspaceId,
        )
            -> WorkspaceStreamFuture<Result<(), crate::daemon::workspaces::WorkspaceHydrationError>>
        + Send
        + Sync,
>;
type WorkspaceStreamUnitEffect =
    Arc<dyn Fn(WorkspaceId) -> WorkspaceStreamFuture<()> + Send + Sync>;

pub(in crate::daemon) struct WorkspaceStreamEffectsParts {
    ensure_workspace_active_snapshot_hydrated: WorkspaceStreamHydrationEffect,
    activate_workspace_merge_queue: WorkspaceStreamUnitEffect,
}

pub(in crate::daemon) struct WorkspaceStreamEffects {
    ensure_workspace_active_snapshot_hydrated: WorkspaceStreamHydrationEffect,
    activate_workspace_merge_queue: WorkspaceStreamUnitEffect,
}

impl WorkspaceStreamEffects {
    pub(in crate::daemon) fn new(parts: WorkspaceStreamEffectsParts) -> Arc<Self> {
        Arc::new(Self {
            ensure_workspace_active_snapshot_hydrated: parts
                .ensure_workspace_active_snapshot_hydrated,
            activate_workspace_merge_queue: parts.activate_workspace_merge_queue,
        })
    }
}

#[derive(Clone)]
pub struct WorkspaceStreamHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    session_stores: SessionStoreLookup,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    sessions: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    lifecycle_host: Arc<WorkspaceStreamSessionLifecycleHost>,
    telemetry: Telemetry,
    perf_telemetry: PerfTelemetry,
    effects: Arc<WorkspaceStreamEffects>,
}

pub(in crate::daemon) struct WorkspaceStreamHandleParts {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    session_stores: SessionStoreLookup,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    sessions: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    lifecycle_host: Arc<WorkspaceStreamSessionLifecycleHost>,
    telemetry: Telemetry,
    perf_telemetry: PerfTelemetry,
    effects: Arc<WorkspaceStreamEffects>,
}

impl WorkspaceStreamHandle {
    pub(in crate::daemon) fn new(parts: WorkspaceStreamHandleParts) -> Self {
        Self {
            global_store: parts.global_store,
            workspace_stores: parts.workspace_stores,
            session_stores: parts.session_stores,
            active_snapshot: parts.active_snapshot,
            sessions: parts.sessions,
            lifecycle_host: parts.lifecycle_host,
            telemetry: parts.telemetry,
            perf_telemetry: parts.perf_telemetry,
            effects: parts.effects,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn session_store_allow_archived(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores
            .existing_session_store_allow_archived(session_id)
            .await
    }

    pub(in crate::daemon) fn active_snapshot(&self) -> &WorkspaceActiveSnapshotHub {
        &self.active_snapshot
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) async fn attach_workspace_stream_session_pin(
        &self,
        session_id: SessionId,
    ) {
        self.sessions
            .attach_session_with_host(self.lifecycle_host.as_ref(), session_id)
            .await;
    }

    pub(in crate::daemon) async fn detach_workspace_stream_session_pin(
        &self,
        session_id: SessionId,
    ) {
        self.sessions
            .detach_session_with_host(self.lifecycle_host.as_ref(), session_id)
            .await;
    }

    pub(in crate::daemon) async fn ensure_workspace_active_snapshot_hydrated(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), crate::daemon::workspaces::WorkspaceHydrationError> {
        (self.effects.ensure_workspace_active_snapshot_hydrated)(workspace_id).await
    }

    pub(in crate::daemon) async fn activate_workspace_merge_queue(
        &self,
        workspace_id: WorkspaceId,
    ) {
        (self.effects.activate_workspace_merge_queue)(workspace_id).await
    }
}

pub(in crate::daemon) type WorkspaceVcsStreamRefreshFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
pub(in crate::daemon) type WorkspaceVcsStreamRefreshEffect =
    Arc<dyn Fn(Worktree, bool, bool) -> WorkspaceVcsStreamRefreshFuture + Send + Sync>;
pub(in crate::daemon) type WorkspaceVcsStreamWatcherFuture =
    Pin<Box<dyn Future<Output = ()> + Send>>;
pub(in crate::daemon) type WorkspaceVcsStreamWatcherEffect =
    Arc<dyn Fn(Worktree) -> WorkspaceVcsStreamWatcherFuture + Send + Sync>;

#[derive(Clone)]
pub struct WorkspaceVcsStreamHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    runtime: crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime,
    perf_telemetry: PerfTelemetry,
    ensure_worktree_vcs_watcher: WorkspaceVcsStreamWatcherEffect,
    refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
}

impl WorkspaceVcsStreamHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        runtime: crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime,
        perf_telemetry: PerfTelemetry,
        ensure_worktree_vcs_watcher: WorkspaceVcsStreamWatcherEffect,
        refresh_worktree_vcs: WorkspaceVcsStreamRefreshEffect,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            runtime,
            perf_telemetry,
            ensure_worktree_vcs_watcher,
            refresh_worktree_vcs,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn store_for_worktree(
        &self,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores.store_for_worktree(worktree_id).await
    }

    pub(in crate::daemon) fn runtime(
        &self,
    ) -> &crate::daemon::workspaces::stream::WorkspaceVcsStreamRuntime {
        &self.runtime
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) async fn ensure_loaded_worktree_vcs_watcher(&self, worktree: Worktree) {
        (self.ensure_worktree_vcs_watcher)(worktree).await
    }

    pub(in crate::daemon) async fn refresh_loaded_worktree_vcs(
        &self,
        worktree: Worktree,
        summary: bool,
        touched_files: bool,
    ) -> anyhow::Result<()> {
        (self.refresh_worktree_vcs)(worktree, summary, touched_files).await
    }
}

pub(in crate::daemon) struct WorkspaceStreamSessionLifecycleHost {
    global_store: Store,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    providers: Arc<ProviderRuntime>,
}

impl WorkspaceStreamSessionLifecycleHost {
    fn new(
        global_store: Store,
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        providers: Arc<ProviderRuntime>,
    ) -> Self {
        Self {
            global_store,
            active_snapshot,
            providers,
        }
    }
}

#[async_trait::async_trait]
impl SessionLifecycleHost for WorkspaceStreamSessionLifecycleHost {
    async fn set_provider_session_pinned(&self, session_id: SessionId, pinned: bool) {
        self.providers
            .set_provider_session_pinned(session_id.0.to_string(), pinned)
            .await;
    }

    async fn remove_workspace_active_session(&self, session_id: SessionId) {
        let workspace_id = self
            .global_store
            .get_workspace_id_for_session(session_id)
            .await
            .ok()
            .flatten();
        if let Some(workspace_id) = workspace_id {
            self.active_snapshot
                .remove_session_with_workspace_hint(workspace_id, session_id)
                .await;
        } else {
            self.active_snapshot.remove_session(session_id).await;
        }
    }
}
