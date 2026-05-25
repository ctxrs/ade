use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use anyhow::Context;
use ctx_core::ids::{SessionId, TaskId, TurnId, WorkspaceId, WorktreeId};
use ctx_core::models::{
    ExecutionEnvironment, Message, SandboxBinding, Session, SessionEvent, SessionEventType,
    SessionHeadDelta, SessionSummary, SessionSummaryDelta, SessionTurn, SessionTurnToolSummary,
    SubagentInvocation, Task, TaskDeltaKind, TerminalSession, Workspace, WorkspaceActiveHeadBatch,
    WorkspaceActiveSnapshot, Worktree, WorktreeVcsSnapshot,
};
use ctx_execution_runtime::ExecutionSetupCoordinator;
use ctx_mcp_auth::McpAuthRegistry;
use ctx_merge_queue::MergeQueueRuntime;
use ctx_observability::ops_events::{OpsEvent, OpsEvents};
use ctx_observability::perf_telemetry::{PerfMetric, PerfMetricKind, PerfTelemetry};
use ctx_observability::telemetry::Telemetry;
use ctx_provider_install::install_state::{
    InstallId, InstallInfo, InstallProgressEvent, InstallTarget,
};
use ctx_provider_runtime::{
    provider_install_tracker::ProviderInstallOpsEvent, ProviderRuntime, ProviderRuntimeHost,
};
use ctx_resource_utilization::resource_governance::ResourceGovernanceRuntime;
use ctx_resource_utilization::ResourceSampler;
use ctx_session_runtime::runtime::{
    SessionEventPublicationHost, SessionLifecycleHost, SessionReplayCursor, SessionRuntime,
    SessionTaskDeltaRefreshHost,
};
use ctx_session_tools::model_resolution::ModelCatalog;
use ctx_session_vcs_service::vcs::SessionVcsDiffBaseQuery;
use ctx_settings_model::ExecutionSettings;
use ctx_storage_admission::{StorageGuardRuntime, StorageGuardStatus};
use ctx_store::{Store, StoreManager};
use ctx_transport_runtime::mobile_tunnel::MobileTunnelManager;
use ctx_transport_runtime::terminal_launch::TerminalLaunchError;
use ctx_transport_runtime::terminals::TerminalManager;
use ctx_transport_runtime::web_sessions::{WebSessionInfo, WebSessionManager};
use ctx_update_service::UpdateDrainCoordinator;
use ctx_workspace_active_snapshot::WorkspaceActiveSnapshotHub;
use ctx_workspace_runtime::HarnessRuntimeManager;
use ctx_worktree_vcs_service::{
    GitStatusSnapshot, WorktreeDiffBaseResolution, WorktreeVcsCommitLookupSource,
    WorktreeVcsDiffBaseQuery, WorktreeVcsDiffSummaryCounts,
};
use tokio::sync::{broadcast, mpsc, Mutex};

use crate::daemon::sessions::title_generation::{
    TitleGenerationLocalHandle, TitleGenerationLocalInstallEffect,
};
use crate::daemon::sessions::{
    subagents::{
        SessionSubagentMcpControlFuture, SessionSubagentMcpControlHandle,
        SessionSubagentMcpControlHandleParts, SessionSubagentMcpControlLifecycleHost,
        SessionSubagentMcpControlPublicationHost, SessionSubagentMcpControlSchedulerSpawner,
        SubagentSpawnHost,
    },
    DemoSeedTranscriptHandle,
};

use super::{
    blobs::BlobHandle,
    route_capabilities::{DaemonRouteHandles, DaemonShutdownSignal},
    state::{
        session_store_access_anyhow, DaemonState, ProtectedWorkspaceStoreLookup,
        SessionStoreLookup, TaskStoreLookup, TelemetryRuntime, WorkspaceFileCompletionsCache,
        WorktreeFileCompletionsCache,
    },
    terminals::CreateTerminalLaunchRequest,
    web_sessions::{WebSessionLaunchError, WebSessionLaunchRequest},
};

#[derive(Clone)]
pub struct DaemonHandle {
    state: Arc<DaemonState>,
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

    pub fn route_handles(&self) -> DaemonRouteHandles {
        DaemonRouteHandles::from_handle(self)
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
        WorkspaceDeletionHandle::new(crate::daemon::workspaces::deletion_runtime_from_state(
            &self.state,
        ))
    }

    pub fn workspace_merge_queue_config(&self) -> WorkspaceMergeQueueConfigHandle {
        WorkspaceMergeQueueConfigHandle::new(
            self.protected_workspace_store_lookup(),
            Arc::clone(&self.state.transport.merge_queue),
        )
    }

