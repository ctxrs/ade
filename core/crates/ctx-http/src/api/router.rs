use axum::extract::FromRef;
use axum::http::{header, HeaderValue, Method};
use axum::middleware;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

use ctx_daemon::daemon::{
    AuthHandle, BlobHandle, DaemonHandle, DaemonShutdownHandle, DemoSeedTranscriptHandle,
    DiagnosticsHandle, DictationHandle, ExecutionLaunchHandle, HealthHandle,
    LinuxSandboxRuntimeHandle, LogsHandle, MergeQueueApiHandle, MobileRuntimeHandle,
    MobileSecureProxyHandle, MobileStoreHandle, OrgPolicyHandle, ProviderAccountsHandle,
    ProviderAdminHandle, ProviderAuthImportHandle, ProviderBootstrapHandle,
    ProviderHarnessConfigHandle, ProviderInstallHandle, ProviderOptionsHandle,
    ProviderStatusHandle, ProviderUsageHandle, ProviderWorkspaceAuthHandle, RepoOnboardingHandle,
    RequestBaseHandle, ResourceUtilizationHandle, RunArchiveHandle, SessionArtifactsHandle,
    SessionControlHandle, SessionFileCompletionsHandle, SessionMessageCommandHandle,
    SessionReadModelsHandle, SessionSubagentMcpControlHandle, SessionSubagentMcpReadHandle,
    SessionSubagentReadHandle, SessionTitleModelModeHandle, SessionVcsHandle, SettingsHandle,
    TaskCreationHandle, TaskLifecycleHandle, TaskListingHandle, TaskReadStateHandle,
    TaskSessionAdmissionHandle, TaskSessionListingHandle, TaskTitleHandle, TelemetryHandle,
    TerminalRouteHandle, TitleGenerationLocalHandle, UpdateActivityHandle, UpdateDrainHandle,
    UpdateReleaseHandle, WebSessionRouteHandle, WorkspaceActiveHandle, WorkspaceAttachmentsHandle,
    WorkspaceDeletionHandle, WorkspaceExecutionConfigHandle, WorkspaceFileCompletionsHandle,
    WorkspaceHarnessContainerHandle, WorkspaceMergeQueueConfigHandle, WorkspaceOrgPolicyHandle,
    WorkspacePrimaryBranchHandle, WorkspacePromptBootstrapConfigHandle,
    WorkspaceProviderModelPreferenceHandle, WorkspaceRegistryHandle, WorkspaceStreamHandle,
    WorkspaceVcsStreamHandle, WorkspaceWorktreeHandle,
};

use super::auth::auth_middleware;
use super::perf::perf_middleware;
use super::request_base::is_loopback_host;
use super::routes;

fn is_allowed_desktop_worker_origin(origin: &HeaderValue) -> bool {
    let Ok(raw) = origin.to_str() else {
        return false;
    };
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("tauri://localhost") {
        return true;
    }
    let Ok(url) = Url::parse(trimmed) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    match url.scheme() {
        "http" | "https" => is_loopback_host(host),
        "tauri" => host.eq_ignore_ascii_case("localhost"),
        _ => false,
    }
}

fn daemon_cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            is_allowed_desktop_worker_origin(origin)
        }))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::HeaderName::from_static("traceparent"),
            header::HeaderName::from_static("x-ctx-run-id"),
        ])
}

macro_rules! impl_route_state_extractors {
    ($($name:ident, $accessor:ident);+ $(;)?) => {
        $(
            impl FromRef<RouteState> for $name {
                fn from_ref(state: &RouteState) -> Self {
                    state.handles.$accessor.clone()
                }
            }
        )+
    };
}

