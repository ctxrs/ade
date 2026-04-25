#[path = "02_server/auth.rs"]
mod auth;
#[path = "02_server/public_routes.rs"]
mod public_routes;

use auth::auth_middleware;
use public_routes::{get_worker_bootstrap, get_worker_shim, health};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    if let Err(err) = rustls::crypto::aws_lc_rs::default_provider().install_default() {
        warn!("failed to install rustls crypto provider: {err:?}");
    }

    let args = Args::parse();
    let config_path = args.config.clone().or_else(default_config_path);
    let config = if let Some(path) = config_path.as_ref() {
        if path.exists() {
            Some(load_gateway_config(path)?)
        } else if args.config.is_some() {
            anyhow::bail!("gateway config file not found: {}", path.display());
        } else {
            None
        }
    } else {
        None
    };
    let resolved = resolve_args(args, config);
    let public_base_url = resolved.public_base_url.trim_end_matches('/').to_string();
    let auth_token = resolved.auth_token.clone();
    let worker_shim_url =
        resolved
            .worker_shim_url
            .clone()
            .unwrap_or_else(|| match auth_token.as_deref() {
                Some(token) => format!("{public_base_url}/shim?token={token}"),
                None => format!("{public_base_url}/shim"),
            });

    let driver: Arc<dyn WorkerDriver> = match resolved.driver.as_str() {
        "local" => Arc::new(LocalDriver {
            shim_path: resolved.worker_shim_path.clone(),
            processes: RwLock::new(HashMap::new()),
        }),
        "aws" => {
            let config = AwsConfig {
                region: resolved
                    .aws_region
                    .clone()
                    .context("missing --aws-region")?,
                ami_id: resolved
                    .aws_ami_id
                    .clone()
                    .context("missing --aws-ami-id")?,
                instance_type: resolved
                    .aws_instance_type
                    .clone()
                    .context("missing --aws-instance-type")?,
                subnet_id: resolved
                    .aws_subnet_id
                    .clone()
                    .context("missing --aws-subnet-id")?,
                security_group_ids: resolved.aws_security_group_ids.clone(),
                key_name: resolved.aws_key_name.clone(),
                instance_profile: resolved.aws_instance_profile.clone(),
                ssh_user: resolved.aws_ssh_user.clone(),
                volume_size_gb: resolved.aws_volume_size_gb,
                volume_type: resolved.aws_volume_type.clone(),
                data_device_name: resolved.aws_data_device_name.clone(),
                delete_volume_on_pause: resolved.aws_delete_volume_on_pause,
                wait_for_snapshot: resolved.aws_wait_for_snapshot,
                availability_zone: resolved.aws_availability_zone.clone(),
            };
            Arc::new(AwsDriver::new(config, auth_token.clone()).await?)
        }
        "gcp" => {
            let config = GcpConfig {
                project_id: resolved
                    .gcp_project_id
                    .clone()
                    .context("missing --gcp-project-id")?,
                zone: resolved.gcp_zone.clone().context("missing --gcp-zone")?,
                machine_type: resolved
                    .gcp_machine_type
                    .clone()
                    .context("missing --gcp-machine-type")?,
                image: resolved.gcp_image.clone().context("missing --gcp-image")?,
                network: resolved.gcp_network.clone(),
                subnetwork: resolved.gcp_subnetwork.clone(),
                service_account: resolved.gcp_service_account.clone(),
                scopes: resolved.gcp_scopes.clone(),
                disk_size_gb: resolved.gcp_disk_size_gb,
                disk_type: resolved.gcp_disk_type.clone(),
                ssh_user: resolved.gcp_ssh_user.clone(),
                delete_disk_on_pause: resolved.gcp_delete_disk_on_pause,
                worker_shim_url,
                mount_path: resolved.session_mount_path.clone(),
                workdir: resolved.workdir_path.clone(),
            };
            Arc::new(GcpDriver::new(config, auth_token.clone()).await?)
        }
        "azure" => {
            let ssh_key = if let Some(path) = resolved.azure_ssh_public_key_path.clone() {
                std::fs::read_to_string(path).context("reading azure ssh public key")?
            } else {
                resolved
                    .azure_ssh_public_key
                    .clone()
                    .context("missing --azure-ssh-public-key")?
            };
            let config = AzureConfig {
                subscription_id: resolved
                    .azure_subscription_id
                    .clone()
                    .context("missing --azure-subscription-id")?,
                resource_group: resolved
                    .azure_resource_group
                    .clone()
                    .context("missing --azure-resource-group")?,
                location: resolved
                    .azure_location
                    .clone()
                    .context("missing --azure-location")?,
                vm_size: resolved
                    .azure_vm_size
                    .clone()
                    .context("missing --azure-vm-size")?,
                image: resolved
                    .azure_image
                    .clone()
                    .context("missing --azure-image")?,
                vnet: resolved
                    .azure_vnet
                    .clone()
                    .context("missing --azure-vnet")?,
                subnet: resolved
                    .azure_subnet
                    .clone()
                    .context("missing --azure-subnet")?,
                admin_username: resolved
                    .azure_admin_username
                    .clone()
                    .context("missing --azure-admin-username")?,
                ssh_public_key: ssh_key,
                disk_size_gb: resolved.azure_disk_size_gb,
                disk_sku: resolved.azure_disk_sku.clone(),
                delete_disk_on_pause: resolved.azure_delete_disk_on_pause,
                use_public_ip: resolved.azure_use_public_ip,
                worker_shim_url,
                mount_path: resolved.session_mount_path.clone(),
                workdir: resolved.workdir_path.clone(),
            };
            Arc::new(AzureDriver::new(config, auth_token.clone()).await?)
        }
        other => anyhow::bail!("unknown driver: {other}"),
    };

    let state = AppState {
        store: Arc::new(WorkerStore {
            workers: RwLock::new(HashMap::new()),
        }),
        driver,
        public_base_url,
        worker_shim_path: resolved.worker_shim_path.clone(),
        session_mount_path: resolved.session_mount_path.clone(),
        workdir_path: resolved.workdir_path.clone(),
        auth_token,
        terminal_relays: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/workers", post(start_worker))
        .route("/workers/:id", get(get_worker))
        .route("/workers/:id/pause", post(pause_worker))
        .route("/workers/:id/resume", post(resume_worker))
        .route("/workers/:id/stop", post(stop_worker))
        .route("/workers/:id/diff", get(get_diff).post(post_diff))
        .route("/workers/:id/export", post(export_patch))
        .route("/workers/:id/register", post(register_worker))
        .route("/workers/:id/terminals", post(open_terminal))
        .route(
            "/workers/:id/terminals/:terminal_id/close",
            post(close_terminal),
        )
        .route(
            "/workers/:id/terminals/control/worker",
            get(terminal_control_ws),
        )
        .route(
            "/workers/:id/terminals/:terminal_id/daemon",
            get(terminal_daemon_ws),
        )
        .route(
            "/workers/:id/terminals/:terminal_id/worker",
            get(terminal_worker_ws),
        )
        .route("/workers/:id/bootstrap", get(get_worker_bootstrap))
        .route("/shim", get(get_worker_shim))
        .route("/health", get(health))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    let addr: SocketAddr = resolved.bind.parse().context("parsing bind addr")?;
    info!("ctx-worker-gateway listening on {addr}");
    match (
        resolved.tls_cert_path.as_deref(),
        resolved.tls_key_path.as_deref(),
    ) {
        (Some(cert_path), Some(key_path)) => {
            let tls_config = RustlsConfig::from_pem_file(cert_path, key_path)
                .await
                .context("loading tls cert/key")?;
            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service())
                .await?;
        }
        (None, None) => {
            axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
        }
        _ => {
            anyhow::bail!("both tls_cert_path and tls_key_path must be set to enable TLS");
        }
    }
    Ok(())
}