    pub fn merge_queue_api(&self) -> MergeQueueApiHandle {
        let workspace_stores = self.protected_workspace_store_lookup();
        let session_stores =
            SessionStoreLookup::new(self.state.global_store().clone(), workspace_stores.clone());
        let publish_merge_queue_notice = Arc::new({
            let state = Arc::clone(&self.state);
            move |notice_event: MergeQueueNoticeSessionEvent| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state.publish_event(notice_event.into_event()).await;
                    Ok(())
                }) as MergeQueueNoticePublicationFuture
            }
        });
        MergeQueueApiHandle::new(Arc::new(
            crate::daemon::merge_queue::MergeQueueRouteHost::new(
                self.state.core.stores.clone(),
                self.state.global_store().clone(),
                workspace_stores,
                session_stores,
                Arc::clone(&self.state.transport.merge_queue),
                self.state.telemetry.ops_events.clone(),
                publish_merge_queue_notice,
            ),
        ))
    }

    pub fn workspace_attachments(&self) -> WorkspaceAttachmentsHandle {
        WorkspaceAttachmentsHandle::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
            crate::daemon::workspaces::attachments::runtime_from_state(&self.state),
        )
    }

    pub fn workspace_primary_branch(&self) -> WorkspacePrimaryBranchHandle {
        let refresh_vcs_snapshot = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::git_status::emit_worktree_vcs_snapshot_for_worktree(
                        &state, &worktree, true,
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
        let cancel_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |session_id: SessionId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::sessions::command_dispatch::cancel_session(&state, session_id)
                        .await
                }) as SessionControlFuture<_>
            }
        });
        let interrupt_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |session_id: SessionId, request_started: std::time::Instant| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::sessions::command_dispatch::interrupt_session(
                        &state,
                        session_id,
                        request_started,
                    )
                    .await
                }) as SessionControlFuture<_>
            }
        });
        let authenticate_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |session_id: SessionId, method_id: Option<String>| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let store = state
                        .existing_session_store_for_write(session_id)
                        .await
                        .map_err(session_store_access_auth_error)?;
                    let session = store
                        .get_session(session_id)
                        .await
                        .map_err(|_| {
                            crate::daemon::sessions::auth::SessionAuthError::Internal(
                                "failed to load session".to_string(),
                            )
                        })?
                        .ok_or(crate::daemon::sessions::auth::SessionAuthError::NotFound(
                            "session",
                        ))?;
                    crate::daemon::sessions::auth::run_session_authentication(
                        &state, &store, &session, method_id,
                    )
                    .await
                }) as SessionControlFuture<_>
            }
        });
        let submit_ask_user_answer = Arc::new({
            let state = Arc::clone(&self.state);
            move |session_id: SessionId,
                  submission: crate::daemon::sessions::ask_user::SubmitAskUserAnswer| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::sessions::ask_user::submit_ask_user_answer(
                        &state, session_id, submission,
                    )
                    .await
                }) as SessionControlFuture<_>
            }
        });
        SessionControlHandle::new(SessionControlEffects::new(SessionControlEffectsParts {
            cancel_session,
            interrupt_session,
            authenticate_session,
            submit_ask_user_answer,
        }))
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
            SessionMessageSchedulerSpawner::new(Arc::downgrade(&self.state)),
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
        let spawn_host = Arc::new(SubagentSpawnHost::new(
            Arc::clone(&self.state),
            self.state.global_store().clone(),
            Arc::clone(&self.state.providers),
            self.state.core.data_root.clone(),
        ));
        let archive_worktree_cleanup = Arc::new(
            crate::daemon::sessions::subagents::SubagentArchiveWorktreeCleanupHost::new(
                self.state.core.data_root.clone(),
                self.state.global_store().clone(),
                crate::daemon::workspaces::vcs_hooks::WorkspaceDeletionVcsHookHost::new(
                    self.state.core.data_root.clone(),
                    self.state.core.daemon_url.clone(),
                    self.state.global_store().clone(),
                    self.state.core.stores.clone(),
                    Arc::clone(&self.state.execution.harness),
                ),
            ),
        );
        SessionSubagentMcpControlHandle::new(SessionSubagentMcpControlHandleParts {
            session_stores: self.session_store_lookup(),
            session_runtime: Arc::clone(&self.state.sessions),
            scheduler_spawner: SessionSubagentMcpControlSchedulerSpawner::new(Arc::downgrade(
                &self.state,
            )),
            publish_host: SessionSubagentMcpControlPublicationHost::new(
                self.session_store_lookup(),
                self.protected_workspace_store_lookup(),
                Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            ),
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

    fn session_artifact_effects(&self) -> Arc<SessionArtifactEffects> {
        let state = Arc::clone(&self.state);
        let publish_event = Arc::new(move |event: SessionEvent| {
            let state = Arc::clone(&state);
            Box::pin(async move { state.publish_event(event).await }) as SessionArtifactsFuture<_>
        });
        SessionArtifactEffects::new(publish_event)
    }

    pub fn session_artifacts(&self) -> SessionArtifactsHandle {
        SessionArtifactsHandle::new(
            self.session_store_lookup(),
            self.state.core.tool_output_spool_dir.clone(),
            self.session_artifact_effects(),
        )
    }

    fn session_vcs_effects(&self) -> Arc<SessionVcsEffects> {
        let worktree_has_vcs_repo = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::git_status::worktree_has_vcs_repo(&state, &worktree).await
                }) as SessionVcsFuture<_>
            }
        });
        let load_git_status_snapshot = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree, include_untracked_files: bool, include_entries: bool| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::git_status::load_git_status_snapshot(
                        &state,
                        &worktree,
                        include_untracked_files,
                        include_entries,
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let resolve_worktree_commit = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree, revision: String| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let source =
                        crate::daemon::git_status::HttpWorktreeVcsSource::new(&state, &worktree);
                    source.resolve_commit(&revision).await
                }) as SessionVcsFuture<_>
            }
        });
        let diff_worktree_for_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree, base_commit_sha: String| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::diff_worktree_for_session(
                        &state,
                        &worktree,
                        &base_commit_sha,
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let diff_worktree_summary_for_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree, base_commit_sha: String| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::diff_worktree_summary_for_session(
                        &state,
                        &worktree,
                        &base_commit_sha,
                    )
                    .await
                }) as SessionVcsFuture<_>
            }
        });
        let resolve_worktree_diff_base = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree, query: SessionVcsDiffBaseQuery| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let source =
                        crate::daemon::git_status::HttpWorktreeVcsSource::new(&state, &worktree);
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
            let state = Arc::clone(&self.state);
            move |worktree_id: WorktreeId| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.get_worktree_vcs_snapshot(worktree_id).await })
                    as SessionVcsFuture<_>
            }
        });
        let emit_compat_payload_reject_counter = Arc::new({
            let state = Arc::clone(&self.state);
            move |surface: &'static str, issue: &'static str| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state
                        .emit_compat_payload_reject_counter(surface, issue, None)
                        .await;
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

    fn task_lifecycle_workspace_runtime(&self) -> Arc<TaskLifecycleWorkspaceRuntime> {
        let state = Arc::clone(&self.state);
        let cleanup_task_worktrees = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace,
                  task_id: TaskId,
                  targets: Vec<crate::daemon::workspaces::TaskWorktreeCleanupTarget>,
                  mode: crate::daemon::workspaces::BranchCleanupErrorMode| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::cleanup_task_worktrees(
                        state.as_ref(),
                        &workspace,
                        task_id,
                        &targets,
                        mode,
                    )
                    .await
                }) as TaskLifecycleFuture<_>
            }
        });
        let rematerialize_sandbox_binding_for_worktree = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace, worktree: Worktree, binding: SandboxBinding| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::rematerialize_sandbox_binding_for_worktree(
                        state.as_ref(),
                        &workspace,
                        &worktree,
                        &binding,
                    )
                    .await
                }) as TaskLifecycleFuture<_>
            }
        });
        let ensure_worktree_attachment_mounts_if_materialized = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace, worktree: Worktree| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::ensure_worktree_attachment_mounts_if_materialized(
                        &state, &workspace, &worktree,
                    )
                    .await
                    .map(|_| ())
                }) as TaskLifecycleFuture<_>
            }
        });
        let spawn_worktree_bootstrap = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace, worktree: Worktree| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::spawn_worktree_bootstrap(state, workspace, worktree)
                        .await
                }) as TaskLifecycleFuture<_>
            }
        });
        let ensure_task_commit_hook = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace, worktree: Worktree, task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::vcs_hooks::ensure_task_commit_hook(
                        state.as_ref(),
                        &workspace,
                        &worktree,
                        task_id,
                    )
                    .await
                }) as TaskLifecycleFuture<_>
            }
        });
        TaskLifecycleWorkspaceRuntime::new(
            self.state.core.data_root.clone(),
            cleanup_task_worktrees,
            rematerialize_sandbox_binding_for_worktree,
            ensure_worktree_attachment_mounts_if_materialized,
            spawn_worktree_bootstrap,
            ensure_task_commit_hook,
        )
    }

    fn task_lifecycle_effects(&self) -> Arc<TaskLifecycleEffects> {
        let state = Arc::clone(&self.state);
        let cleanup_session = Arc::new({
            let state = Arc::clone(&state);
            move |session_id: SessionId| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.cleanup_session(session_id).await })
                    as TaskLifecycleFuture<_>
            }
        });
        let emit_workspace_task_delta = Arc::new({
            let state = Arc::clone(&state);
            move |task: Task, kind: TaskDeltaKind| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let _ = state.emit_workspace_task_delta(task, kind).await;
                }) as TaskLifecycleFuture<_>
            }
        });
        let emit_workspace_task_upsert = Arc::new({
            let state = Arc::clone(&state);
            move |task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.emit_workspace_task_upsert(task_id).await })
                    as TaskLifecycleFuture<_>
            }
        });
        let remove_active_snapshot_session = Arc::new({
            let state = Arc::clone(&state);
            move |session_id: SessionId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state
                        .workspaces
                        .workspace_active_snapshot
                        .remove_session(session_id)
                        .await;
                }) as TaskLifecycleFuture<_>
            }
        });
        let refresh_session_head_cache = Arc::new({
            let state = Arc::clone(&state);
            move |session_id: SessionId| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.refresh_session_head_cache(session_id).await })
                    as TaskLifecycleFuture<_>
            }
        });
        let emit_workspace_archived_task_delete = Arc::new({
            let state = Arc::clone(&state);
            move |workspace_id: WorkspaceId, task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state
                        .emit_workspace_archived_task_delete(workspace_id, task_id)
                        .await;
                }) as TaskLifecycleFuture<_>
            }
        });
        let emit_workspace_task_delete = Arc::new({
            let state = Arc::clone(&state);
            move |workspace_id: WorkspaceId, task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state
                        .emit_workspace_task_delete(workspace_id, task_id)
                        .await;
                }) as TaskLifecycleFuture<_>
            }
        });
        TaskLifecycleEffects::new(
            cleanup_session,
            emit_workspace_task_delta,
            emit_workspace_task_upsert,
            remove_active_snapshot_session,
            refresh_session_head_cache,
            emit_workspace_archived_task_delete,
            emit_workspace_task_delete,
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
            self.task_lifecycle_workspace_runtime(),
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

    fn task_store_lookup(&self) -> TaskStoreLookup {
        TaskStoreLookup::new(
            self.state.global_store().clone(),
            self.protected_workspace_store_lookup(),
        )
    }

    fn task_metadata_effects(&self) -> Arc<TaskMetadataEffects> {
        let state = Arc::clone(&self.state);
        let emit_workspace_task_delta = Arc::new({
            let state = Arc::clone(&state);
            move |task: Task, kind: TaskDeltaKind| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    let _ = state.emit_workspace_task_delta(task, kind).await;
                }) as TaskMetadataFuture<_>
            }
        });
        let emit_workspace_task_upsert = Arc::new({
            let state = Arc::clone(&state);
            move |task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.emit_workspace_task_upsert(task_id).await })
                    as TaskMetadataFuture<_>
            }
        });
        TaskMetadataEffects::new(emit_workspace_task_delta, emit_workspace_task_upsert)
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

    fn task_session_admission_workspace_runtime(&self) -> Arc<TaskAdmissionWorkspaceRuntime> {
        let state = Arc::clone(&self.state);
        let resolve_existing_worktree_execution = Arc::new({
            let state = Arc::clone(&state);
            move |store: Store, workspace: Workspace, worktree_id: WorktreeId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::resolve_existing_worktree_execution(
                        &state,
                        &store,
                        &workspace,
                        worktree_id,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            }
        });
        let provision_worktree_for_execution = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace,
                  worktree_id: WorktreeId,
                  base_commit_sha: String,
                  branch_name: String,
                  effective: ExecutionSettings| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::provision_worktree_for_execution(
                        &state,
                        &workspace,
                        worktree_id,
                        &base_commit_sha,
                        &branch_name,
                        &effective,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            }
        });
        let persist_provisioned_worktree = Arc::new({
            let state = Arc::clone(&state);
            move |store: Store,
                  workspace: Workspace,
                  worktree: Worktree,
                  sandbox_binding: Option<SandboxBinding>| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::persist_provisioned_worktree(
                        &state,
                        &store,
                        &workspace,
                        worktree,
                        sandbox_binding,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            }
        });
        let cleanup_task_worktrees = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace,
                  task_id: TaskId,
                  targets: Vec<crate::daemon::workspaces::TaskWorktreeCleanupTarget>,
                  mode: crate::daemon::workspaces::BranchCleanupErrorMode| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::cleanup_task_worktrees(
                        state.as_ref(),
                        &workspace,
                        task_id,
                        &targets,
                        mode,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            }
        });
        let ensure_task_commit_hook = Arc::new({
            let state = Arc::clone(&state);
            move |workspace: Workspace, worktree: Worktree, task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::workspaces::vcs_hooks::ensure_task_commit_hook(
                        state.as_ref(),
                        &workspace,
                        &worktree,
                        task_id,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            }
        });
        let emit_workspace_task_upsert = Arc::new({
            let state = Arc::clone(&state);
            move |task_id: TaskId| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.emit_workspace_task_upsert(task_id).await })
                    as TaskAdmissionFuture<_>
            }
        });
        TaskAdmissionWorkspaceRuntime::new(
            self.state.core.data_root.clone(),
            resolve_existing_worktree_execution,
            provision_worktree_for_execution,
            persist_provisioned_worktree,
            cleanup_task_worktrees,
            ensure_task_commit_hook,
            emit_workspace_task_upsert,
        )
    }

    fn task_session_admission_effects(&self) -> Arc<TaskAdmissionSessionEffects> {
        let state = Arc::clone(&self.state);
        let publish_event = Arc::new({
            let state = Arc::clone(&state);
            move |event| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.publish_event(event).await }) as TaskAdmissionFuture<_>
            }
        });
        let ensure_scheduler = Arc::new({
            let state = Arc::clone(&state);
            move |session: Session| {
                let state = Arc::clone(&state);
                Box::pin(async move { state.ensure_scheduler(session).await })
                    as TaskAdmissionFuture<_>
            }
        });
        let schedule_title_generation = Arc::new({
            let state = Arc::clone(&state);
            move |session: Session, prompt: String, force: bool| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::sessions::title_generation::schedule_session_title_generation(
                        state, session, prompt, force,
                    )
                    .await
                }) as TaskAdmissionFuture<_>
            }
        });
        TaskAdmissionSessionEffects::new(publish_event, ensure_scheduler, schedule_title_generation)
    }

    fn task_session_admission_provider_status(&self) -> ProviderStatusHandle {
        ProviderStatusHandle::new(
            self.state.core.data_root.clone(),
            Arc::clone(&self.state.providers),
            self.state.telemetry.ops_events.clone(),
        )
    }

    fn task_session_admission_model_catalog_loader(&self) -> TaskAdmissionModelCatalogLoader {
        let state = Arc::clone(&self.state);
        Arc::new(
            move |workspace: Workspace,
                  provider_id: String,
                  execution_environment: ExecutionEnvironment| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment(
                        state.as_ref(),
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
            self.task_session_admission_workspace_runtime(),
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
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace_id: WorkspaceId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state
                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                        .await
                }) as WorkspaceActiveFuture<_>
            }
        });
        let activate_workspace_merge_queue = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace_id: WorkspaceId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::merge_queue::activate_workspace_merge_queue(
                        &state,
                        workspace_id,
                    )
                    .await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        let cache_workspace_active_snapshot = Arc::new({
            let state = Arc::clone(&self.state);
            move |snapshot: WorkspaceActiveSnapshot| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state.cache_workspace_active_snapshot(snapshot).await;
                }) as WorkspaceActiveFuture<_>
            }
        });
        let cache_workspace_active_heads = Arc::new({
            let state = Arc::clone(&self.state);
            move |heads: WorkspaceActiveHeadBatch| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state.cache_workspace_active_heads(heads).await;
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
        let lifecycle_host = Arc::new(WorkspaceStreamSessionLifecycleHost::new(
            self.state.global_store().clone(),
            Arc::clone(&self.state.workspaces.workspace_active_snapshot),
            Arc::clone(&self.state.providers),
        ));
        let ensure_workspace_active_snapshot_hydrated = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace_id: WorkspaceId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state
                        .ensure_workspace_active_snapshot_hydrated(workspace_id)
                        .await
                }) as WorkspaceStreamFuture<_>
            }
        });
        let activate_workspace_merge_queue = Arc::new({
            let state = Arc::clone(&self.state);
            move |workspace_id: WorkspaceId| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::merge_queue::activate_workspace_merge_queue(
                        &state,
                        workspace_id,
                    )
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
        let ensure_worktree_vcs_watcher = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state.ensure_git_status_watcher(worktree).await;
                }) as WorkspaceVcsStreamWatcherFuture
            }
        });
        let refresh_worktree_vcs = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree, summary: bool, touched_files: bool| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state.ensure_git_status_watcher(worktree.clone()).await;
                    crate::daemon::git_status::request_worktree_vcs_refresh_without_transient(
                        &state,
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
        let ensure_worktree_vcs_watcher = Arc::new({
            let state = Arc::clone(&self.state);
            move |worktree: Worktree| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    state.ensure_git_status_watcher(worktree).await;
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
        let create_terminal = Arc::new({
            let state = Arc::clone(&self.state);
            move |req: CreateTerminalLaunchRequest| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::terminals::create_workspace_terminal(&state, req).await
                }) as CreateTerminalFuture
            }
        });
        TerminalRouteHandle::new(Arc::clone(&self.state.transport.terminals), create_terminal)
    }

    pub fn web_session_route(&self) -> WebSessionRouteHandle {
        let create_web_session = Arc::new({
            let state = Arc::clone(&self.state);
            move |req: WebSessionLaunchRequest| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::web_sessions::create_web_session(&state, req).await
                }) as CreateWebSessionFuture
            }
        });
        WebSessionRouteHandle::new(
            Arc::clone(&self.state.transport.web_sessions),
            create_web_session,
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
        let request_shutdown = Arc::new({
            let state = Arc::clone(&self.state);
            move |reason: String| {
                let state = Arc::clone(&state);
                Box::pin(async move {
                    crate::daemon::maintenance::request_daemon_shutdown(state, reason).await
                }) as DaemonShutdownFuture
            }
        });
        DaemonShutdownHandle::new(
            self.state.core.local_shutdown_token.clone(),
            request_shutdown,
        )
    }
}

impl TelemetryHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf, runtime: &TelemetryRuntime) -> Self {
        Self {
            data_root,
            perf_telemetry: runtime.perf_telemetry.clone(),
            telemetry: runtime.telemetry.clone(),
        }
    }

    pub fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub async fn read_perf_telemetry_export_for_date(
        &self,
        date: &str,
    ) -> Result<Vec<u8>, ctx_route_contracts::telemetry::TelemetryExportError> {
        let path = ctx_observability::perf_telemetry::perf_log_path_for_date(&self.data_root, date);
        tokio::fs::read(&path)
            .await
            .map_err(|_| ctx_route_contracts::telemetry::TelemetryExportError::not_found())
    }
}

#[derive(Clone)]
pub struct TelemetryHandle {
    data_root: PathBuf,
    perf_telemetry: PerfTelemetry,
    telemetry: Telemetry,
}

#[derive(Clone)]
pub struct AuthHandle {
    auth_token: Option<String>,
    mcp_auth: Arc<McpAuthRegistry>,
    store: Store,
    ops_events: OpsEvents,
}

impl AuthHandle {
    pub(in crate::daemon) fn new(
        auth_token: Option<String>,
        mcp_auth: Arc<McpAuthRegistry>,
        store: Store,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            auth_token,
            mcp_auth,
            store,
            ops_events,
        }
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    pub fn has_auth_token(&self) -> bool {
        self.auth_token.is_some()
    }

    pub async fn verify_mcp_auth_token(&self, token: &str) -> Option<ctx_mcp_auth::McpAuthContext> {
        self.mcp_auth.verify_token(token).await
    }

    pub fn emit_mcp_token_denied(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        method: &str,
        path: &str,
        reason: &str,
    ) {
        let mut event = OpsEvent::new("warn", "mcp_token_denied");
        event.session_id = Some(mcp_auth.session_id.0.to_string());
        event.worktree_id = Some(mcp_auth.worktree_id.0.to_string());
        event.meta = Some(serde_json::json!({
            "workspace_id": mcp_auth.workspace_id.0.to_string(),
            "capabilities": mcp_auth.capabilities.names(),
            "detail": {
                "method": method,
                "path": path,
                "reason": reason,
            },
        }));
        self.ops_events.emit(event);
    }

    pub async fn verify_mobile_api_token_hash(
        &self,
        hash: &str,
    ) -> Result<
        Option<ctx_mobile_access_service::MobileAuthContext>,
        ctx_mobile_access_service::MobileAuthContextError,
    > {
        ctx_mobile_access_service::verify_mobile_api_token_hash(&self.store, hash).await
    }
}

#[derive(Clone)]
pub struct HealthHandle {
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    storage_guard: Arc<StorageGuardRuntime>,
}

impl HealthHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        daemon_url: String,
        auth_token: Option<String>,
        storage_guard: Arc<StorageGuardRuntime>,
    ) -> Self {
        Self {
            data_root,
            daemon_url,
            auth_token,
            storage_guard,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub fn auth_token(&self) -> Option<&str> {
        self.auth_token.as_deref()
    }

    pub(in crate::daemon) fn auth_required(&self) -> bool {
        self.auth_token.is_some()
    }

    pub(in crate::daemon) fn storage_guard_snapshot(&self) -> StorageGuardStatus {
        self.storage_guard.snapshot()
    }
}

#[derive(Clone)]
pub struct DiagnosticsHandle {
    health: HealthHandle,
    data_root: PathBuf,
    execution_setup: Arc<ExecutionSetupCoordinator>,
    providers: Arc<ProviderRuntime>,
}

impl DiagnosticsHandle {
    pub(in crate::daemon) fn new(
        health: HealthHandle,
        data_root: PathBuf,
        execution_setup: Arc<ExecutionSetupCoordinator>,
        providers: Arc<ProviderRuntime>,
    ) -> Self {
        Self {
            health,
            data_root,
            execution_setup,
            providers,
        }
    }

    pub(in crate::daemon) fn health(&self) -> &HealthHandle {
        &self.health
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn execution_setup(&self) -> &ExecutionSetupCoordinator {
        &self.execution_setup
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        &self.providers
    }
}

#[derive(Clone)]
pub struct RequestBaseHandle {
    daemon_url: String,
    public_base_url: Option<String>,
}

impl RequestBaseHandle {
    pub(in crate::daemon) fn new(daemon_url: String, public_base_url: Option<String>) -> Self {
        Self {
            daemon_url,
            public_base_url,
        }
    }

    pub fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub fn public_base_url(&self) -> Option<&str> {
        self.public_base_url.as_deref()
    }
}

#[derive(Clone)]
pub struct LogsHandle {
    data_root: PathBuf,
}

impl LogsHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Clone)]
pub struct OrgPolicyHandle {
    store: Store,
}