#[derive(Clone)]
pub struct RouteHandles {
    auth: AuthHandle,
    health: HealthHandle,
    diagnostics: DiagnosticsHandle,
    blob: BlobHandle,
    request_base: RequestBaseHandle,
    repo_onboarding: RepoOnboardingHandle,
    logs: LogsHandle,
    org_policy: OrgPolicyHandle,
    workspace_org_policy: WorkspaceOrgPolicyHandle,
    workspace_prompt_bootstrap_config: WorkspacePromptBootstrapConfigHandle,
    workspace_execution_config: WorkspaceExecutionConfigHandle,
    workspace_file_completions: WorkspaceFileCompletionsHandle,
    workspace_harness_container: WorkspaceHarnessContainerHandle,
    workspace_provider_model_preferences: WorkspaceProviderModelPreferenceHandle,
    workspace_worktree: WorkspaceWorktreeHandle,
    workspace_registry: WorkspaceRegistryHandle,
    workspace_merge_queue_config: WorkspaceMergeQueueConfigHandle,
    merge_queue_api: MergeQueueApiHandle,
    workspace_attachments: WorkspaceAttachmentsHandle,
    workspace_primary_branch: WorkspacePrimaryBranchHandle,
    dictation: DictationHandle,
    update_release: UpdateReleaseHandle,
    update_activity: UpdateActivityHandle,
    settings: SettingsHandle,
    mobile_store: MobileStoreHandle,
    mobile_runtime: MobileRuntimeHandle,
    mobile_secure_proxy: MobileSecureProxyHandle,
    resource_utilization: ResourceUtilizationHandle,
    run_archive: RunArchiveHandle,
    session_artifacts: SessionArtifactsHandle,
    session_control: SessionControlHandle,
    session_file_completions: SessionFileCompletionsHandle,
    session_message_command: SessionMessageCommandHandle,
    session_read_models: SessionReadModelsHandle,
    session_subagent_mcp_read: SessionSubagentMcpReadHandle,
    session_subagent_mcp_control: SessionSubagentMcpControlHandle,
    session_subagent_read: SessionSubagentReadHandle,
    session_title_model_mode: SessionTitleModelModeHandle,
    session_vcs: SessionVcsHandle,
    demo_seed_transcript: DemoSeedTranscriptHandle,
    title_generation_local: TitleGenerationLocalHandle,
    task_creation: TaskCreationHandle,
    task_lifecycle: TaskLifecycleHandle,
    task_listing: TaskListingHandle,
    task_read_state: TaskReadStateHandle,
    task_session_admission: TaskSessionAdmissionHandle,
    task_session_listing: TaskSessionListingHandle,
    task_title: TaskTitleHandle,
    workspace_deletion: WorkspaceDeletionHandle,
    workspace_active: WorkspaceActiveHandle,
    workspace_stream: WorkspaceStreamHandle,
    workspace_vcs_stream: WorkspaceVcsStreamHandle,
    provider_accounts: ProviderAccountsHandle,
    provider_auth_import: ProviderAuthImportHandle,
    provider_status: ProviderStatusHandle,
    provider_admin: ProviderAdminHandle,
    provider_install: ProviderInstallHandle,
    provider_usage: ProviderUsageHandle,
    provider_harness_config: ProviderHarnessConfigHandle,
    provider_bootstrap: ProviderBootstrapHandle,
    provider_options: ProviderOptionsHandle,
    provider_workspace_auth: ProviderWorkspaceAuthHandle,
    telemetry: TelemetryHandle,
    terminal_route: TerminalRouteHandle,
    web_session_route: WebSessionRouteHandle,
    execution_launch: ExecutionLaunchHandle,
    linux_sandbox_runtime: LinuxSandboxRuntimeHandle,
    update_drain: UpdateDrainHandle,
    daemon_shutdown: DaemonShutdownHandle,
}

