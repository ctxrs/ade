use super::*;
use crate::api::shared::store_for_existing_workspace_status;
use ctx_linux_sandbox_runtime::linux_sandbox_runtime_status;

#[derive(Debug, Deserialize)]
pub(in crate::api) struct MergeQueueSubmitReq {
    #[serde(default)]
    pub(in crate::api) session_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) worktree_id: Option<String>,
    #[serde(default)]
    pub(in crate::api) worktree_root: Option<String>,
    #[serde(default)]
    pub(in crate::api) target_branch: Option<String>,
    #[serde(default)]
    pub(in crate::api) message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct MergeQueueListParams {
    pub(in crate::api) workspace_id: String,
    #[serde(default)]
    pub(in crate::api) limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct CreateMobileConnectionProfileReq {
    pub(in crate::api) label: String,
    pub(in crate::api) base_url: String,
    #[serde(default)]
    pub(in crate::api) scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct CreateMobileConnectionProfileResp {
    pub(in crate::api) profile: MobileConnectionProfile,
    pub(in crate::api) token: String,
    pub(in crate::api) qr_payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct RegisterMobileDeviceReq {
    pub(in crate::api) device_id: String,
    #[serde(default)]
    pub(in crate::api) device_label: Option<String>,
    #[serde(default)]
    pub(in crate::api) platform: Option<String>,
    #[serde(default)]
    pub(in crate::api) push_token: Option<String>,
    #[serde(default)]
    pub(in crate::api) push_provider: Option<String>,
    #[serde(default)]
    pub(in crate::api) public_key: Option<String>,
    #[serde(default)]
    pub(in crate::api) app_version: Option<String>,
}

pub(in crate::api) async fn health(
    State(state): State<Arc<AppState>>,
) -> Result<Json<HealthResp>, StatusCode> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let build_id = option_env!("CTX_BUILD_ID")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();
    let dev_instance_id = option_env!("CTX_DEV_INSTANCE_ID")
        .unwrap_or("unknown")
        .to_string();
    Ok(Json(HealthResp {
        version: version.clone(),
        daemon_version: version.clone(),
        pid: std::process::id(),
        data_root: state.core.data_root.to_string_lossy().to_string(),
        daemon_url: state.core.daemon_url.clone(),
        auth_required: state.core.auth_token.is_some(),
        storage: state.storage_guard_snapshot(),
        compatibility: HealthCompatibility {
            desktop_exact_version: version,
            desktop_build_id: build_id,
            desktop_dev_instance_id: dev_instance_id,
            mobile_api_min: MOBILE_API_MIN_VERSION,
            mobile_api_max: MOBILE_API_MAX_VERSION,
        },
    }))
}

pub(in crate::api) const TITLE_GENERATION_LOCAL_INSTALL_KEY: &str = "title_generation_local";

#[derive(Debug, Serialize)]
pub(in crate::api) struct TitleGenerationLocalStatusResponse {
    pub ready: bool,
    pub runtime: title_generation_local::TitleGenerationLocalRuntimeStatus,
    pub model: title_generation_local::TitleGenerationLocalModelStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_id: Option<InstallId>,
    pub install_running: bool,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct TitleGenerationLocalInstallResponse {
    pub install_id: InstallId,
}

pub(in crate::api) async fn get_title_generation_local_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalStatusResponse>, StatusCode> {
    let status = title_generation_local::local_status(&state.core.data_root)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let install_id = state
        .find_running_install(TITLE_GENERATION_LOCAL_INSTALL_KEY, None)
        .await;
    Ok(Json(TitleGenerationLocalStatusResponse {
        ready: status.ready,
        runtime: status.runtime,
        model: status.model,
        install_id,
        install_running: install_id.is_some(),
    }))
}

pub(in crate::api) async fn install_title_generation_local(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TitleGenerationLocalInstallResponse>, StatusCode> {
    let (install_id, started_new) = state
        .start_install(TITLE_GENERATION_LOCAL_INSTALL_KEY.to_string(), None)
        .await;
    if started_new {
        let state2 = state.clone();
        tokio::spawn(async move {
            if let Err(e) =
                installer::install_title_generation_local_with_progress(state2.clone(), install_id)
                    .await
            {
                tracing::error!("local title generation install failed: {e:#}");
            }
        });
    }

    Ok(Json(TitleGenerationLocalInstallResponse { install_id }))
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct DiagnosticsResp {
    daemon: HealthResp,
    platform: serde_json::Value,
    logs: serde_json::Value,
    execution: serde_json::Value,
    providers: Vec<ProviderStatus>,
    managed_installs: serde_json::Value,
}

pub(in crate::api) async fn diagnostics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<DiagnosticsResp>, StatusCode> {
    let startup_prewarm = state.execution.setup.startup_status().await;
    let linux_sandbox_runtime = linux_sandbox_runtime_status(&state.core.data_root)
        .await
        .map(|status| serde_json::to_value(status).unwrap_or_else(|_| serde_json::json!({})))
        .unwrap_or_else(
            |err| serde_json::json!({"error": logs::redact_sensitive(&err.to_string())}),
        );
    let providers = {
        let map = state.providers.statuses.lock().await;
        map.values()
            .cloned()
            .map(|mut s| {
                s.diagnostics = s
                    .diagnostics
                    .into_iter()
                    .map(|d| logs::redact_sensitive(&d))
                    .collect();
                s.details = s
                    .details
                    .into_iter()
                    .filter(|(k, _)| !is_sensitive_key(k))
                    .map(|(k, v)| (k, logs::redact_sensitive(&v)))
                    .collect();
                s
            })
            .collect::<Vec<_>>()
    };

    let log_files = logs::list_log_files(&state.core.data_root).await;
    let managed_installs = installer::load_agent_server_config(&state.core.data_root)
        .await
        .map(|cfg| serde_json::to_value(cfg).unwrap_or_else(|_| serde_json::json!({})))
        .unwrap_or_else(|e| serde_json::json!({"error": logs::redact_sensitive(&e.to_string())}));
    let managed_installs = redact_json_value(managed_installs);

    let version = env!("CARGO_PKG_VERSION").to_string();
    let build_id = option_env!("CTX_BUILD_ID")
        .unwrap_or(env!("CARGO_PKG_VERSION"))
        .to_string();
    let dev_instance_id = option_env!("CTX_DEV_INSTANCE_ID")
        .unwrap_or("unknown")
        .to_string();
    Ok(Json(DiagnosticsResp {
        daemon: HealthResp {
            version: version.clone(),
            daemon_version: version.clone(),
            pid: std::process::id(),
            data_root: state.core.data_root.to_string_lossy().to_string(),
            daemon_url: state.core.daemon_url.clone(),
            auth_required: state.core.auth_token.is_some(),
            storage: state.storage_guard_snapshot(),
            compatibility: HealthCompatibility {
                desktop_exact_version: version,
                desktop_build_id: build_id,
                desktop_dev_instance_id: dev_instance_id,
                mobile_api_min: MOBILE_API_MIN_VERSION,
                mobile_api_max: MOBILE_API_MAX_VERSION,
            },
        },
        platform: serde_json::json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        }),
        logs: serde_json::json!({
            "dir": logs::logs_dir(&state.core.data_root).to_string_lossy(),
            "files": log_files,
        }),
        execution: serde_json::json!({
            "startup_prewarm": startup_prewarm,
            "linux_sandbox_runtime": linux_sandbox_runtime,
        }),
        providers,
        managed_installs,
    }))
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct ResourceUtilizationQuery {
    workspace_id: String,
}

pub(in crate::api) async fn resource_utilization(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ResourceUtilizationQuery>,
) -> Result<Json<resource_utilization::ResourceUtilizationSnapshot>, StatusCode> {
    if resource_utilization_disabled() {
        return Err(StatusCode::NOT_FOUND);
    }
    let workspace_id = WorkspaceId(
        uuid::Uuid::parse_str(&query.workspace_id).map_err(|_| StatusCode::BAD_REQUEST)?,
    );
    let workspace = state
        .global_store()
        .get_workspace(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let store = store_for_existing_workspace_status(&state, workspace_id).await?;
    let worktrees = store
        .list_worktrees(workspace_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let provider_adapters = {
        let providers = state.providers.adapters.lock().await;
        providers.values().cloned().collect::<Vec<_>>()
    };
    let mut provider_processes = Vec::new();
    for adapter in provider_adapters {
        provider_processes.extend(adapter.list_processes().await);
    }

    let (system, disks, cache_age_ms, processes, disk_cache) = {
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        let (system, disks, cache_age_ms) = sampler.system_snapshot();
        let processes = sampler.processes_snapshot_light(std::process::id(), &provider_processes);
        let disk_cache = sampler.disk_cache_entry(workspace_id);
        (system, disks, cache_age_ms, processes, disk_cache)
    };

    let disk = resource_utilization::disk_for_path(StdPath::new(&workspace.root_path), &disks);

    let now = Instant::now();
    let refresh_disk = resource_utilization::should_refresh_disk_cache(now, disk_cache.as_ref());
    let (mut workspace_snapshot, size_cache_age_ms) = if refresh_disk {
        let workspace_clone = workspace.clone();
        let worktrees_clone = worktrees.clone();
        let disk_clone = disk.clone();
        let snapshot = tokio::task::spawn_blocking(move || {
            resource_utilization::compute_workspace_disk_snapshot(
                workspace_clone,
                worktrees_clone,
                disk_clone,
                0,
            )
        })
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut sampler = state.telemetry.resource_sampler.lock().await;
        sampler.update_disk_cache(workspace_id, now, snapshot.clone());
        (snapshot, 0)
    } else {
        let age_ms = resource_utilization::disk_cache_age_ms(now, disk_cache.as_ref());
        let snapshot = disk_cache
            .as_ref()
            .map(|c| c.snapshot.clone())
            .unwrap_or_else(|| {
                resource_utilization::compute_workspace_disk_snapshot(
                    workspace.clone(),
                    worktrees.clone(),
                    disk.clone(),
                    age_ms,
                )
            });
        (snapshot, age_ms)
    };

    workspace_snapshot.disk = disk;
    workspace_snapshot.size_cache_age_ms = size_cache_age_ms;

    Ok(Json(resource_utilization::ResourceUtilizationSnapshot {
        collected_at: chrono::Utc::now().to_rfc3339(),
        cache_age_ms,
        system,
        processes,
        workspace: workspace_snapshot,
    }))
}

pub(in crate::api) fn resource_utilization_disabled() -> bool {
    env_bool("CTX_RESOURCE_UTILIZATION_DISABLED").unwrap_or(true)
}

pub(in crate::api) fn env_bool(key: &str) -> Option<bool> {
    std::env::var(key)
        .ok()
        .as_deref()
        .and_then(ctx_core::boolish::parse_boolish)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct LspFileReq {
    /// Optional session scope; when present, `path` is resolved within the session worktree.
    pub(in crate::api) session_id: Option<String>,
    /// Optional explicit root path; used only when `session_id` is absent.
    pub(in crate::api) root_path: Option<String>,
    /// File path to analyze (absolute or relative to resolved root).
    pub(in crate::api) path: String,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct LspServerStatus {
    pub(in crate::api) language: String,
    pub(in crate::api) command: String,
    pub(in crate::api) args: Vec<String>,
    pub(in crate::api) found: bool,
    pub(in crate::api) resolved_path: Option<String>,
    pub(in crate::api) version: Option<String>,
    pub(in crate::api) install_hints: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(in crate::api) struct LspStatusResp {
    pub(in crate::api) enabled: bool,
    pub(in crate::api) edit_plans_enabled: bool,
    pub(in crate::api) servers: Vec<LspServerStatus>,
}

pub(in crate::api) async fn open_logs_folder(
    State(state): State<Arc<AppState>>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    logs::open_logs_folder(&state.core.data_root)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct DesktopLogReq {
    level: Option<String>,
    message: String,
}

pub(in crate::api) async fn append_desktop_log(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DesktopLogReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiErrorResp>)> {
    let level = req.level.unwrap_or_else(|| "info".to_string());
    let line = format!(
        "{} [{level}] {}",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        req.message
    );
    logs::append_desktop_log_line(&state.core.data_root, &line)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

pub(in crate::api) async fn perf_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let start = Instant::now();
    let method = req.method().to_string();
    let run_id = req
        .headers()
        .get("x-ctx-run-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let endpoint = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let parent = state
        .telemetry
        .perf_telemetry
        .extract_trace_context(req.headers());
    let span = state.telemetry.perf_telemetry.start_span(
        "http_request",
        SpanKind::Server,
        Some(parent),
        vec![
            KeyValue::new("http.method", method.clone()),
            KeyValue::new("http.route", endpoint.clone()),
        ],
    );
    let response = next.run(req).await;
    let status = response.status().as_u16();
    let duration_ms = start.elapsed().as_millis() as u64;
    let success = status < 500;
    let mut labels = HashMap::new();
    labels.insert("endpoint".to_string(), endpoint);
    labels.insert("method".to_string(), method);
    labels.insert("status".to_string(), status.to_string());
    labels.insert("success".to_string(), success.to_string());
    labels.insert("source".to_string(), "daemon".to_string());
    let (trace_id, span_id) = state.telemetry.perf_telemetry.finish_span(
        span,
        Some(status.to_string()),
        Some(success),
        vec![KeyValue::new("http.status_code", status as i64)],
    );
    let metric = PerfMetric {
        name: "http.request.duration_ms".to_string(),
        kind: PerfMetricKind::Histogram,
        unit: "ms".to_string(),
        value: duration_ms as f64,
        labels,
    };
    state
        .telemetry
        .perf_telemetry
        .record_metric(metric, run_id, trace_id, span_id)
        .await;
    response
}