impl OrgPolicyHandle {
    pub(in crate::daemon) fn new(store: Store) -> Self {
        Self { store }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }
}

#[derive(Clone)]
pub struct WorkspaceOrgPolicyHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
}

impl WorkspaceOrgPolicyHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
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
pub struct WorkspacePromptBootstrapConfigHandle {
    workspace_stores: ProtectedWorkspaceStoreLookup,
}

impl WorkspacePromptBootstrapConfigHandle {
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
pub struct WorkspaceFileCompletionsHandle {
    global_store: Store,
    workspace_file_completions_cache: WorkspaceFileCompletionsCache,
    perf_telemetry: PerfTelemetry,
}

impl WorkspaceFileCompletionsHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_file_completions_cache: WorkspaceFileCompletionsCache,
        perf_telemetry: PerfTelemetry,
    ) -> Self {
        Self {
            global_store,
            workspace_file_completions_cache,
            perf_telemetry,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn workspace_file_completions_cache(
        &self,
    ) -> &WorkspaceFileCompletionsCache {
        &self.workspace_file_completions_cache
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }
}

#[derive(Clone)]
pub struct WorkspaceRegistryHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    telemetry: Telemetry,
}

impl WorkspaceRegistryHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        telemetry: Telemetry,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            telemetry,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }
}

#[derive(Clone)]
pub struct WorkspaceDeletionHandle {
    runtime: Arc<crate::daemon::workspaces::WorkspaceDeletionRuntime>,
}

impl WorkspaceDeletionHandle {
    pub(in crate::daemon) fn new(
        runtime: Arc<crate::daemon::workspaces::WorkspaceDeletionRuntime>,
    ) -> Self {
        Self { runtime }
    }

    pub(in crate::daemon) async fn delete_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<(), crate::daemon::workspaces::WorkspaceDeleteError> {
        self.runtime.delete_workspace(workspace_id).await
    }

    #[cfg(test)]
    pub(in crate::daemon) fn fail_next_delete_after_begin_for_test(&self) {
        self.runtime.fail_next_delete_after_begin_for_test();
    }
}

#[derive(Clone)]
pub struct WorkspaceMergeQueueConfigHandle {
    workspace_stores: ProtectedWorkspaceStoreLookup,
    merge_queue: Arc<MergeQueueRuntime>,
}

impl WorkspaceMergeQueueConfigHandle {
    pub(in crate::daemon) fn new(
        workspace_stores: ProtectedWorkspaceStoreLookup,
        merge_queue: Arc<MergeQueueRuntime>,
    ) -> Self {
        Self {
            workspace_stores,
            merge_queue,
        }
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn schedule_store_if_enabled_and_queued(
        &self,
        store: &Store,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<bool> {
        ctx_merge_queue::schedule_store_if_enabled_and_queued(
            self.merge_queue.as_ref(),
            store,
            workspace_id,
        )
        .await
    }

    pub(in crate::daemon) async fn cancel_store_queued_entries_for_disabled_workspace(
        &self,
        store: &Store,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<()> {
        ctx_merge_queue::cancel_store_queued_entries_for_disabled_workspace(
            self.merge_queue.as_ref(),
            store,
            workspace_id,
        )
        .await
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
pub struct WorkspaceAttachmentsHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    runtime: Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime>,
}

impl WorkspaceAttachmentsHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        runtime: Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime>,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            runtime,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) fn runtime(
        &self,
    ) -> &Arc<crate::daemon::workspaces::attachments::WorkspaceAttachmentsRuntime> {
        &self.runtime
    }
}

pub(in crate::daemon) type WorkspacePrimaryBranchRefreshFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;
pub(in crate::daemon) type WorkspacePrimaryBranchRefreshEffect =
    Arc<dyn Fn(Worktree) -> WorkspacePrimaryBranchRefreshFuture + Send + Sync>;

#[derive(Clone)]
pub struct WorkspacePrimaryBranchHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
}

impl WorkspacePrimaryBranchHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        refresh_vcs_snapshot: WorkspacePrimaryBranchRefreshEffect,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            refresh_vcs_snapshot,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn refresh_vcs_snapshot(
        &self,
        worktree: Worktree,
    ) -> anyhow::Result<()> {
        (self.refresh_vcs_snapshot)(worktree).await
    }
}

#[derive(Clone)]
pub struct WorkspaceExecutionConfigHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    data_root: PathBuf,
}

impl WorkspaceExecutionConfigHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        data_root: PathBuf,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            data_root,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
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
pub struct WorkspaceHarnessContainerHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    daemon_url: String,
    harness: Arc<HarnessRuntimeManager>,
}

impl WorkspaceHarnessContainerHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        daemon_url: String,
        harness: Arc<HarnessRuntimeManager>,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            daemon_url,
            harness,
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

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }
}

#[derive(Clone)]
pub struct WorkspaceWorktreeHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    data_root: PathBuf,
}

impl WorkspaceWorktreeHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        data_root: PathBuf,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            data_root,
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

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }
}

#[derive(Clone)]
pub struct WorkspaceProviderModelPreferenceHandle {
    launch: Arc<ProviderWorkspaceLaunchRuntime>,
}

impl WorkspaceProviderModelPreferenceHandle {
    pub(in crate::daemon) fn new(launch: Arc<ProviderWorkspaceLaunchRuntime>) -> Self {
        Self { launch }
    }

    pub(in crate::daemon) fn launch(&self) -> &ProviderWorkspaceLaunchRuntime {
        self.launch.as_ref()
    }
}

#[derive(Clone)]
pub struct DictationHandle {
    store: Store,
}

impl DictationHandle {
    pub(in crate::daemon) fn new(store: Store) -> Self {
        Self { store }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
    }
}

#[derive(Clone)]
pub struct UpdateReleaseHandle {
    data_root: PathBuf,
}

impl UpdateReleaseHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
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
pub struct MobileStoreHandle {
    store: Store,
}

impl MobileStoreHandle {
    pub(in crate::daemon) fn new(store: Store) -> Self {
        Self { store }
    }

