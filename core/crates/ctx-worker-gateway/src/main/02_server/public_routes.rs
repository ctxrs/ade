use super::*;

pub(super) async fn get_worker_shim(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, StatusCode> {
    let file = tokio::fs::File::open(&state.worker_shim_path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);
    Ok((
        [
            ("content-type", "application/octet-stream"),
            ("cache-control", "no-store"),
        ],
        body,
    ))
}

pub(super) async fn get_worker_bootstrap(
    State(state): State<AppState>,
    Path(worker_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let spec = get_request_spec(&state, &worker_id).await?;
    let base_commit = get_base_commit(&state, &worker_id).await?;
    let mut candidates = vec![
        "/dev/xvdf".to_string(),
        "/dev/sdf".to_string(),
        "/dev/nvme1n1".to_string(),
    ];
    candidates.retain(|v| !v.is_empty());
    let shim_url = format!("{}/shim", state.public_base_url.trim_end_matches('/'));
    let bootstrap = bootstrap::BootstrapSpec {
        worker_id: &worker_id,
        gateway_url: state.public_base_url.as_str(),
        gateway_token: spec.env.get("CTX_WORKER_GATEWAY_TOKEN").map(|v| v.as_str()),
        base_commit: &base_commit,
        diff_debounce_ms: spec.diff_debounce_ms.unwrap_or(1500),
        repo: &spec.repo,
        provider_id: spec.provider_id.as_deref(),
        env: &spec.env,
        shim_url: &shim_url,
        workdir: &state.workdir_path,
        mount_path: &state.session_mount_path,
        mount_device_candidates: candidates,
    };
    let script = render_bootstrap_script(&bootstrap);
    Ok((
        [
            ("content-type", "text/x-shellscript"),
            ("cache-control", "no-store"),
        ],
        script,
    ))
}

pub(super) async fn health() -> impl IntoResponse {
    Json(json!({"ok": true}))
}