async fn start_worker(
    State(state): State<AppState>,
    Json(request): Json<StartWorkerRequest>,
) -> Result<Json<StartWorkerResponse>, (StatusCode, Json<ErrorResponse>)> {
    let worker_id = new_worker_id();
    let now = Utc::now();

    let base_commit = request
        .base_commit_sha
        .clone()
        .unwrap_or_else(|| "HEAD".to_string());
    let mut spec = request.clone();
    spec.base_commit_sha = Some(base_commit.clone());

    let driver_spec = spec.clone();
    let record = WorkerRecord {
        worker_id: worker_id.clone(),
        spec,
        task_id: request.task_id.clone(),
        provider_id: request.provider_id.clone(),
        model_id: request.model_id.clone(),
        base_commit_sha: base_commit.clone(),
        state: WorkerState::Starting,
        created_at: now,
        updated_at: now,
        last_diff_at: None,
        last_diff: None,
        ssh: None,
    };

    {
        let mut workers = state.store.workers.write().await;
        workers.insert(worker_id.clone(), record);
    }

    let ssh = match state
        .driver
        .start(
            &worker_id,
            &driver_spec,
            &base_commit,
            &state.public_base_url,
        )
        .await
    {
        Ok(info) => info,
        Err(err) => {
            let _ = state.driver.stop(&worker_id).await;
            update_state(&state, &worker_id, WorkerState::Failed).await;
            error!("failed to start worker: {err:#}");
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new("failed_to_start")),
            ));
        }
    };

    update_state_with_ssh(&state, &worker_id, WorkerState::Running, ssh.clone()).await;

    Ok(Json(StartWorkerResponse {
        worker_id,
        state: WorkerState::Running,
        ssh,
    }))
}