impl RouteHandles {
    pub fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self {
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
}

#[derive(Clone)]
pub(in crate::api) struct RouteState {
    handles: RouteHandles,
}

impl_route_state_extractors! {
    AuthHandle, auth;
    HealthHandle, health;
    DiagnosticsHandle, diagnostics;
    BlobHandle, blob;
    RequestBaseHandle, request_base;
    RepoOnboardingHandle, repo_onboarding;
    LogsHandle, logs;
    OrgPolicyHandle, org_policy;
    WorkspaceOrgPolicyHandle, workspace_org_policy;
    WorkspacePromptBootstrapConfigHandle, workspace_prompt_bootstrap_config;
    WorkspaceExecutionConfigHandle, workspace_execution_config;
    WorkspaceFileCompletionsHandle, workspace_file_completions;
    WorkspaceHarnessContainerHandle, workspace_harness_container;
    WorkspaceProviderModelPreferenceHandle, workspace_provider_model_preferences;
    WorkspaceWorktreeHandle, workspace_worktree;
    WorkspaceRegistryHandle, workspace_registry;
    WorkspaceMergeQueueConfigHandle, workspace_merge_queue_config;
    MergeQueueApiHandle, merge_queue_api;
    WorkspaceAttachmentsHandle, workspace_attachments;
    WorkspacePrimaryBranchHandle, workspace_primary_branch;
    DictationHandle, dictation;
    UpdateReleaseHandle, update_release;
    UpdateActivityHandle, update_activity;
    SettingsHandle, settings;
    MobileStoreHandle, mobile_store;
    MobileRuntimeHandle, mobile_runtime;
    MobileSecureProxyHandle, mobile_secure_proxy;
    ResourceUtilizationHandle, resource_utilization;
    RunArchiveHandle, run_archive;
    SessionArtifactsHandle, session_artifacts;
    SessionControlHandle, session_control;
    SessionFileCompletionsHandle, session_file_completions;
    SessionMessageCommandHandle, session_message_command;
    SessionReadModelsHandle, session_read_models;
    SessionSubagentMcpReadHandle, session_subagent_mcp_read;
    SessionSubagentMcpControlHandle, session_subagent_mcp_control;
    SessionSubagentReadHandle, session_subagent_read;
    SessionTitleModelModeHandle, session_title_model_mode;
    SessionVcsHandle, session_vcs;
    DemoSeedTranscriptHandle, demo_seed_transcript;
    TitleGenerationLocalHandle, title_generation_local;
    TaskCreationHandle, task_creation;
    TaskLifecycleHandle, task_lifecycle;
    TaskListingHandle, task_listing;
    TaskReadStateHandle, task_read_state;
    TaskSessionAdmissionHandle, task_session_admission;
    TaskSessionListingHandle, task_session_listing;
    TaskTitleHandle, task_title;
    WorkspaceDeletionHandle, workspace_deletion;
    WorkspaceActiveHandle, workspace_active;
    WorkspaceStreamHandle, workspace_stream;
    WorkspaceVcsStreamHandle, workspace_vcs_stream;
    ProviderAccountsHandle, provider_accounts;
    ProviderAuthImportHandle, provider_auth_import;
    ProviderStatusHandle, provider_status;
    ProviderAdminHandle, provider_admin;
    ProviderInstallHandle, provider_install;
    ProviderUsageHandle, provider_usage;
    ProviderHarnessConfigHandle, provider_harness_config;
    ProviderBootstrapHandle, provider_bootstrap;
    ProviderOptionsHandle, provider_options;
    ProviderWorkspaceAuthHandle, provider_workspace_auth;
    TelemetryHandle, telemetry;
    TerminalRouteHandle, terminal_route;
    WebSessionRouteHandle, web_session_route;
    ExecutionLaunchHandle, execution_launch;
    LinuxSandboxRuntimeHandle, linux_sandbox_runtime;
    UpdateDrainHandle, update_drain;
    DaemonShutdownHandle, daemon_shutdown;
}

pub fn router(handles: RouteHandles) -> axum::Router {
    let state = RouteState { handles };
    let auth_state = state.clone();
    let perf_state = state.clone();
    let api = routes::api_routes()
        .route_layer(middleware::from_fn_with_state(perf_state, perf_middleware))
        .layer(middleware::from_fn_with_state(auth_state, auth_middleware))
        .layer(daemon_cors_layer())
        .with_state(state);

    let dist_dir = std::env::var("CTX_WEB_DIST").unwrap_or_else(|_| "apps/web/dist".into());
    let index_path = format!("{dist_dir}/index.html");
    api.fallback_service(ServeDir::new(dist_dir).not_found_service(ServeFile::new(index_path)))
}
