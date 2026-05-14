use axum::extract::FromRef;
use axum::http::{header, HeaderValue, Method};
use axum::middleware;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use url::Url;

use ctx_daemon::daemon::{
    CoreHandle, DaemonHandle, ExecutionHandle, ProvidersHandle, SessionsHandle, TasksHandle,
    TelemetryHandle, TransportHandle, WorkspaceStreamHandle, WorkspacesHandle,
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
                    state.handle.$accessor()
                }
            }
        )+
    };
}

#[derive(Clone)]
pub(in crate::api) struct RouteState {
    pub(in crate::api) handle: DaemonHandle,
}

impl_route_state_extractors! {
    CoreHandle, core;
    SessionsHandle, sessions;
    TasksHandle, tasks;
    WorkspacesHandle, workspaces;
    WorkspaceStreamHandle, workspace_stream;
    ProvidersHandle, providers;
    TelemetryHandle, telemetry;
    TransportHandle, transport;
    ExecutionHandle, execution;
}

pub fn router(state: impl Into<DaemonHandle>) -> axum::Router {
    let state = RouteState {
        handle: state.into(),
    };
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