async fn get_worker(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WorkerInfo>, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(&id).ok_or_else(not_found)?;

    Ok(Json(WorkerInfo {
        worker_id: record.worker_id.clone(),
        task_id: record.task_id.clone(),
        provider_id: record.provider_id.clone(),
        model_id: record.model_id.clone(),
        state: record.state.clone(),
        base_commit_sha: Some(record.base_commit_sha.clone()),
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_diff_at: record.last_diff_at,
        ssh: record.ssh.clone(),
    }))
}

async fn pause_worker(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    if let Err(err) = state.driver.pause(&id).await {
        error!("failed to pause worker {id}: {err:#}");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("failed_to_pause")),
        ));
    }
    update_state(&state, &id, WorkerState::Paused).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_worker(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let spec = get_request_spec(&state, &id).await?;
    let base_commit = get_base_commit(&state, &id).await?;

    if let Err(err) = state
        .driver
        .resume(&id, &spec, &base_commit, &state.public_base_url)
        .await
    {
        error!("failed to resume worker {id}: {err:#}");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("failed_to_resume")),
        ));
    }

    update_state(&state, &id, WorkerState::Running).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn stop_worker(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    if let Err(err) = state.driver.stop(&id).await {
        error!("failed to stop worker {id}: {err:#}");
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("failed_to_stop")),
        ));
    }
    update_state(&state, &id, WorkerState::Stopped).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_diff(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DiffArtifact>, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(&id).ok_or_else(not_found)?;
    let diff = record.last_diff.clone().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("diff_unavailable")),
        )
    })?;
    Ok(Json(diff))
}

async fn post_diff(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(diff): Json<DiffArtifact>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let mut workers = state.store.workers.write().await;
    let record = workers.get_mut(&id).ok_or_else(not_found)?;
    record.last_diff_at = Some(Utc::now());
    record.last_diff = Some(diff);
    record.updated_at = Utc::now();
    Ok(StatusCode::NO_CONTENT)
}