    pub(in crate::daemon) fn store(&self) -> &Store {
        &self.store
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
pub struct RepoOnboardingHandle {
    data_root: PathBuf,
}

impl RepoOnboardingHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf) -> Self {
        Self { data_root }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
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
pub struct ProviderAccountsHandle {
    data_root: PathBuf,
    daemon_url: String,
    providers: Arc<ProviderRuntime>,
}

impl ProviderAccountsHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        daemon_url: String,
        providers: Arc<ProviderRuntime>,
    ) -> Self {
        Self {
            data_root,
            daemon_url,
            providers,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn providers_arc(&self) -> Arc<ProviderRuntime> {
        Arc::clone(&self.providers)
    }
}

type TaskAdmissionFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type TaskLifecycleFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type SessionControlFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type SessionArtifactsFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub(in crate::daemon) type TaskMetadataFuture<T> =
    Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub(in crate::daemon) type TaskArchivedRevLoader =
    Arc<dyn Fn(WorkspaceId) -> TaskMetadataFuture<i64> + Send + Sync>;
pub(in crate::daemon) type TaskCloseWebSessionsForTask = Arc<
    dyn Fn(HashSet<String>, HashSet<String>) -> TaskMetadataFuture<anyhow::Result<usize>>
        + Send
        + Sync,
>;
type TaskAdmissionModelCatalogLoader = Arc<
    dyn Fn(
            Workspace,
            String,
            ExecutionEnvironment,
        ) -> TaskAdmissionFuture<Result<Option<ModelCatalog>, String>>
        + Send
        + Sync,
>;

fn session_store_access_auth_error(
    error: crate::daemon::SessionStoreAccessError,
) -> crate::daemon::sessions::auth::SessionAuthError {
    match error {
        crate::daemon::SessionStoreAccessError::NotFound => {
            crate::daemon::sessions::auth::SessionAuthError::NotFound("session")
        }
        crate::daemon::SessionStoreAccessError::LookupUnavailable(error) => {
            crate::daemon::sessions::auth::SessionAuthError::Internal(error.to_string())
        }
        crate::daemon::SessionStoreAccessError::StoreUnavailable => {
            crate::daemon::sessions::auth::SessionAuthError::Internal(
                "workspace store unavailable".to_string(),
            )
        }
    }
}

type SessionControlCancelEffect = Arc<
    dyn Fn(
            SessionId,
        ) -> SessionControlFuture<
            Result<(), crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError>,
        > + Send
        + Sync,
>;
type SessionControlInterruptEffect = Arc<
    dyn Fn(
            SessionId,
            std::time::Instant,
        ) -> SessionControlFuture<
            Result<(), crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError>,
        > + Send
        + Sync,
>;
type SessionControlAuthEffect = Arc<
    dyn Fn(
            SessionId,
            Option<String>,
        )
            -> SessionControlFuture<Result<(), crate::daemon::sessions::auth::SessionAuthError>>
        + Send
        + Sync,
>;
type SessionControlAskUserEffect = Arc<
    dyn Fn(
            SessionId,
            crate::daemon::sessions::ask_user::SubmitAskUserAnswer,
        ) -> SessionControlFuture<
            Result<(), crate::daemon::sessions::ask_user::SubmitAskUserAnswerError>,
        > + Send
        + Sync,
>;

pub(in crate::daemon) struct SessionControlEffectsParts {
    cancel_session: SessionControlCancelEffect,
    interrupt_session: SessionControlInterruptEffect,
    authenticate_session: SessionControlAuthEffect,
    submit_ask_user_answer: SessionControlAskUserEffect,
}

pub(in crate::daemon) struct SessionControlEffects {
    cancel_session: SessionControlCancelEffect,
    interrupt_session: SessionControlInterruptEffect,
    authenticate_session: SessionControlAuthEffect,
    submit_ask_user_answer: SessionControlAskUserEffect,
}

impl SessionControlEffects {
    pub(in crate::daemon) fn new(parts: SessionControlEffectsParts) -> Arc<Self> {
        Arc::new(Self {
            cancel_session: parts.cancel_session,
            interrupt_session: parts.interrupt_session,
            authenticate_session: parts.authenticate_session,
            submit_ask_user_answer: parts.submit_ask_user_answer,
        })
    }

    pub(in crate::daemon) async fn cancel_session(
        &self,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError> {
        (self.cancel_session)(session_id).await
    }

    pub(in crate::daemon) async fn interrupt_session(
        &self,
        session_id: SessionId,
        request_started: std::time::Instant,
    ) -> Result<(), crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError> {
        (self.interrupt_session)(session_id, request_started).await
    }

    pub(in crate::daemon) async fn authenticate_session(
        &self,
        session_id: SessionId,
        method_id: Option<String>,
    ) -> Result<(), crate::daemon::sessions::auth::SessionAuthError> {
        (self.authenticate_session)(session_id, method_id).await
    }

    pub(in crate::daemon) async fn submit_ask_user_answer(
        &self,
        session_id: SessionId,
        submission: crate::daemon::sessions::ask_user::SubmitAskUserAnswer,
    ) -> Result<(), crate::daemon::sessions::ask_user::SubmitAskUserAnswerError> {
        (self.submit_ask_user_answer)(session_id, submission).await
    }
}

#[derive(Clone)]
pub struct SessionControlHandle {
    effects: Arc<SessionControlEffects>,
}

impl SessionControlHandle {
    pub(in crate::daemon) fn new(effects: Arc<SessionControlEffects>) -> Self {
        Self { effects }
    }

    pub(in crate::daemon) async fn cancel_session(
        &self,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError> {
        self.effects.cancel_session(session_id).await
    }

    pub(in crate::daemon) async fn interrupt_session(
        &self,
        session_id: SessionId,
        request_started: std::time::Instant,
    ) -> Result<(), crate::daemon::sessions::command_dispatch::SessionSchedulerCommandError> {
        self.effects
            .interrupt_session(session_id, request_started)
            .await
    }

    pub(in crate::daemon) async fn authenticate_session(
        &self,
        session_id: SessionId,
        method_id: Option<String>,
    ) -> Result<(), crate::daemon::sessions::auth::SessionAuthError> {
        self.effects
            .authenticate_session(session_id, method_id)
            .await
    }

    pub(in crate::daemon) async fn submit_ask_user_answer(
        &self,
        session_id: SessionId,
        submission: crate::daemon::sessions::ask_user::SubmitAskUserAnswer,
    ) -> Result<(), crate::daemon::sessions::ask_user::SubmitAskUserAnswerError> {
        self.effects
            .submit_ask_user_answer(session_id, submission)
            .await
    }
}

#[derive(Clone)]
pub struct SessionFileCompletionsHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    worktree_file_completions_cache: WorktreeFileCompletionsCache,
    perf_telemetry: PerfTelemetry,
    data_root: PathBuf,
    daemon_url: String,
    harness: Arc<HarnessRuntimeManager>,
}

pub(in crate::daemon) struct SessionFileCompletionsHandleParts {
    global_store: Store,
    session_stores: SessionStoreLookup,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    worktree_file_completions_cache: WorktreeFileCompletionsCache,
    perf_telemetry: PerfTelemetry,
    data_root: PathBuf,
    daemon_url: String,
    harness: Arc<HarnessRuntimeManager>,
}

impl SessionFileCompletionsHandle {
    pub(in crate::daemon) fn new(parts: SessionFileCompletionsHandleParts) -> Self {
        Self {
            global_store: parts.global_store,
            session_stores: parts.session_stores,
            workspace_stores: parts.workspace_stores,
            worktree_file_completions_cache: parts.worktree_file_completions_cache,
            perf_telemetry: parts.perf_telemetry,
            data_root: parts.data_root,
            daemon_url: parts.daemon_url,
            harness: parts.harness,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn existing_session_store(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores.existing_session_store(session_id).await
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) fn worktree_file_completions_cache(
        &self,
    ) -> &WorktreeFileCompletionsCache {
        &self.worktree_file_completions_cache
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn daemon_url(&self) -> &str {
        &self.daemon_url
    }

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }
}

pub(in crate::daemon) struct SessionTitleModelModeHandleParts {
    global_store: Store,
    session_stores: SessionStoreLookup,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    provider_runtime: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    harness: Arc<HarnessRuntimeManager>,
}

#[derive(Clone)]
pub struct SessionTitleModelModeHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    provider_runtime: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
    data_root: PathBuf,
    daemon_url: String,
    auth_token: Option<String>,
    harness: Arc<HarnessRuntimeManager>,
}

impl SessionTitleModelModeHandle {
    pub(in crate::daemon) fn new(parts: SessionTitleModelModeHandleParts) -> Self {
        Self {
            global_store: parts.global_store,
            session_stores: parts.session_stores,
            workspace_stores: parts.workspace_stores,
            session_runtime: parts.session_runtime,
            active_snapshot: parts.active_snapshot,
            provider_runtime: parts.provider_runtime,
            ops_events: parts.ops_events,
            data_root: parts.data_root,
            daemon_url: parts.daemon_url,
            auth_token: parts.auth_token,
            harness: parts.harness,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn active_snapshot(&self) -> &WorkspaceActiveSnapshotHub {
        self.active_snapshot.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.provider_runtime.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
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

    pub(in crate::daemon) fn harness(&self) -> &HarnessRuntimeManager {
        self.harness.as_ref()
    }

    pub(in crate::daemon) async fn get_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Option<Workspace>> {
        self.global_store().get_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores
            .existing_session_store_for_write(session_id)
            .await
    }

    pub(in crate::daemon) async fn session_store_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self.session_stores.existing_session_store(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(crate::daemon::SessionStoreAccessError::LookupUnavailable(error)) => Err(error),
            Err(crate::daemon::SessionStoreAccessError::StoreUnavailable) => {
                anyhow::bail!("workspace store unavailable")
            }
        }
    }

    pub(in crate::daemon) async fn session_store_for_write_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self.existing_session_store_for_write(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(crate::daemon::SessionStoreAccessError::LookupUnavailable(error)) => Err(error),
            Err(crate::daemon::SessionStoreAccessError::StoreUnavailable) => {
                anyhow::bail!("workspace store unavailable")
            }
        }
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn store_for_task(&self, task_id: TaskId) -> anyhow::Result<Store> {
        let workspace_id = self
            .global_store()
            .get_workspace_id_for_task(task_id)
            .await?
            .with_context(|| format!("workspace missing for task {}", task_id.0))?;
        self.store_for_workspace(workspace_id).await
    }

    pub(in crate::daemon) async fn store_for_session(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Store> {
        self.session_store_or_none(session_id)
            .await?
            .with_context(|| format!("workspace missing for session {}", session_id.0))
    }

    pub(in crate::daemon) async fn remember_session_meta(&self, session: &Session) {
        self.session_runtime.remember_session_meta(session).await;
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        let host = SessionTitleModelModePublicationHost::new(self.clone());
        self.session_runtime
            .publish_event_with_host(&host, event)
            .await;
    }

    pub(in crate::daemon) async fn emit_workspace_task_upsert(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        let mut task: Option<Task> = None;
        let store = self.store_for_task(task_id).await?;
        match store.get_workspace_active_task_summary(task_id).await? {
            Some(summary) => {
                let workspace_id = summary.task.workspace_id;
                task = Some(summary.task.clone());
                self.active_snapshot
                    .publish_active_task_upsert(workspace_id, summary)
                    .await;
            }
            None => {
                if let Some(loaded) = store.get_task(task_id).await? {
                    task = Some(loaded.clone());
                    self.active_snapshot
                        .publish_active_task_delete(loaded.workspace_id, task_id)
                        .await;
                }
            }
        }

        if let Some(task) = task.as_ref().filter(|task| task.archived_at.is_some()) {
            self.emit_workspace_archived_task_upsert(task).await?;
        }
        Ok(())
    }

    async fn emit_workspace_archived_task_upsert(&self, task: &Task) -> anyhow::Result<()> {
        let store = self.store_for_task(task.id).await?;
        let Some(summary) = store.get_workspace_task_summary(task.id).await? else {
            return Ok(());
        };
        if summary.task.archived_at.is_none() {
            return Ok(());
        }

        let _ = store
            .bump_workspace_archived_snapshot_rev(task.workspace_id)
            .await?;
        self.active_snapshot
            .publish_archived_task_upsert(task.workspace_id, summary)
            .await;
        Ok(())
    }

    pub(in crate::daemon) async fn load_provider_model_catalog_for_execution_environment(
        &self,
        workspace: &Workspace,
        provider_id: &str,
        execution_environment: ExecutionEnvironment,
    ) -> Result<Option<ctx_session_tools::model_resolution::ModelCatalog>, String> {
        crate::daemon::sessions::model_catalog::load_provider_model_catalog_for_execution_environment(
            self,
            workspace,
            provider_id,
            execution_environment,
        )
        .await
    }
}

#[derive(Clone)]
pub(in crate::daemon) struct SessionMessageSchedulerSpawner {
    state: Weak<DaemonState>,
}

impl SessionMessageSchedulerSpawner {
    pub(in crate::daemon) fn new(state: Weak<DaemonState>) -> Self {
        Self { state }
    }

    pub(in crate::daemon) async fn ensure_scheduler(
        &self,
        runtime: &SessionRuntime<crate::daemon::scheduler::SchedulerCommand>,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        let state = self.state.clone();
        runtime
            .ensure_scheduler(session, move |session, rx| {
                crate::daemon::scheduler::session_worker(state, session, rx)
            })
            .await
    }
}

#[derive(Clone)]
pub struct SessionMessageCommandHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    update_drain: Arc<UpdateDrainCoordinator>,
    data_root: PathBuf,
    title_model_mode: SessionTitleModelModeHandle,
    scheduler_spawner: SessionMessageSchedulerSpawner,
}

impl SessionMessageCommandHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        session_stores: SessionStoreLookup,
        session_runtime: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
        update_drain: Arc<UpdateDrainCoordinator>,
        data_root: PathBuf,
        title_model_mode: SessionTitleModelModeHandle,
        scheduler_spawner: SessionMessageSchedulerSpawner,
    ) -> Self {
        Self {
            global_store,
            session_stores,
            session_runtime,
            update_drain,
            data_root,
            title_model_mode,
            scheduler_spawner,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.session_stores
            .existing_session_store_for_write(session_id)
            .await
    }

    pub(in crate::daemon) async fn remember_session_meta(&self, session: &Session) {
        self.session_runtime.remember_session_meta(session).await;
    }

    pub(in crate::daemon) async fn session_order_seq_state(
        &self,
        store: &Store,
        session_id: SessionId,
    ) -> Arc<Mutex<ctx_session_tools::order_seq::OrderSeqState>> {
        self.session_runtime
            .get_order_seq_state(store, session_id)
            .await
    }

    pub(in crate::daemon) async fn is_session_running(&self, session_id: SessionId) -> bool {
        self.session_runtime.is_running(session_id).await
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        self.title_model_mode.publish_event(event).await;
    }

    pub(in crate::daemon) async fn post_message_update_drain_reason(&self) -> Option<String> {
        self.update_drain.snapshot().await.map(|drain| drain.reason)
    }

    pub(in crate::daemon) async fn scheduler_sender(
        &self,
        session_id: SessionId,
    ) -> Option<mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>> {
        self.session_runtime.scheduler_sender(session_id).await
    }

    pub(in crate::daemon) async fn ensure_scheduler(
        &self,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        self.scheduler_spawner
            .ensure_scheduler(&self.session_runtime, session)
            .await
    }

    #[cfg(test)]
    pub(in crate::daemon) async fn ensure_scheduler_for_test<F, Fut>(
        &self,
        session: Session,
        spawn_worker: F,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>
    where
        F: FnOnce(Session, mpsc::Receiver<crate::daemon::scheduler::SchedulerCommand>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.session_runtime
            .ensure_scheduler(session, spawn_worker)
            .await
    }

    #[cfg(test)]
    pub(in crate::daemon) async fn subscribe_session_event_head_for_test(
        &self,
        session_id: SessionId,
    ) -> tokio::sync::watch::Receiver<i64> {
        self.session_runtime
            .subscribe_session_event_head(session_id)
            .await
    }

    pub(in crate::daemon) async fn maybe_schedule_first_message_title_generation(
        &self,
        store: &Store,
        session: Session,
        message: &Message,
    ) {
        if let Ok(count) = store.count_user_messages_for_session(session.id).await {
            if count == 1 {
                let _ = self
                    .title_model_mode
                    .schedule_session_title_generation(session, message.content.clone(), false)
                    .await;
            }
        }
    }
}

impl ProviderRuntimeHost for SessionTitleModelModeHandle {
    fn data_root(&self) -> &Path {
        self.data_root()
    }

    fn current_ctx_version(&self) -> Option<String> {
        crate::daemon::provider_capability_hosts::current_ctx_version_for_provider_runtime()
    }

    fn provider_runtime(&self) -> &ProviderRuntime {
        self.providers()
    }

    fn publish_provider_install_ops_events(&self, events: Vec<ProviderInstallOpsEvent>) {
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            self.ops_events(),
            events,
        );
    }
}

struct SessionTitleModelModePublicationHost {
    handle: SessionTitleModelModeHandle,
    task_delta_refresh_host: Arc<SessionTitleModelModeTaskDeltaRefreshHost>,
}

impl SessionTitleModelModePublicationHost {
    fn new(handle: SessionTitleModelModeHandle) -> Self {
        Self {
            task_delta_refresh_host: Arc::new(SessionTitleModelModeTaskDeltaRefreshHost {
                handle: handle.clone(),
            }),
            handle,
        }
    }
}

#[async_trait::async_trait]
impl SessionEventPublicationHost for SessionTitleModelModePublicationHost {
    type TaskDeltaRefreshHost = SessionTitleModelModeTaskDeltaRefreshHost;

    fn task_delta_refresh_host(&self) -> Arc<Self::TaskDeltaRefreshHost> {
        Arc::clone(&self.task_delta_refresh_host)
    }

    async fn load_session(&self, session_id: SessionId) -> Option<Session> {
        let store = self.handle.store_for_session(session_id).await.ok()?;
        store.get_session(session_id).await.ok().flatten()
    }

    async fn list_turn_tool_summaries_for_turn(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Vec<SessionTurnToolSummary> {
        let Ok(store) = self.handle.store_for_session(session_id).await else {
            return Vec::new();
        };
        store
            .list_turn_tool_summaries_for_turns(session_id, std::slice::from_ref(&turn_id))
            .await
            .unwrap_or_default()
    }

    async fn cached_turn_for_read(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Option<SessionTurn> {
        self.handle
            .active_snapshot()
            .get_cached_session_head_for_read(session_id)
            .await
            .and_then(|head| head.turns.into_iter().find(|turn| turn.turn_id == turn_id))
    }

    async fn load_turn(
        &self,
        session_id: SessionId,
        turn_id: ctx_core::ids::TurnId,
    ) -> Option<SessionTurn> {
        let store = self.handle.store_for_session(session_id).await.ok()?;
        store
            .get_session_turn(session_id, turn_id)
            .await
            .ok()
            .flatten()
    }

    async fn session_replay_cursor(
        &self,
        workspace_id: WorkspaceId,
        session_id: SessionId,
    ) -> SessionReplayCursor {
        let cursor = self
            .handle
            .active_snapshot()
            .session_replay_cursor(workspace_id, session_id)
            .await;
        SessionReplayCursor {
            last_event_seq: cursor.last_event_seq,
            projection_rev: cursor.projection_rev,
        }
    }

    async fn load_projection_rev(&self, session_id: SessionId) -> Option<i64> {
        let store = self.handle.store_for_session(session_id).await.ok()?;
        store.get_session_projection_rev(session_id).await.ok()
    }

    async fn publish_session_head_delta(
        &self,
        workspace_id: WorkspaceId,
        session: &Session,
        delta: SessionHeadDelta,
        durable: bool,
    ) {
        self.handle
            .active_snapshot()
            .publish_session_head_delta(workspace_id, session, delta, durable)
            .await;
    }

    async fn publish_session_summary_delta(
        &self,
        workspace_id: WorkspaceId,
        delta: SessionSummaryDelta,
    ) {
        self.handle
            .active_snapshot()
            .publish_session_summary_delta(workspace_id, delta)
            .await;
    }
}

pub(in crate::daemon) struct SessionTitleModelModeTaskDeltaRefreshHost {
    handle: SessionTitleModelModeHandle,
}

#[async_trait::async_trait]
impl SessionTaskDeltaRefreshHost for SessionTitleModelModeTaskDeltaRefreshHost {
    async fn emit_task_delta_refresh(&self, task_id: TaskId) {
        let store = match self.handle.store_for_task(task_id).await {
            Ok(store) => store,
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace task delta refresh store lookup failed: {err:?}"
                );
                return;
            }
        };
        match store.get_workspace_active_task_summary(task_id).await {
            Ok(Some(summary)) => {
                let _ = self
                    .handle
                    .active_snapshot()
                    .publish_task_delta(
                        summary.task.workspace_id,
                        summary.task,
                        TaskDeltaKind::Updated,
                    )
                    .await;
            }
            Ok(None) => match store.get_task(task_id).await {
                Ok(Some(task)) => {
                    let kind = if task.archived_at.is_some() {
                        TaskDeltaKind::Archived
                    } else {
                        TaskDeltaKind::Updated
                    };
                    let _ = self
                        .handle
                        .active_snapshot()
                        .publish_task_delta(task.workspace_id, task, kind)
                        .await;
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(
                        task_id = %task_id.0,
                        "workspace task delta refresh read failed: {err:?}"
                    );
                }
            },
            Err(err) => {
                tracing::warn!(
                    task_id = %task_id.0,
                    "workspace task delta refresh summary read failed: {err:?}"
                );
            }
        }
    }
}

#[derive(Clone)]
pub struct SessionSubagentReadHandle {
    session_stores: SessionStoreLookup,
}

impl SessionSubagentReadHandle {
    pub(in crate::daemon) fn new(session_stores: SessionStoreLookup) -> Self {
        Self { session_stores }
    }

    async fn load_session_store_and_parent(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<(Store, Session)>> {
        let store = match self.session_stores.existing_session_store(session_id).await {
            Ok(store) => store,
            Err(crate::daemon::SessionStoreAccessError::NotFound) => return Ok(None),
            Err(error) => return Err(session_store_access_anyhow(error)),
        };
        let Some(session) = store.get_session(session_id).await? else {
            return Ok(None);
        };
        Ok(Some((store, session)))
    }

    pub(in crate::daemon) async fn list_session_subagents_for_request(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Vec<SessionSummary>>> {
        let Some((store, session)) = self.load_session_store_and_parent(session_id).await? else {
            return Ok(None);
        };
        store.list_subagent_sessions(session.id).await.map(Some)
    }

    pub(in crate::daemon) async fn list_session_subagent_invocations_for_request(
        &self,
        session_id: SessionId,
        turn_id: Option<TurnId>,
    ) -> anyhow::Result<Option<Vec<SubagentInvocation>>> {
        let Some((store, session)) = self.load_session_store_and_parent(session_id).await? else {
            return Ok(None);
        };
        store
            .list_subagent_invocations_for_session(session.id, turn_id)
            .await
            .map(Some)
    }

    pub(in crate::daemon) async fn get_session_subagent_invocation_for_request(
        &self,
        session_id: SessionId,
        invocation_id: &str,
    ) -> anyhow::Result<Option<SubagentInvocation>> {
        let Some((store, _session)) = self.load_session_store_and_parent(session_id).await? else {
            return Ok(None);
        };
        let Some(invocation) = store.get_subagent_invocation(invocation_id).await? else {
            return Ok(None);
        };
        if invocation.parent_session_id != session_id {
            return Ok(None);
        }
        Ok(Some(invocation))
    }
}

type SessionSubagentMcpReadFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type SessionSubagentMcpReadProviderTimeout =
    Arc<dyn Fn() -> SessionSubagentMcpReadFuture<Duration> + Send + Sync>;
type SessionSubagentMcpReadLegacyContextWindowRejectCounter =
    Arc<dyn Fn(String) -> SessionSubagentMcpReadFuture<()> + Send + Sync>;

#[derive(Clone)]
pub struct SessionSubagentMcpReadHandle {
    session_stores: SessionStoreLookup,
    provider_inactivity_timeout: SessionSubagentMcpReadProviderTimeout,
    emit_legacy_context_window_key_reject: SessionSubagentMcpReadLegacyContextWindowRejectCounter,
}

impl SessionSubagentMcpReadHandle {
    pub(in crate::daemon) fn new(
        session_stores: SessionStoreLookup,
        provider_inactivity_timeout: SessionSubagentMcpReadProviderTimeout,
        emit_legacy_context_window_key_reject: SessionSubagentMcpReadLegacyContextWindowRejectCounter,
    ) -> Self {
        Self {
            session_stores,
            provider_inactivity_timeout,
            emit_legacy_context_window_key_reject,
        }
    }

    async fn load_parent_session(
        &self,
        parent_id: SessionId,
    ) -> Result<(Store, Session), crate::daemon::sessions::subagents::SubagentError> {
        let store = match self.session_stores.existing_session_store(parent_id).await {
            Ok(store) => store,
            Err(crate::daemon::SessionStoreAccessError::NotFound) => {
                return Err(crate::daemon::sessions::subagents::not_found(
                    "parent session not found",
                ));
            }
            Err(error) => {
                return Err(crate::daemon::sessions::subagents::internal_api_error(
                    session_store_access_anyhow(error),
                ));
            }
        };
        let parent = store
            .get_session(parent_id)
            .await
            .map_err(crate::daemon::sessions::subagents::internal_api_error)?
            .ok_or_else(|| {
                crate::daemon::sessions::subagents::not_found("parent session not found")
            })?;
        Ok((store, parent))
    }

    async fn provider_inactivity_timeout(&self) -> Duration {
        (self.provider_inactivity_timeout)().await
    }

    pub(in crate::daemon) async fn require_scoped_mcp_session_context(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::ScopedMcpSessionAccessError> {
        self.session_stores
            .require_scoped_mcp_session_context(mcp_auth, session_id)
            .await
    }

    pub(in crate::daemon) async fn list_agents(
        &self,
        parent_id: SessionId,
    ) -> Result<
        Vec<crate::daemon::sessions::subagents::AgentSummary>,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let (store, parent) = self.load_parent_session(parent_id).await?;
        let inactivity_timeout = self.provider_inactivity_timeout().await;
        let subs = store
            .list_subagent_sessions(parent.id)
            .await
            .map_err(crate::daemon::sessions::subagents::internal_api_error)?;
        let mut agents = Vec::with_capacity(subs.len());
        for sub in subs {
            let (summary, _latest_turn) = crate::daemon::sessions::subagents::build_agent_summary(
                &store,
                sub.id,
                &sub.title,
                inactivity_timeout,
            )
            .await?;
            agents.push(summary);
        }
        Ok(agents)
    }

    pub(in crate::daemon) async fn get_agent(
        &self,
        parent_id: SessionId,
        req: crate::daemon::sessions::subagents::GetAgentReq,
    ) -> Result<
        crate::daemon::sessions::subagents::GetAgentResp,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let (store, parent) = self.load_parent_session(parent_id).await?;
        let inactivity_timeout = self.provider_inactivity_timeout().await;
        let child = crate::daemon::sessions::subagents::resolve_child_agent_session(
            &store,
            &parent,
            &req.agent_id,
        )
        .await?;
        let detail = crate::daemon::sessions::subagents::build_agent_detail_for_mcp_read(
            &store,
            &parent,
            &child,
            inactivity_timeout,
            &self.emit_legacy_context_window_key_reject,
        )
        .await?;
        Ok(crate::daemon::sessions::subagents::GetAgentResp { agent: detail })
    }

    pub(in crate::daemon) async fn wait_agent(
        &self,
        parent_id: SessionId,
        req: crate::daemon::sessions::subagents::WaitAgentReq,
    ) -> Result<
        crate::daemon::sessions::subagents::WaitAgentResp,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let agent_ids = ctx_subagent_service::normalize_wait_agent_ids(
            req.agent_id.as_deref(),
            req.agent_ids.as_deref(),
        )
        .map_err(|error| {
            crate::daemon::sessions::subagents::api_error(
                crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                error,
            )
        })?;
        let (store, parent) = self.load_parent_session(parent_id).await?;
        let inactivity_timeout = self.provider_inactivity_timeout().await;
        let targets =
            crate::daemon::sessions::subagents::collect_wait_targets(&store, &parent, &agent_ids)
                .await?;
        let mode = ctx_subagent_service::parse_wait_mode(req.mode.as_deref()).map_err(|error| {
            crate::daemon::sessions::subagents::api_error(
                crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                error,
            )
        })?;
        let until =
            ctx_subagent_service::parse_wait_until(req.until.as_deref()).map_err(|error| {
                crate::daemon::sessions::subagents::api_error(
                    crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                    error,
                )
            })?;
        if req.since_seq.is_some() && targets.len() != 1 {
            return Err(crate::daemon::sessions::subagents::api_error(
                crate::daemon::sessions::subagents::SubagentErrorKind::BadRequest,
                "since_seq is only supported with a single agent_id",
            ));
        }

        let timeout_ms = req.timeout_ms.unwrap_or(30_000);
        let mut details = self
            .collect_wait_details(&store, &parent, &targets, inactivity_timeout)
            .await?;
        let thresholds = subagent_wait_update_thresholds(&details, until, req.since_seq);

        if ctx_subagent_service::wait_predicate_satisfied(
            &subagent_agent_wait_details(&details),
            mode,
            until,
            &thresholds,
        ) {
            return Ok(subagent_wait_response("matched", mode, until, details));
        }
        if timeout_ms == 0 {
            return Ok(subagent_wait_response("timeout", mode, until, details));
        }

        let started_at = Instant::now();
        while started_at.elapsed() < Duration::from_millis(timeout_ms) {
            tokio::time::sleep(Duration::from_millis(100)).await;
            details = self
                .collect_wait_details(&store, &parent, &targets, inactivity_timeout)
                .await?;
            if ctx_subagent_service::wait_predicate_satisfied(
                &subagent_agent_wait_details(&details),
                mode,
                until,
                &thresholds,
            ) {
                return Ok(subagent_wait_response("matched", mode, until, details));
            }
        }

        Ok(subagent_wait_response("timeout", mode, until, details))
    }

    async fn collect_wait_details(
        &self,
        store: &Store,
        parent: &Session,
        targets: &[Session],
        inactivity_timeout: Duration,
    ) -> Result<
        Vec<crate::daemon::sessions::subagents::AgentDetail>,
        crate::daemon::sessions::subagents::SubagentError,
    > {
        let mut details = Vec::with_capacity(targets.len());
        for target in targets {
            details.push(
                crate::daemon::sessions::subagents::build_agent_detail_for_mcp_read(
                    store,
                    parent,
                    target,
                    inactivity_timeout,
                    &self.emit_legacy_context_window_key_reject,
                )
                .await?,
            );
        }
        Ok(details)
    }
}

fn subagent_wait_update_thresholds(
    details: &[crate::daemon::sessions::subagents::AgentDetail],
    until: ctx_subagent_service::AgentWaitUntil,
    since_seq: Option<i64>,
) -> HashMap<String, i64> {
    let mut thresholds = HashMap::new();
    match until {
        ctx_subagent_service::AgentWaitUntil::Terminal => {}
        ctx_subagent_service::AgentWaitUntil::Update => {
            if let Some(since_seq) = since_seq {
                thresholds.insert(details[0].agent.agent_id.clone(), since_seq);
            } else {
                for detail in details {
                    thresholds.insert(detail.agent.agent_id.clone(), detail.agent.last_event_seq);
                }
            }
        }
    }
    thresholds
}

fn subagent_wait_response(
    wait_status: &str,
    mode: ctx_subagent_service::AgentWaitMode,
    until: ctx_subagent_service::AgentWaitUntil,
    results: Vec<crate::daemon::sessions::subagents::AgentDetail>,
) -> crate::daemon::sessions::subagents::WaitAgentResp {
    crate::daemon::sessions::subagents::WaitAgentResp {
        wait_status: wait_status.to_string(),
        mode: mode.as_str().to_string(),
        until: until.as_str().to_string(),
        results,
    }
}

fn subagent_agent_wait_details(
    details: &[crate::daemon::sessions::subagents::AgentDetail],
) -> Vec<ctx_subagent_service::AgentWaitDetail<'_>> {
    details
        .iter()
        .map(|detail| ctx_subagent_service::AgentWaitDetail {
            agent_id: &detail.agent.agent_id,
            has_current_run: detail.agent.current_run_id.is_some(),
            has_latest_result: detail.agent.latest_result_status.is_some(),
            last_event_seq: detail.agent.last_event_seq,
        })
        .collect()
}

#[derive(Clone)]
pub struct SessionReadModelsHandle {
    global_store: Store,
    session_stores: SessionStoreLookup,
    stores: StoreManager,
    active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
    tool_output_spool_dir: PathBuf,
    perf_telemetry: PerfTelemetry,
}

impl SessionReadModelsHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        session_stores: SessionStoreLookup,
        stores: StoreManager,
        active_snapshot: Arc<WorkspaceActiveSnapshotHub>,
        tool_output_spool_dir: PathBuf,
        perf_telemetry: PerfTelemetry,
    ) -> Self {
        Self {
            global_store,
            session_stores,
            stores,
            active_snapshot,
            tool_output_spool_dir,
            perf_telemetry,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn session_stores(&self) -> &SessionStoreLookup {
        &self.session_stores
    }

    pub(in crate::daemon) fn stores(&self) -> &StoreManager {
        &self.stores
    }

    pub(in crate::daemon) fn active_snapshot(&self) -> &WorkspaceActiveSnapshotHub {
        self.active_snapshot.as_ref()
    }

    pub(in crate::daemon) fn tool_output_spool_dir(&self) -> &Path {
        &self.tool_output_spool_dir
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }
}

#[derive(Clone)]
pub struct SessionArtifactsHandle {
    lookup: SessionStoreLookup,
    tool_output_spool_dir: PathBuf,
    effects: Arc<SessionArtifactEffects>,
}

impl SessionArtifactsHandle {
    pub(in crate::daemon) fn new(
        lookup: SessionStoreLookup,
        tool_output_spool_dir: PathBuf,
        effects: Arc<SessionArtifactEffects>,
    ) -> Self {
        Self {
            lookup,
            tool_output_spool_dir,
            effects,
        }
    }

    pub(in crate::daemon) async fn existing_session_store(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.lookup.existing_session_store(session_id).await
    }

    pub(in crate::daemon) async fn existing_session_store_for_write(
        &self,
        session_id: SessionId,
    ) -> Result<Store, crate::daemon::SessionStoreAccessError> {
        self.lookup
            .existing_session_store_for_write(session_id)
            .await
    }

    pub(in crate::daemon) async fn require_scoped_mcp_session_context(
        &self,
        mcp_auth: ctx_mcp_auth::McpAuthContext,
        session_id: SessionId,
    ) -> Result<(), crate::daemon::ScopedMcpSessionAccessError> {
        self.lookup
            .require_scoped_mcp_session_context(mcp_auth, session_id)
            .await
    }

    pub(in crate::daemon) fn session_tool_output_spool_dir(
        &self,
        session_id: SessionId,
    ) -> PathBuf {
        self.tool_output_spool_dir.join(session_id.0.to_string())
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        self.effects.publish_event(event).await;
    }
}

pub(in crate::daemon) struct SessionArtifactEffects {
    publish_event: Arc<dyn Fn(SessionEvent) -> SessionArtifactsFuture<()> + Send + Sync>,
}

impl SessionArtifactEffects {
    pub(in crate::daemon) fn new(
        publish_event: Arc<dyn Fn(SessionEvent) -> SessionArtifactsFuture<()> + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(Self { publish_event })
    }

    pub(in crate::daemon) async fn publish_event(&self, event: SessionEvent) {
        (self.publish_event)(event).await;
    }
}

type SessionVcsFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type SessionVcsWorktreeBoolEffect =
    Arc<dyn Fn(Worktree) -> SessionVcsFuture<anyhow::Result<bool>> + Send + Sync>;
type SessionVcsGitStatusEffect = Arc<
    dyn Fn(Worktree, bool, bool) -> SessionVcsFuture<anyhow::Result<GitStatusSnapshot>>
        + Send
        + Sync,
>;
type SessionVcsCommitEffect =
    Arc<dyn Fn(Worktree, String) -> SessionVcsFuture<anyhow::Result<String>> + Send + Sync>;
type SessionVcsDiffEffect =
    Arc<dyn Fn(Worktree, String) -> SessionVcsFuture<anyhow::Result<String>> + Send + Sync>;
type SessionVcsDiffSummaryEffect = Arc<
    dyn Fn(Worktree, String) -> SessionVcsFuture<anyhow::Result<WorktreeVcsDiffSummaryCounts>>
        + Send
        + Sync,
>;
type SessionVcsDiffBaseEffect = Arc<
    dyn Fn(Worktree, SessionVcsDiffBaseQuery) -> SessionVcsFuture<WorktreeDiffBaseResolution>
        + Send
        + Sync,
>;
type SessionVcsPatchEffect =
    Arc<dyn Fn(Worktree, String, bool) -> SessionVcsFuture<anyhow::Result<()>> + Send + Sync>;
type SessionVcsSnapshotEffect =
    Arc<dyn Fn(WorktreeId) -> SessionVcsFuture<Option<WorktreeVcsSnapshot>> + Send + Sync>;
type SessionVcsCompatMetricEffect =
    Arc<dyn Fn(&'static str, &'static str) -> SessionVcsFuture<()> + Send + Sync>;
type SessionVcsNoRepoClassifier = Arc<dyn Fn(&anyhow::Error) -> bool + Send + Sync>;

pub(in crate::daemon) struct SessionVcsEffectsParts {
    worktree_has_vcs_repo: SessionVcsWorktreeBoolEffect,
    load_git_status_snapshot: SessionVcsGitStatusEffect,
    resolve_worktree_commit: SessionVcsCommitEffect,
    diff_worktree_for_session: SessionVcsDiffEffect,
    diff_worktree_summary_for_session: SessionVcsDiffSummaryEffect,
    resolve_worktree_diff_base: SessionVcsDiffBaseEffect,
    apply_worktree_vcs_session_patch: SessionVcsPatchEffect,
    cached_worktree_vcs_snapshot: SessionVcsSnapshotEffect,
    emit_compat_payload_reject_counter: SessionVcsCompatMetricEffect,
    is_no_vcs_repo_error: SessionVcsNoRepoClassifier,
}

pub(in crate::daemon) struct SessionVcsEffects {
    worktree_has_vcs_repo: SessionVcsWorktreeBoolEffect,
    load_git_status_snapshot: SessionVcsGitStatusEffect,
    resolve_worktree_commit: SessionVcsCommitEffect,
    diff_worktree_for_session: SessionVcsDiffEffect,
    diff_worktree_summary_for_session: SessionVcsDiffSummaryEffect,
    resolve_worktree_diff_base: SessionVcsDiffBaseEffect,
    apply_worktree_vcs_session_patch: SessionVcsPatchEffect,
    cached_worktree_vcs_snapshot: SessionVcsSnapshotEffect,
    emit_compat_payload_reject_counter: SessionVcsCompatMetricEffect,
    is_no_vcs_repo_error: SessionVcsNoRepoClassifier,
}

impl SessionVcsEffects {
    pub(in crate::daemon) fn new(parts: SessionVcsEffectsParts) -> Arc<Self> {
        Arc::new(Self {
            worktree_has_vcs_repo: parts.worktree_has_vcs_repo,
            load_git_status_snapshot: parts.load_git_status_snapshot,
            resolve_worktree_commit: parts.resolve_worktree_commit,
            diff_worktree_for_session: parts.diff_worktree_for_session,
            diff_worktree_summary_for_session: parts.diff_worktree_summary_for_session,
            resolve_worktree_diff_base: parts.resolve_worktree_diff_base,
            apply_worktree_vcs_session_patch: parts.apply_worktree_vcs_session_patch,
            cached_worktree_vcs_snapshot: parts.cached_worktree_vcs_snapshot,
            emit_compat_payload_reject_counter: parts.emit_compat_payload_reject_counter,
            is_no_vcs_repo_error: parts.is_no_vcs_repo_error,
        })
    }
}

#[derive(Clone)]
pub struct SessionVcsHandle {
    lookup: SessionStoreLookup,
    effects: Arc<SessionVcsEffects>,
}

impl SessionVcsHandle {
    pub(in crate::daemon) fn new(
        lookup: SessionStoreLookup,
        effects: Arc<SessionVcsEffects>,
    ) -> Self {
        Self { lookup, effects }
    }

    pub(in crate::daemon) async fn session_store_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self.lookup.existing_session_store(session_id).await {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    pub(in crate::daemon) async fn session_store_for_write_or_none(
        &self,
        session_id: SessionId,
    ) -> anyhow::Result<Option<Store>> {
        match self
            .lookup
            .existing_session_store_for_write(session_id)
            .await
        {
            Ok(store) => Ok(Some(store)),
            Err(crate::daemon::SessionStoreAccessError::NotFound) => Ok(None),
            Err(error) => Err(session_store_access_anyhow(error)),
        }
    }

    pub(in crate::daemon) async fn worktree_has_vcs_repo(
        &self,
        worktree: &Worktree,
    ) -> anyhow::Result<bool> {
        (self.effects.worktree_has_vcs_repo)(worktree.clone()).await
    }

    pub(in crate::daemon) async fn load_git_status_snapshot(
        &self,
        worktree: &Worktree,
        include_untracked_files: bool,
        include_entries: bool,
    ) -> anyhow::Result<GitStatusSnapshot> {
        (self.effects.load_git_status_snapshot)(
            worktree.clone(),
            include_untracked_files,
            include_entries,
        )
        .await
    }

    pub(in crate::daemon) async fn resolve_worktree_commit(
        &self,
        worktree: &Worktree,
        revision: &str,
    ) -> anyhow::Result<String> {
        (self.effects.resolve_worktree_commit)(worktree.clone(), revision.to_string()).await
    }

    pub(in crate::daemon) async fn diff_worktree_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<String> {
        (self.effects.diff_worktree_for_session)(worktree.clone(), base_commit_sha.to_string())
            .await
    }

    pub(in crate::daemon) async fn diff_worktree_summary_for_session(
        &self,
        worktree: &Worktree,
        base_commit_sha: &str,
    ) -> anyhow::Result<WorktreeVcsDiffSummaryCounts> {
        (self.effects.diff_worktree_summary_for_session)(
            worktree.clone(),
            base_commit_sha.to_string(),
        )
        .await
    }

    pub(in crate::daemon) async fn resolve_worktree_diff_base(
        &self,
        worktree: &Worktree,
        query: SessionVcsDiffBaseQuery,
    ) -> WorktreeDiffBaseResolution {
        (self.effects.resolve_worktree_diff_base)(worktree.clone(), query).await
    }

    pub(in crate::daemon) async fn apply_worktree_vcs_session_patch(
        &self,
        worktree: &Worktree,
        patch: &str,
        reverse_patch: bool,
    ) -> anyhow::Result<()> {
        (self.effects.apply_worktree_vcs_session_patch)(
            worktree.clone(),
            patch.to_string(),
            reverse_patch,
        )
        .await
    }

    pub(in crate::daemon) async fn cached_worktree_vcs_snapshot(
        &self,
        worktree_id: WorktreeId,
    ) -> Option<WorktreeVcsSnapshot> {
        (self.effects.cached_worktree_vcs_snapshot)(worktree_id).await
    }

    pub(in crate::daemon) async fn emit_compat_payload_reject_counter(
        &self,
        surface: &'static str,
        issue: &'static str,
    ) {
        (self.effects.emit_compat_payload_reject_counter)(surface, issue).await;
    }

    pub(in crate::daemon) fn is_no_vcs_repo_error(&self, error: &anyhow::Error) -> bool {
        (self.effects.is_no_vcs_repo_error)(error)
    }
}

#[derive(Clone)]
pub struct TaskListingHandle {
    workspace_stores: ProtectedWorkspaceStoreLookup,
    archived_rev_loader: TaskArchivedRevLoader,
}

impl TaskListingHandle {
    pub(in crate::daemon) fn new(
        workspace_stores: ProtectedWorkspaceStoreLookup,
        archived_rev_loader: TaskArchivedRevLoader,
    ) -> Self {
        Self {
            workspace_stores,
            archived_rev_loader,
        }
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn load_archived_rev(&self, workspace_id: WorkspaceId) -> i64 {
        (self.archived_rev_loader)(workspace_id).await
    }
}

#[derive(Clone)]
pub struct TaskSessionListingHandle {
    lookup: TaskStoreLookup,
}

impl TaskSessionListingHandle {
    pub(in crate::daemon) fn new(lookup: TaskStoreLookup) -> Self {
        Self { lookup }
    }

    pub(in crate::daemon) async fn task_store_or_none(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<Option<Store>> {
        self.lookup.task_store_or_none(task_id).await
    }
}

#[derive(Clone)]
pub struct TaskReadStateHandle {
    lookup: TaskStoreLookup,
    effects: Arc<TaskMetadataEffects>,
}

impl TaskReadStateHandle {
    pub(in crate::daemon) fn new(
        lookup: TaskStoreLookup,
        effects: Arc<TaskMetadataEffects>,
    ) -> Self {
        Self { lookup, effects }
    }

    pub(in crate::daemon) async fn task_store_or_none(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<Option<Store>> {
        self.lookup.task_store_or_none(task_id).await
    }

    pub(in crate::daemon) fn effects(&self) -> &TaskMetadataEffects {
        self.effects.as_ref()
    }

    #[cfg(test)]
    pub(in crate::daemon) fn with_effects_for_test(
        &self,
        effects: Arc<TaskMetadataEffects>,
    ) -> Self {
        Self {
            lookup: self.lookup.clone(),
            effects,
        }
    }
}

#[derive(Clone)]
pub struct TaskTitleHandle {
    lookup: TaskStoreLookup,
    effects: Arc<TaskMetadataEffects>,
    close_web_sessions_for_task: TaskCloseWebSessionsForTask,
}

impl TaskTitleHandle {
    pub(in crate::daemon) fn new(
        lookup: TaskStoreLookup,
        effects: Arc<TaskMetadataEffects>,
        close_web_sessions_for_task: TaskCloseWebSessionsForTask,
    ) -> Self {
        Self {
            lookup,
            effects,
            close_web_sessions_for_task,
        }
    }

    pub(in crate::daemon) async fn task_store_or_none(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<Option<Store>> {
        self.lookup.task_store_or_none(task_id).await
    }

    pub(in crate::daemon) fn effects(&self) -> &TaskMetadataEffects {
        self.effects.as_ref()
    }

    pub(in crate::daemon) async fn close_web_sessions_for_task(
        &self,
        session_ids: HashSet<String>,
        worktree_ids: HashSet<String>,
    ) -> anyhow::Result<usize> {
        (self.close_web_sessions_for_task)(session_ids, worktree_ids).await
    }

    #[cfg(test)]
    pub(in crate::daemon) fn with_effects_and_close_for_test(
        &self,
        effects: Arc<TaskMetadataEffects>,
        close_web_sessions_for_task: TaskCloseWebSessionsForTask,
    ) -> Self {
        Self {
            lookup: self.lookup.clone(),
            effects,
            close_web_sessions_for_task,
        }
    }
}

#[derive(Clone)]
pub struct TaskLifecycleHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    workspace: Arc<TaskLifecycleWorkspaceRuntime>,
    effects: Arc<TaskLifecycleEffects>,
}

impl TaskLifecycleHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        workspace: Arc<TaskLifecycleWorkspaceRuntime>,
        effects: Arc<TaskLifecycleEffects>,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            workspace,
            effects,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) async fn existing_workspace_store(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Store, crate::daemon::WorkspaceStoreAccessError> {
        self.workspace_stores
            .existing_workspace_store(workspace_id)
            .await
    }

    pub(in crate::daemon) fn workspace(&self) -> &TaskLifecycleWorkspaceRuntime {
        &self.workspace
    }

    pub(in crate::daemon) fn effects(&self) -> &TaskLifecycleEffects {
        &self.effects
    }
}

#[derive(Clone)]
pub struct TaskCreationHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    session_admission: TaskSessionAdmissionHandle,
    task_lifecycle: TaskLifecycleHandle,
}

impl TaskCreationHandle {
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        session_admission: TaskSessionAdmissionHandle,
        task_lifecycle: TaskLifecycleHandle,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            session_admission,
            task_lifecycle,
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

    pub(in crate::daemon) fn session_admission(&self) -> &TaskSessionAdmissionHandle {
        &self.session_admission
    }

    pub(in crate::daemon) async fn delete_loaded_task_with_cleanup(
        &self,
        store: &Store,
        workspace: &Workspace,
        task: &Task,
    ) -> Result<(), crate::daemon::tasks::TaskLifecycleError> {
        self.task_lifecycle
            .delete_loaded_task_with_cleanup(store, workspace, task)
            .await
    }
}

#[derive(Clone)]
pub struct TaskSessionAdmissionHandle {
    global_store: Store,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    sessions: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
    providers: Arc<ProviderRuntime>,
    provider_status: ProviderStatusHandle,
    workspace: Arc<TaskAdmissionWorkspaceRuntime>,
    effects: Arc<TaskAdmissionSessionEffects>,
    model_catalog_loader: TaskAdmissionModelCatalogLoader,
    telemetry: Telemetry,
    ops_events: OpsEvents,
    perf_telemetry: PerfTelemetry,
}

impl TaskSessionAdmissionHandle {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::daemon) fn new(
        global_store: Store,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        sessions: Arc<SessionRuntime<crate::daemon::scheduler::SchedulerCommand>>,
        providers: Arc<ProviderRuntime>,
        provider_status: ProviderStatusHandle,
        workspace: Arc<TaskAdmissionWorkspaceRuntime>,
        effects: Arc<TaskAdmissionSessionEffects>,
        model_catalog_loader: TaskAdmissionModelCatalogLoader,
        telemetry: Telemetry,
        ops_events: OpsEvents,
        perf_telemetry: PerfTelemetry,
    ) -> Self {
        Self {
            global_store,
            workspace_stores,
            sessions,
            providers,
            provider_status,
            workspace,
            effects,
            model_catalog_loader,
            telemetry,
            ops_events,
            perf_telemetry,
        }
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        &self.global_store
    }

    pub(in crate::daemon) fn sessions(
        &self,
    ) -> &SessionRuntime<crate::daemon::scheduler::SchedulerCommand> {
        self.sessions.as_ref()
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn provider_status(&self) -> &ProviderStatusHandle {
        &self.provider_status
    }

    pub(in crate::daemon) fn workspace(&self) -> &TaskAdmissionWorkspaceRuntime {
        self.workspace.as_ref()
    }

    pub(in crate::daemon) fn effects(&self) -> &TaskAdmissionSessionEffects {
        self.effects.as_ref()
    }

    pub(in crate::daemon) fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) fn perf_telemetry(&self) -> &PerfTelemetry {
        &self.perf_telemetry
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
            .await
    }

    pub(in crate::daemon) async fn load_provider_model_catalog_for_execution_environment(
        &self,
        workspace: &Workspace,
        provider_id: &str,
        execution_environment: ExecutionEnvironment,
    ) -> Result<Option<ModelCatalog>, String> {
        (self.model_catalog_loader)(
            workspace.clone(),
            provider_id.to_string(),
            execution_environment,
        )
        .await
    }
}

type TaskWorktreeCleanupTarget = crate::daemon::workspaces::TaskWorktreeCleanupTarget;
type BranchCleanupErrorMode = crate::daemon::workspaces::BranchCleanupErrorMode;
type ResolvedExistingWorktreeExecution =
    crate::daemon::workspaces::ResolvedExistingWorktreeExecution;

type TaskLifecycleCleanupTaskWorktrees = Arc<
    dyn Fn(
            Workspace,
            TaskId,
            Vec<TaskWorktreeCleanupTarget>,
            BranchCleanupErrorMode,
        ) -> TaskLifecycleFuture<Vec<anyhow::Error>>
        + Send
        + Sync,
>;
type TaskLifecycleRematerializeSandboxBinding = Arc<
    dyn Fn(
            Workspace,
            Worktree,
            SandboxBinding,
        ) -> TaskLifecycleFuture<anyhow::Result<SandboxBinding>>
        + Send
        + Sync,
>;
type TaskLifecycleWorktreeEffect =
    Arc<dyn Fn(Workspace, Worktree) -> TaskLifecycleFuture<anyhow::Result<()>> + Send + Sync>;
type TaskLifecycleTaskWorktreeEffect = Arc<
    dyn Fn(Workspace, Worktree, TaskId) -> TaskLifecycleFuture<anyhow::Result<()>> + Send + Sync,
>;
type TaskAdmissionResolveExistingWorktreeExecution = Arc<
    dyn Fn(
            Store,
            Workspace,
            WorktreeId,
        ) -> TaskAdmissionFuture<anyhow::Result<ResolvedExistingWorktreeExecution>>
        + Send
        + Sync,
>;
type TaskAdmissionProvisionWorktreeForExecution = Arc<
    dyn Fn(
            Workspace,
            WorktreeId,
            String,
            String,
            ExecutionSettings,
        ) -> TaskAdmissionFuture<anyhow::Result<(PathBuf, Option<SandboxBinding>)>>
        + Send
        + Sync,
>;
type TaskAdmissionPersistProvisionedWorktree = Arc<
    dyn Fn(
            Store,
            Workspace,
            Worktree,
            Option<SandboxBinding>,
        ) -> TaskAdmissionFuture<anyhow::Result<Worktree>>
        + Send
        + Sync,
>;
type TaskAdmissionCleanupTaskWorktrees = Arc<
    dyn Fn(
            Workspace,
            TaskId,
            Vec<TaskWorktreeCleanupTarget>,
            BranchCleanupErrorMode,
        ) -> TaskAdmissionFuture<Vec<anyhow::Error>>
        + Send
        + Sync,
>;
type TaskAdmissionTaskWorktreeEffect = Arc<
    dyn Fn(Workspace, Worktree, TaskId) -> TaskAdmissionFuture<anyhow::Result<()>> + Send + Sync,
>;
type TaskAdmissionTaskUpsertEffect =
    Arc<dyn Fn(TaskId) -> TaskAdmissionFuture<anyhow::Result<()>> + Send + Sync>;

pub(in crate::daemon) struct TaskLifecycleWorkspaceRuntime {
    data_root: PathBuf,
    cleanup_task_worktrees: TaskLifecycleCleanupTaskWorktrees,
    rematerialize_sandbox_binding_for_worktree: TaskLifecycleRematerializeSandboxBinding,
    ensure_worktree_attachment_mounts_if_materialized: TaskLifecycleWorktreeEffect,
    spawn_worktree_bootstrap: TaskLifecycleWorktreeEffect,
    ensure_task_commit_hook: TaskLifecycleTaskWorktreeEffect,
}

impl TaskLifecycleWorkspaceRuntime {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        cleanup_task_worktrees: TaskLifecycleCleanupTaskWorktrees,
        rematerialize_sandbox_binding_for_worktree: TaskLifecycleRematerializeSandboxBinding,
        ensure_worktree_attachment_mounts_if_materialized: TaskLifecycleWorktreeEffect,
        spawn_worktree_bootstrap: TaskLifecycleWorktreeEffect,
        ensure_task_commit_hook: TaskLifecycleTaskWorktreeEffect,
    ) -> Arc<Self> {
        Arc::new(Self {
            data_root,
            cleanup_task_worktrees,
            rematerialize_sandbox_binding_for_worktree,
            ensure_worktree_attachment_mounts_if_materialized,
            spawn_worktree_bootstrap,
            ensure_task_commit_hook,
        })
    }

    pub(in crate::daemon) fn managed_worktree_root(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Option<PathBuf> {
        ctx_worktree_vcs_service::matching_managed_worktree_path(
            &self.data_root,
            workspace.id,
            worktree.id,
            PathBuf::from(&worktree.root_path),
        )
    }

    pub(in crate::daemon) async fn cleanup_task_worktrees(
        &self,
        workspace: &Workspace,
        task_id: TaskId,
        targets: &[crate::daemon::workspaces::TaskWorktreeCleanupTarget],
        mode: crate::daemon::workspaces::BranchCleanupErrorMode,
    ) -> Vec<anyhow::Error> {
        (self.cleanup_task_worktrees)(workspace.clone(), task_id, targets.to_vec(), mode).await
    }

    pub(in crate::daemon) async fn rematerialize_sandbox_binding_for_worktree(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        binding: &SandboxBinding,
    ) -> anyhow::Result<SandboxBinding> {
        (self.rematerialize_sandbox_binding_for_worktree)(
            workspace.clone(),
            worktree.clone(),
            binding.clone(),
        )
        .await
    }

    pub(in crate::daemon) async fn ensure_worktree_attachment_mounts_if_materialized(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> anyhow::Result<()> {
        (self.ensure_worktree_attachment_mounts_if_materialized)(
            workspace.clone(),
            worktree.clone(),
        )
        .await
    }

    pub(in crate::daemon) async fn spawn_worktree_bootstrap(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> anyhow::Result<()> {
        (self.spawn_worktree_bootstrap)(workspace.clone(), worktree.clone()).await
    }

    pub(in crate::daemon) async fn ensure_task_commit_hook(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        (self.ensure_task_commit_hook)(workspace.clone(), worktree.clone(), task_id).await
    }
}

pub(in crate::daemon) struct TaskMetadataEffects {
    emit_workspace_task_delta:
        Arc<dyn Fn(Task, TaskDeltaKind) -> TaskMetadataFuture<()> + Send + Sync>,
    emit_workspace_task_upsert:
        Arc<dyn Fn(TaskId) -> TaskMetadataFuture<anyhow::Result<()>> + Send + Sync>,
}

impl TaskMetadataEffects {
    pub(in crate::daemon) fn new(
        emit_workspace_task_delta: Arc<
            dyn Fn(Task, TaskDeltaKind) -> TaskMetadataFuture<()> + Send + Sync,
        >,
        emit_workspace_task_upsert: Arc<
            dyn Fn(TaskId) -> TaskMetadataFuture<anyhow::Result<()>> + Send + Sync,
        >,
    ) -> Arc<Self> {
        Arc::new(Self {
            emit_workspace_task_delta,
            emit_workspace_task_upsert,
        })
    }

    pub(in crate::daemon) async fn publish_task_updated(&self, task_id: TaskId, task: Task) {
        (self.emit_workspace_task_delta)(task, TaskDeltaKind::Updated).await;
        if let Err(error) = (self.emit_workspace_task_upsert)(task_id).await {
            tracing::warn!(task_id = %task_id.0, "workspace active snapshot refresh failed: {error:?}");
        }
    }
}

pub(in crate::daemon) struct TaskLifecycleEffects {
    cleanup_session: Arc<dyn Fn(SessionId) -> TaskLifecycleFuture<()> + Send + Sync>,
    emit_workspace_task_delta:
        Arc<dyn Fn(Task, TaskDeltaKind) -> TaskLifecycleFuture<()> + Send + Sync>,
    emit_workspace_task_upsert:
        Arc<dyn Fn(TaskId) -> TaskLifecycleFuture<anyhow::Result<()>> + Send + Sync>,
    remove_active_snapshot_session: Arc<dyn Fn(SessionId) -> TaskLifecycleFuture<()> + Send + Sync>,
    refresh_session_head_cache: Arc<dyn Fn(SessionId) -> TaskLifecycleFuture<()> + Send + Sync>,
    emit_workspace_archived_task_delete:
        Arc<dyn Fn(WorkspaceId, TaskId) -> TaskLifecycleFuture<()> + Send + Sync>,
    emit_workspace_task_delete:
        Arc<dyn Fn(WorkspaceId, TaskId) -> TaskLifecycleFuture<()> + Send + Sync>,
}

impl TaskLifecycleEffects {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::daemon) fn new(
        cleanup_session: Arc<dyn Fn(SessionId) -> TaskLifecycleFuture<()> + Send + Sync>,
        emit_workspace_task_delta: Arc<
            dyn Fn(Task, TaskDeltaKind) -> TaskLifecycleFuture<()> + Send + Sync,
        >,
        emit_workspace_task_upsert: Arc<
            dyn Fn(TaskId) -> TaskLifecycleFuture<anyhow::Result<()>> + Send + Sync,
        >,
        remove_active_snapshot_session: Arc<
            dyn Fn(SessionId) -> TaskLifecycleFuture<()> + Send + Sync,
        >,
        refresh_session_head_cache: Arc<dyn Fn(SessionId) -> TaskLifecycleFuture<()> + Send + Sync>,
        emit_workspace_archived_task_delete: Arc<
            dyn Fn(WorkspaceId, TaskId) -> TaskLifecycleFuture<()> + Send + Sync,
        >,
        emit_workspace_task_delete: Arc<
            dyn Fn(WorkspaceId, TaskId) -> TaskLifecycleFuture<()> + Send + Sync,
        >,
    ) -> Arc<Self> {
        Arc::new(Self {
            cleanup_session,
            emit_workspace_task_delta,
            emit_workspace_task_upsert,
            remove_active_snapshot_session,
            refresh_session_head_cache,
            emit_workspace_archived_task_delete,
            emit_workspace_task_delete,
        })
    }

    pub(in crate::daemon) async fn cleanup_session(&self, session_id: SessionId) {
        (self.cleanup_session)(session_id).await;
    }

    pub(in crate::daemon) async fn emit_workspace_task_delta(
        &self,
        task: Task,
        kind: TaskDeltaKind,
    ) {
        (self.emit_workspace_task_delta)(task, kind).await;
    }

    pub(in crate::daemon) async fn emit_workspace_task_upsert(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        (self.emit_workspace_task_upsert)(task_id).await
    }

    pub(in crate::daemon) async fn remove_active_snapshot_session(&self, session_id: SessionId) {
        (self.remove_active_snapshot_session)(session_id).await;
    }

    pub(in crate::daemon) async fn refresh_session_head_cache(&self, session_id: SessionId) {
        (self.refresh_session_head_cache)(session_id).await;
    }

    pub(in crate::daemon) async fn emit_workspace_archived_task_delete(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        (self.emit_workspace_archived_task_delete)(workspace_id, task_id).await;
    }

    pub(in crate::daemon) async fn emit_workspace_task_delete(
        &self,
        workspace_id: WorkspaceId,
        task_id: TaskId,
    ) {
        (self.emit_workspace_task_delete)(workspace_id, task_id).await;
    }
}

pub(in crate::daemon) struct TaskAdmissionWorkspaceRuntime {
    data_root: PathBuf,
    resolve_existing_worktree_execution: TaskAdmissionResolveExistingWorktreeExecution,
    provision_worktree_for_execution: TaskAdmissionProvisionWorktreeForExecution,
    persist_provisioned_worktree: TaskAdmissionPersistProvisionedWorktree,
    cleanup_task_worktrees: TaskAdmissionCleanupTaskWorktrees,
    ensure_task_commit_hook: TaskAdmissionTaskWorktreeEffect,
    emit_workspace_task_upsert: TaskAdmissionTaskUpsertEffect,
}

impl TaskAdmissionWorkspaceRuntime {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        resolve_existing_worktree_execution: TaskAdmissionResolveExistingWorktreeExecution,
        provision_worktree_for_execution: TaskAdmissionProvisionWorktreeForExecution,
        persist_provisioned_worktree: TaskAdmissionPersistProvisionedWorktree,
        cleanup_task_worktrees: TaskAdmissionCleanupTaskWorktrees,
        ensure_task_commit_hook: TaskAdmissionTaskWorktreeEffect,
        emit_workspace_task_upsert: TaskAdmissionTaskUpsertEffect,
    ) -> Arc<Self> {
        Arc::new(Self {
            data_root,
            resolve_existing_worktree_execution,
            provision_worktree_for_execution,
            persist_provisioned_worktree,
            cleanup_task_worktrees,
            ensure_task_commit_hook,
            emit_workspace_task_upsert,
        })
    }

    pub(in crate::daemon) fn managed_worktree_root(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
    ) -> Option<PathBuf> {
        ctx_worktree_vcs_service::matching_managed_worktree_path(
            &self.data_root,
            workspace.id,
            worktree.id,
            PathBuf::from(&worktree.root_path),
        )
    }

    pub(in crate::daemon) async fn resolve_existing_worktree_execution(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree_id: WorktreeId,
    ) -> anyhow::Result<crate::daemon::workspaces::ResolvedExistingWorktreeExecution> {
        (self.resolve_existing_worktree_execution)(store.clone(), workspace.clone(), worktree_id)
            .await
    }

    pub(in crate::daemon) async fn provision_worktree_for_execution(
        &self,
        workspace: &Workspace,
        worktree_id: WorktreeId,
        base_commit_sha: &str,
        branch_name: &str,
        effective: &ExecutionSettings,
    ) -> anyhow::Result<(PathBuf, Option<SandboxBinding>)> {
        (self.provision_worktree_for_execution)(
            workspace.clone(),
            worktree_id,
            base_commit_sha.to_string(),
            branch_name.to_string(),
            effective.clone(),
        )
        .await
    }

    pub(in crate::daemon) async fn persist_provisioned_worktree(
        &self,
        store: &Store,
        workspace: &Workspace,
        worktree: Worktree,
        sandbox_binding: Option<SandboxBinding>,
    ) -> anyhow::Result<Worktree> {
        (self.persist_provisioned_worktree)(
            store.clone(),
            workspace.clone(),
            worktree,
            sandbox_binding,
        )
        .await
    }

    pub(in crate::daemon) async fn cleanup_task_worktrees(
        &self,
        workspace: &Workspace,
        task_id: TaskId,
        targets: &[crate::daemon::workspaces::TaskWorktreeCleanupTarget],
        mode: crate::daemon::workspaces::BranchCleanupErrorMode,
    ) -> Vec<anyhow::Error> {
        (self.cleanup_task_worktrees)(workspace.clone(), task_id, targets.to_vec(), mode).await
    }

    pub(in crate::daemon) async fn ensure_task_commit_hook(
        &self,
        workspace: &Workspace,
        worktree: &Worktree,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        (self.ensure_task_commit_hook)(workspace.clone(), worktree.clone(), task_id).await
    }

    pub(in crate::daemon) async fn emit_workspace_task_upsert(
        &self,
        task_id: TaskId,
    ) -> anyhow::Result<()> {
        (self.emit_workspace_task_upsert)(task_id).await
    }
}

pub(in crate::daemon) struct TaskAdmissionSessionEffects {
    publish_event:
        Arc<dyn Fn(ctx_core::models::SessionEvent) -> TaskAdmissionFuture<()> + Send + Sync>,
    ensure_scheduler: Arc<
        dyn Fn(
                Session,
            )
                -> TaskAdmissionFuture<mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>>
            + Send
            + Sync,
    >,
    schedule_title_generation:
        Arc<dyn Fn(Session, String, bool) -> TaskAdmissionFuture<bool> + Send + Sync>,
}

impl TaskAdmissionSessionEffects {
    pub(in crate::daemon) fn new(
        publish_event: Arc<
            dyn Fn(ctx_core::models::SessionEvent) -> TaskAdmissionFuture<()> + Send + Sync,
        >,
        ensure_scheduler: Arc<
            dyn Fn(
                    Session,
                )
                    -> TaskAdmissionFuture<mpsc::Sender<crate::daemon::scheduler::SchedulerCommand>>
                + Send
                + Sync,
        >,
        schedule_title_generation: Arc<
            dyn Fn(Session, String, bool) -> TaskAdmissionFuture<bool> + Send + Sync,
        >,
    ) -> Arc<Self> {
        Arc::new(Self {
            publish_event,
            ensure_scheduler,
            schedule_title_generation,
        })
    }

    pub(in crate::daemon) async fn publish_event(&self, event: ctx_core::models::SessionEvent) {
        (self.publish_event)(event).await;
    }

    pub(in crate::daemon) async fn ensure_scheduler(
        &self,
        session: Session,
    ) -> mpsc::Sender<crate::daemon::scheduler::SchedulerCommand> {
        (self.ensure_scheduler)(session).await
    }

    pub(in crate::daemon) async fn schedule_title_generation(
        &self,
        session: Session,
        prompt: String,
        force: bool,
    ) -> bool {
        (self.schedule_title_generation)(session, prompt, force).await
    }
}

#[derive(Clone)]
pub struct ProviderBootstrapHandle {
    data_root: PathBuf,
    workspace_stores: ProtectedWorkspaceStoreLookup,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderBootstrapHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        workspace_stores: ProtectedWorkspaceStoreLookup,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            workspace_stores,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) fn global_store(&self) -> &Store {
        self.workspace_stores.global_store()
    }

    pub(in crate::daemon) async fn store_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> anyhow::Result<Store> {
        self.workspace_stores
            .store_for_workspace(workspace_id)
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
pub struct ProviderOptionsHandle {
    launch: Arc<ProviderWorkspaceLaunchRuntime>,
}

impl ProviderOptionsHandle {
    pub(in crate::daemon) fn new(launch: Arc<ProviderWorkspaceLaunchRuntime>) -> Self {
        Self { launch }
    }

    pub(in crate::daemon) fn launch(&self) -> &ProviderWorkspaceLaunchRuntime {
        self.launch.as_ref()
    }
}

#[derive(Clone)]
pub struct ProviderWorkspaceAuthHandle {
    launch: Arc<ProviderWorkspaceLaunchRuntime>,
}

impl ProviderWorkspaceAuthHandle {
    pub(in crate::daemon) fn new(launch: Arc<ProviderWorkspaceLaunchRuntime>) -> Self {
        Self { launch }
    }

    pub(in crate::daemon) fn launch(&self) -> &ProviderWorkspaceLaunchRuntime {
        self.launch.as_ref()
    }
}

#[derive(Clone)]
pub struct ProviderStatusHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderStatusHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }
}

#[derive(Clone)]
pub struct ProviderAdminHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderAdminHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }
}

#[derive(Clone)]
pub struct ProviderInstallHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    ops_events: OpsEvents,
}

impl ProviderInstallHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        ops_events: OpsEvents,
    ) -> Self {
        Self {
            data_root,
            providers,
            ops_events,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn ops_events(&self) -> &OpsEvents {
        &self.ops_events
    }

    pub(in crate::daemon) async fn get_install_polling_info(
        &self,
        install_id: InstallId,
    ) -> Option<InstallInfo> {
        let outcome = self.providers.get_install_polling_info(install_id).await;
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            &self.ops_events,
            outcome.ops_events,
        );
        outcome.info
    }

    pub(in crate::daemon) async fn cancel_install(
        &self,
        install_id: InstallId,
    ) -> Option<InstallInfo> {
        let outcome = self.providers.cancel_install(install_id).await?;
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            &self.ops_events,
            outcome.ops_events,
        );
        Some(outcome.info)
    }

    pub(in crate::daemon) async fn list_install_events(
        &self,
        install_id: InstallId,
    ) -> Option<Vec<InstallProgressEvent>> {
        let outcome = self.providers.get_install_events(install_id).await;
        crate::daemon::provider_capability_hosts::emit_provider_install_ops_events(
            &self.ops_events,
            outcome.ops_events,
        );
        outcome.events
    }

    pub(in crate::daemon) async fn install_event_sender(
        &self,
        install_id: InstallId,
    ) -> Option<broadcast::Sender<InstallProgressEvent>> {
        self.providers.get_install_sender(install_id).await
    }
}

