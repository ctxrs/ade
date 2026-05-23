use axum::extract::FromRef;
use axum::http::{header, HeaderValue, Method};
use axum::middleware;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

use ctx_daemon::daemon::{
    AuthHandle, BlobHandle, CoreHandle, DaemonHandle, DiagnosticsHandle, DictationHandle,
    ExecutionHandle, HealthHandle, LogsHandle, MobileStoreHandle, OrgPolicyHandle, ProvidersHandle,
    RequestBaseHandle, SessionsHandle, SettingsHandle, TasksHandle, TelemetryHandle,
    TransportHandle, UpdateReleaseHandle, WorkspaceStreamHandle, WorkspacesHandle,
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
    pub(in crate::api) core: CoreHandle,
    pub(in crate::api) auth: AuthHandle,
    pub(in crate::api) health: HealthHandle,
    pub(in crate::api) diagnostics: DiagnosticsHandle,
    pub(in crate::api) blob: BlobHandle,
    pub(in crate::api) request_base: RequestBaseHandle,
    pub(in crate::api) logs: LogsHandle,
    pub(in crate::api) org_policy: OrgPolicyHandle,
    pub(in crate::api) dictation: DictationHandle,
    pub(in crate::api) update_release: UpdateReleaseHandle,
    pub(in crate::api) settings: SettingsHandle,
    pub(in crate::api) mobile_store: MobileStoreHandle,
    pub(in crate::api) sessions: SessionsHandle,
    pub(in crate::api) tasks: TasksHandle,
    pub(in crate::api) workspaces: WorkspacesHandle,
    pub(in crate::api) workspace_stream: WorkspaceStreamHandle,
    pub(in crate::api) providers: ProvidersHandle,
    pub(in crate::api) telemetry: TelemetryHandle,
    pub(in crate::api) transport: TransportHandle,
    pub(in crate::api) execution: ExecutionHandle,
}

impl RouteHandles {
    pub fn from_daemon_handle(handle: DaemonHandle) -> Self {
        Self {
            core: handle.core(),
            auth: handle.auth(),
            health: handle.health(),
            diagnostics: handle.diagnostics(),
            blob: handle.blob(),
            request_base: handle.request_base(),
            logs: handle.logs(),
            org_policy: handle.org_policy(),
            dictation: handle.dictation(),
            update_release: handle.update_release(),
            settings: handle.settings(),
            mobile_store: handle.mobile_store(),
            sessions: handle.sessions(),
            tasks: handle.tasks(),
            workspaces: handle.workspaces(),
            workspace_stream: handle.workspace_stream(),
            providers: handle.providers(),
            telemetry: handle.telemetry(),
            transport: handle.transport(),
            execution: handle.execution(),
        }
    }
}

#[derive(Clone)]
pub(in crate::api) struct RouteState {
    pub(in crate::api) handles: RouteHandles,
}

impl_route_state_extractors! {
    CoreHandle, core;
    AuthHandle, auth;
    HealthHandle, health;
    DiagnosticsHandle, diagnostics;
    BlobHandle, blob;
    RequestBaseHandle, request_base;
    LogsHandle, logs;
    OrgPolicyHandle, org_policy;
    DictationHandle, dictation;
    UpdateReleaseHandle, update_release;
    SettingsHandle, settings;
    MobileStoreHandle, mobile_store;
    SessionsHandle, sessions;
    TasksHandle, tasks;
    WorkspacesHandle, workspaces;
    WorkspaceStreamHandle, workspace_stream;
    ProvidersHandle, providers;
    TelemetryHandle, telemetry;
    TransportHandle, transport;
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