async fn export_patch(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ExportPatchResponse>, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(&id).ok_or_else(not_found)?;
    let diff = record.last_diff.clone().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("diff_unavailable")),
        )
    })?;

    Ok(Json(ExportPatchResponse {
        worker_id: diff.worker_id,
        base_commit_sha: diff.base_commit_sha,
        head_commit_sha: diff.head_commit_sha,
        generated_at: diff.generated_at,
        patch: diff.patch,
        changed_files: diff.changed_files,
        file_count: diff.file_count,
        line_additions: diff.line_additions,
        line_deletions: diff.line_deletions,
    }))
}

async fn register_worker(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(reg): Json<WorkerRegistration>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let mut workers = state.store.workers.write().await;
    let record = workers.get_mut(&id).ok_or_else(not_found)?;
    if reg.ssh.is_some() {
        record.ssh = reg.ssh;
    }
    record.updated_at = Utc::now();
    Ok(StatusCode::NO_CONTENT)
}

async fn open_terminal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TerminalOpenRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    debug!(
        worker_id = %id,
        terminal_id = %req.terminal_id,
        "terminal open requested"
    );
    let relay = terminal_relay_for(&state, &id).await;
    let msg = TerminalControlMessage::Open {
        terminal_id: req.terminal_id.clone(),
        shell: req.shell.clone(),
        cwd: req.cwd.clone(),
        cols: req.cols,
        rows: req.rows,
    };
    let mut relay_guard = relay.lock().await;
    relay_guard
        .sessions
        .entry(req.terminal_id.clone())
        .or_insert_with(TerminalSessionRelay::default);
    if let Some(control_tx) = relay_guard.control_tx.as_ref() {
        let _ = control_tx.send(msg);
    } else {
        relay_guard.pending.push_back(msg);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn close_terminal(
    State(state): State<AppState>,
    Path((id, terminal_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    let relay = terminal_relay_for(&state, &id).await;
    let msg = TerminalControlMessage::Close {
        terminal_id: terminal_id.clone(),
    };
    let mut relay_guard = relay.lock().await;
    if let Some(control_tx) = relay_guard.control_tx.as_ref() {
        let _ = control_tx.send(msg);
    } else {
        relay_guard.pending.push_back(msg);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn terminal_control_ws(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    Ok(ws.on_upgrade(move |socket| handle_terminal_control_socket(state, id, socket)))
}

async fn terminal_daemon_ws(
    State(state): State<AppState>,
    Path((id, terminal_id)): Path<(String, String)>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    Ok(ws.on_upgrade(move |socket| {
        handle_terminal_data_socket(state, id, terminal_id, TerminalSide::Daemon, socket)
    }))
}

async fn terminal_worker_ws(
    State(state): State<AppState>,
    Path((id, terminal_id)): Path<(String, String)>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    Ok(ws.on_upgrade(move |socket| {
        handle_terminal_data_socket(state, id, terminal_id, TerminalSide::Worker, socket)
    }))
}

async fn handle_terminal_control_socket(state: AppState, worker_id: String, socket: WebSocket) {
    let relay = terminal_relay_for(&state, &worker_id).await;
    debug!(worker_id = %worker_id, "terminal control connected");
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<TerminalControlMessage>();

    {
        let mut relay_guard = relay.lock().await;
        relay_guard.control_tx = Some(tx);
        while let Some(pending) = relay_guard.pending.pop_front() {
            if let Some(control_tx) = relay_guard.control_tx.as_ref() {
                let _ = control_tx.send(pending);
            }
        }
    }

    let relay_for_sender = relay.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(text) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
        }
        let mut relay_guard = relay_for_sender.lock().await;
        relay_guard.control_tx = None;
    });

    while let Some(Ok(_msg)) = receiver.next().await {}

    let _ = send_task.await;
    let mut relay_guard = relay.lock().await;
    relay_guard.control_tx = None;
}
