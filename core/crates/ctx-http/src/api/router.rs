use axum::extract::FromRef;
use axum::http::{header, HeaderValue, Method};
use axum::middleware;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

use ctx_daemon::daemon::{
    AuthHandle, BlobHandle, DaemonHandle, DiagnosticsHandle, DictationHandle, ExecutionHandle,
    ExecutionLaunchHandle, HealthHandle, LinuxSandboxRuntimeHandle, LogsHandle,
    MobileRuntimeHandle, MobileSecureProxyHandle, MobileStoreHandle, OrgPolicyHandle,
    ProviderAccountsHandle, ProviderAdminHandle, ProviderAuthImportHandle, ProviderBootstrapHandle,
    ProviderHarnessConfigHandle, ProviderInstallHandle, ProviderOptionsHandle,
    ProviderStatusHandle, ProviderUsageHandle, ProviderWorkspaceAuthHandle, RepoOnboardingHandle,
    RequestBaseHandle, ResourceUtilizationHandle, RunArchiveHandle, SessionArtifactsHandle,
    SessionVcsHandle, SessionsHandle, SettingsHandle, TaskCreationHandle, TaskLifecycleHandle,
    TaskListingHandle, TaskReadStateHandle, TaskSessionAdmissionHandle, TaskSessionListingHandle,
    TaskTitleHandle, TelemetryHandle, TransportHandle, UpdateActivityHandle, UpdateDrainHandle,
    UpdateReleaseHandle, WorkspaceActiveHandle, WorkspaceExecutionConfigHandle,
    WorkspaceFileCompletionsHandle, WorkspaceHarnessContainerHandle, WorkspaceOrgPolicyHandle,
    WorkspacePromptBootstrapConfigHandle, WorkspaceProviderModelPreferenceHandle,
    WorkspaceStreamHandle, WorkspacesHandle,
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
    pub(in crate::api) auth: AuthHandle,
    pub(in crate::api) health: HealthHandle,
    pub(in crate::api) diagnostics: DiagnosticsHandle,
    pub(in crate::api) blob: BlobHandle,
    pub(in crate::api) request_base: RequestBaseHandle,
    pub(in crate::api) repo_onboarding: RepoOnboardingHandle,
    pub(in crate::api) logs: LogsHandle,
    pub(in crate::api) org_policy: OrgPolicyHandle,
    pub(in crate::api) workspace_org_policy: WorkspaceOrgPolicyHandle,
    pub(in crate::api) workspace_prompt_bootstrap_config: WorkspacePromptBootstrapConfigHandle,
    pub(in crate::api) workspace_execution_config: WorkspaceExecutionConfigHandle,
    pub(in crate::api) workspace_file_completions: WorkspaceFileCompletionsHandle,
    pub(in crate::api) workspace_harness_container: WorkspaceHarnessContainerHandle,
    pub(in crate::api) workspace_provider_model_preferences: WorkspaceProviderModelPreferenceHandle,
    pub(in crate::api) dictation: DictationHandle,
    pub(in crate::api) update_release: UpdateReleaseHandle,
    pub(in crate::api) update_activity: UpdateActivityHandle,
    pub(in crate::api) settings: SettingsHandle,
    pub(in crate::api) mobile_store: MobileStoreHandle,
    pub(in crate::api) mobile_runtime: MobileRuntimeHandle,
    pub(in crate::api) mobile_secure_proxy: MobileSecureProxyHandle,
    pub(in crate::api) resource_utilization: ResourceUtilizationHandle,
    pub(in crate::api) run_archive: RunArchiveHandle,
    pub(in crate::api) session_artifacts: SessionArtifactsHandle,
    pub(in crate::api) session_vcs: SessionVcsHandle,
    pub(in crate::api) sessions: SessionsHandle,
    pub(in crate::api) task_creation: TaskCreationHandle,
    pub(in crate::api) task_lifecycle: TaskLifecycleHandle,
    pub(in crate::api) task_listing: TaskListingHandle,
    pub(in crate::api) task_read_state: TaskReadStateHandle,
    pub(in crate::api) task_session_admission: TaskSessionAdmissionHandle,
    pub(in crate::api) task_session_listing: TaskSessionListingHandle,
    pub(in crate::api) task_title: TaskTitleHandle,
    pub(in crate::api) workspaces: WorkspacesHandle,
    pub(in crate::api) workspace_active: WorkspaceActiveHandle,
    pub(in crate::api) workspace_stream: WorkspaceStreamHandle,
    pub(in crate::api) provider_accounts: ProviderAccountsHandle,
    pub(in crate::api) provider_auth_import: ProviderAuthImportHandle,
    pub(in crate::api) provider_status: ProviderStatusHandle,
    pub(in crate::api) provider_admin: ProviderAdminHandle,
    pub(in crate::api) provider_install: ProviderInstallHandle,
    pub(in crate::api) provider_usage: ProviderUsageHandle,
    pub(in crate::api) provider_harness_config: ProviderHarnessConfigHandle,
    pub(in crate::api) provider_bootstrap: ProviderBootstrapHandle,
    pub(in crate::api) provider_options: ProviderOptionsHandle,
    pub(in crate::api) provider_workspace_auth: ProviderWorkspaceAuthHandle,
    pub(in crate::api) telemetry: TelemetryHandle,
    pub(in crate::api) transport: TransportHandle,
    pub(in crate::api) execution_launch: ExecutionLaunchHandle,
    pub(in crate::api) linux_sandbox_runtime: LinuxSandboxRuntimeHandle,
    pub(in crate::api) update_drain: UpdateDrainHandle,
    pub(in crate::api) execution: ExecutionHandle,
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
            session_vcs: handle.session_vcs(),
            sessions: handle.sessions(),
            task_creation: handle.task_creation(),
            task_lifecycle: handle.task_lifecycle(),
            task_listing: handle.task_listing(),
            task_read_state: handle.task_read_state(),
            task_session_admission: handle.task_session_admission(),
            task_session_listing: handle.task_session_listing(),
            task_title: handle.task_title(),
            workspaces: handle.workspaces(),
            workspace_active: handle.workspace_active(),
            workspace_stream: handle.workspace_stream(),
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
            transport: handle.transport(),
            execution_launch: handle.execution_launch(),
            linux_sandbox_runtime: handle.linux_sandbox_runtime(),
            update_drain: handle.update_drain(),
            execution: handle.execution(),
        }
    }
}

#[derive(Clone)]
pub(in crate::api) struct RouteState {
    pub(in crate::api) handles: RouteHandles,
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
    SessionVcsHandle, session_vcs;
    SessionsHandle, sessions;
    TaskCreationHandle, task_creation;
    TaskLifecycleHandle, task_lifecycle;
    TaskListingHandle, task_listing;
    TaskReadStateHandle, task_read_state;
    TaskSessionAdmissionHandle, task_session_admission;
    TaskSessionListingHandle, task_session_listing;
    TaskTitleHandle, task_title;
    WorkspacesHandle, workspaces;
    WorkspaceActiveHandle, workspace_active;
    WorkspaceStreamHandle, workspace_stream;
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
    TransportHandle, transport;
    ExecutionLaunchHandle, execution_launch;
    LinuxSandboxRuntimeHandle, linux_sandbox_runtime;
    UpdateDrainHandle, update_drain;
    ExecutionHandle, execution;
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