#[derive(Clone)]
pub struct ProviderAuthImportHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
}

impl ProviderAuthImportHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf, providers: Arc<ProviderRuntime>) -> Self {
        Self {
            data_root,
            providers,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }
}

#[derive(Clone)]
pub struct ProviderUsageHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
    shutdown_tx: broadcast::Sender<()>,
}

impl ProviderUsageHandle {
    pub(in crate::daemon) fn new(
        data_root: PathBuf,
        providers: Arc<ProviderRuntime>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Self {
        Self {
            data_root,
            providers,
            shutdown_tx,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
    }

    pub(in crate::daemon) fn shutdown_tx(&self) -> &broadcast::Sender<()> {
        &self.shutdown_tx
    }
}

#[derive(Clone)]
pub struct ProviderHarnessConfigHandle {
    data_root: PathBuf,
    providers: Arc<ProviderRuntime>,
}

impl ProviderHarnessConfigHandle {
    pub(in crate::daemon) fn new(data_root: PathBuf, providers: Arc<ProviderRuntime>) -> Self {
        Self {
            data_root,
            providers,
        }
    }

    pub(in crate::daemon) fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub(in crate::daemon) fn providers(&self) -> &ProviderRuntime {
        self.providers.as_ref()
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

pub(in crate::daemon) type CreateTerminalFuture =
    Pin<Box<dyn Future<Output = Result<TerminalSession, TerminalLaunchError>> + Send + 'static>>;
pub(in crate::daemon) type CreateTerminalEffect =
    Arc<dyn Fn(CreateTerminalLaunchRequest) -> CreateTerminalFuture + Send + Sync>;

#[derive(Clone)]
pub struct TerminalRouteHandle {
    terminals: Arc<TerminalManager>,
    create_terminal: CreateTerminalEffect,
}

impl TerminalRouteHandle {
    pub(in crate::daemon) fn new(
        terminals: Arc<TerminalManager>,
        create_terminal: CreateTerminalEffect,
    ) -> Self {
        Self {
            terminals,
            create_terminal,
        }
    }

    pub(in crate::daemon) fn terminals(&self) -> &TerminalManager {
        self.terminals.as_ref()
    }

    pub(in crate::daemon) async fn create_terminal(
        &self,
        req: CreateTerminalLaunchRequest,
    ) -> Result<TerminalSession, TerminalLaunchError> {
        (self.create_terminal)(req).await
    }
}

pub(in crate::daemon) type CreateWebSessionFuture =
    Pin<Box<dyn Future<Output = Result<WebSessionInfo, WebSessionLaunchError>> + Send + 'static>>;
pub(in crate::daemon) type CreateWebSessionEffect =
    Arc<dyn Fn(WebSessionLaunchRequest) -> CreateWebSessionFuture + Send + Sync>;

#[derive(Clone)]
pub struct WebSessionRouteHandle {
    web_sessions: Arc<WebSessionManager>,
    create_web_session: CreateWebSessionEffect,
}

impl WebSessionRouteHandle {
    pub(in crate::daemon) fn new(
        web_sessions: Arc<WebSessionManager>,
        create_web_session: CreateWebSessionEffect,
    ) -> Self {
        Self {
            web_sessions,
            create_web_session,
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
        (self.create_web_session)(req).await
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

type DaemonShutdownFuture = Pin<
    Box<
        dyn Future<
                Output = Result<
                    crate::daemon::DaemonTurnActivitySummary,
                    crate::daemon::maintenance::DaemonShutdownError,
                >,
            > + Send
            + 'static,
    >,
>;
type DaemonShutdownEffect = Arc<dyn Fn(String) -> DaemonShutdownFuture + Send + Sync>;

#[derive(Clone)]
pub struct DaemonShutdownHandle {
    local_shutdown_token: Option<String>,
    request_shutdown: DaemonShutdownEffect,
}

impl DaemonShutdownHandle {
    pub(in crate::daemon) fn new(
        local_shutdown_token: Option<String>,
        request_shutdown: DaemonShutdownEffect,
    ) -> Self {
        Self {
            local_shutdown_token,
            request_shutdown,
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
        (self.request_shutdown)(reason).await
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
