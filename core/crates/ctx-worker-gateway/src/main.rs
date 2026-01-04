use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::extract::ws::{Message, WebSocket};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use chrono::{DateTime, Utc};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{error, info};

use ctx_worker_protocol::{
    new_worker_id, DiffArtifact, ExportPatchResponse, RelayMessage, StartWorkerRequest,
    StartWorkerResponse, TerminalControlMessage, TerminalOpenRequest, WorkerInfo, WorkerRegistration,
    WorkerState,
};
use tokio_util::io::ReaderStream;

mod bootstrap;
mod drivers;

use drivers::aws::{AwsConfig, AwsDriver};
use drivers::azure::{AzureConfig, AzureDriver};
use drivers::gcp::{GcpConfig, GcpDriver};
use drivers::local::LocalDriver;
use drivers::WorkerDriver;
use serde_json::json;

#[derive(Parser, Debug)]
#[command(name = "ctx-worker-gateway")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8787")]
    bind: String,
    #[arg(long, default_value = "local")]
    driver: String,
    #[arg(long, default_value = "ctx-worker-shim")]
    worker_shim_path: String,
    #[arg(long)]
    worker_shim_url: Option<String>,
    #[arg(long)]
    auth_token: Option<String>,
    #[arg(long, default_value = "http://127.0.0.1:8787")]
    public_base_url: String,
    #[arg(long, default_value = "/mnt/ctx-session")]
    session_mount_path: String,
    #[arg(long, default_value = "/mnt/ctx-session/workdir")]
    workdir_path: String,

    #[arg(long)]
    aws_region: Option<String>,
    #[arg(long)]
    aws_ami_id: Option<String>,
    #[arg(long)]
    aws_instance_type: Option<String>,
    #[arg(long)]
    aws_subnet_id: Option<String>,
    #[arg(long, value_delimiter = ',')]
    aws_security_group_ids: Vec<String>,
    #[arg(long)]
    aws_key_name: Option<String>,
    #[arg(long)]
    aws_instance_profile: Option<String>,
    #[arg(long)]
    aws_ssh_user: Option<String>,
    #[arg(long, default_value_t = 100)]
    aws_volume_size_gb: i32,
    #[arg(long, default_value = "gp3")]
    aws_volume_type: String,
    #[arg(long, default_value = "/dev/sdf")]
    aws_data_device_name: String,
    #[arg(long)]
    aws_delete_volume_on_pause: bool,
    #[arg(long)]
    aws_wait_for_snapshot: bool,
    #[arg(long)]
    aws_availability_zone: Option<String>,

    #[arg(long)]
    gcp_project_id: Option<String>,
    #[arg(long)]
    gcp_zone: Option<String>,
    #[arg(long)]
    gcp_machine_type: Option<String>,
    #[arg(long)]
    gcp_image: Option<String>,
    #[arg(long)]
    gcp_network: Option<String>,
    #[arg(long)]
    gcp_subnetwork: Option<String>,
    #[arg(long)]
    gcp_service_account: Option<String>,
    #[arg(long, value_delimiter = ',')]
    gcp_scopes: Vec<String>,
    #[arg(long, default_value_t = 100)]
    gcp_disk_size_gb: i64,
    #[arg(long, default_value = "pd-ssd")]
    gcp_disk_type: String,
    #[arg(long)]
    gcp_ssh_user: Option<String>,
    #[arg(long)]
    gcp_delete_disk_on_pause: bool,

    #[arg(long)]
    azure_subscription_id: Option<String>,
    #[arg(long)]
    azure_resource_group: Option<String>,
    #[arg(long)]
    azure_location: Option<String>,
    #[arg(long)]
    azure_vm_size: Option<String>,
    #[arg(long)]
    azure_image: Option<String>,
    #[arg(long)]
    azure_vnet: Option<String>,
    #[arg(long)]
    azure_subnet: Option<String>,
    #[arg(long)]
    azure_admin_username: Option<String>,
    #[arg(long)]
    azure_ssh_public_key: Option<String>,
    #[arg(long)]
    azure_ssh_public_key_path: Option<String>,
    #[arg(long, default_value_t = 100)]
    azure_disk_size_gb: i32,
    #[arg(long, default_value = "Premium_LRS")]
    azure_disk_sku: String,
    #[arg(long)]
    azure_delete_disk_on_pause: bool,
    #[arg(long)]
    azure_use_public_ip: bool,
}

#[derive(Clone)]
struct AppState {
    store: Arc<WorkerStore>,
    driver: Arc<dyn WorkerDriver>,
    public_base_url: String,
    worker_shim_path: String,
    auth_token: Option<String>,
    relays: Arc<RwLock<HashMap<String, Arc<Mutex<RelayState>>>>>,
    terminal_relays: Arc<RwLock<HashMap<String, Arc<Mutex<TerminalRelayState>>>>>,
}

struct WorkerStore {
    workers: RwLock<HashMap<String, WorkerRecord>>,
}

struct RelayState {
    worker_tx: Option<mpsc::UnboundedSender<RelayMessage>>,
    sessions: HashMap<String, mpsc::UnboundedSender<RelayMessage>>,
}

impl RelayState {
    fn new() -> Self {
        Self {
            worker_tx: None,
            sessions: HashMap::new(),
        }
    }
}

#[derive(Default)]
struct TerminalRelayState {
    control_tx: Option<mpsc::UnboundedSender<TerminalControlMessage>>,
    pending: VecDeque<TerminalControlMessage>,
    sessions: HashMap<String, TerminalSessionRelay>,
}

#[derive(Default)]
struct TerminalSessionRelay {
    daemon_tx: Option<mpsc::UnboundedSender<Message>>,
    worker_tx: Option<mpsc::UnboundedSender<Message>>,
}

#[derive(Clone)]
struct WorkerRecord {
    worker_id: String,
    spec: StartWorkerRequest,
    task_id: String,
    track_id: String,
    provider_id: Option<String>,
    model_id: Option<String>,
    base_commit_sha: String,
    state: WorkerState,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_diff_at: Option<DateTime<Utc>>,
    last_diff: Option<DiffArtifact>,
    ssh: Option<ctx_worker_protocol::SshInfo>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let public_base_url = args.public_base_url.trim_end_matches('/').to_string();
    let auth_token = args.auth_token.clone();
    let worker_shim_url = args
        .worker_shim_url
        .clone()
        .unwrap_or_else(|| match auth_token.as_deref() {
            Some(token) => format!("{public_base_url}/shim?token={token}"),
            None => format!("{public_base_url}/shim"),
        });

    let driver: Arc<dyn WorkerDriver> = match args.driver.as_str() {
        "local" => Arc::new(LocalDriver {
            shim_path: args.worker_shim_path.clone(),
            processes: RwLock::new(HashMap::new()),
        }),
        "aws" => {
            let config = AwsConfig {
                region: args.aws_region.clone().context("missing --aws-region")?,
                ami_id: args.aws_ami_id.clone().context("missing --aws-ami-id")?,
                instance_type: args
                    .aws_instance_type
                    .clone()
                    .context("missing --aws-instance-type")?,
                subnet_id: args
                    .aws_subnet_id
                    .clone()
                    .context("missing --aws-subnet-id")?,
                security_group_ids: args.aws_security_group_ids.clone(),
                key_name: args.aws_key_name.clone(),
                instance_profile: args.aws_instance_profile.clone(),
                ssh_user: args.aws_ssh_user.clone(),
                volume_size_gb: args.aws_volume_size_gb,
                volume_type: args.aws_volume_type.clone(),
                data_device_name: args.aws_data_device_name.clone(),
                delete_volume_on_pause: args.aws_delete_volume_on_pause,
                wait_for_snapshot: args.aws_wait_for_snapshot,
                worker_shim_url,
                mount_path: args.session_mount_path.clone(),
                workdir: args.workdir_path.clone(),
                availability_zone: args.aws_availability_zone.clone(),
            };
            Arc::new(AwsDriver::new(config, auth_token.clone()).await?)
        }
        "gcp" => {
            let config = GcpConfig {
                project_id: args
                    .gcp_project_id
                    .clone()
                    .context("missing --gcp-project-id")?,
                zone: args.gcp_zone.clone().context("missing --gcp-zone")?,
                machine_type: args
                    .gcp_machine_type
                    .clone()
                    .context("missing --gcp-machine-type")?,
                image: args.gcp_image.clone().context("missing --gcp-image")?,
                network: args.gcp_network.clone(),
                subnetwork: args.gcp_subnetwork.clone(),
                service_account: args.gcp_service_account.clone(),
                scopes: args.gcp_scopes.clone(),
                disk_size_gb: args.gcp_disk_size_gb,
                disk_type: args.gcp_disk_type.clone(),
                ssh_user: args.gcp_ssh_user.clone(),
                delete_disk_on_pause: args.gcp_delete_disk_on_pause,
                worker_shim_url,
                mount_path: args.session_mount_path.clone(),
                workdir: args.workdir_path.clone(),
            };
            Arc::new(GcpDriver::new(config, auth_token.clone()).await?)
        }
        "azure" => {
            let ssh_key = if let Some(path) = args.azure_ssh_public_key_path.clone() {
                std::fs::read_to_string(path).context("reading azure ssh public key")?
            } else {
                args.azure_ssh_public_key
                    .clone()
                    .context("missing --azure-ssh-public-key")?
            };
            let config = AzureConfig {
                subscription_id: args
                    .azure_subscription_id
                    .clone()
                    .context("missing --azure-subscription-id")?,
                resource_group: args
                    .azure_resource_group
                    .clone()
                    .context("missing --azure-resource-group")?,
                location: args
                    .azure_location
                    .clone()
                    .context("missing --azure-location")?,
                vm_size: args
                    .azure_vm_size
                    .clone()
                    .context("missing --azure-vm-size")?,
                image: args
                    .azure_image
                    .clone()
                    .context("missing --azure-image")?,
                vnet: args.azure_vnet.clone().context("missing --azure-vnet")?,
                subnet: args
                    .azure_subnet
                    .clone()
                    .context("missing --azure-subnet")?,
                admin_username: args
                    .azure_admin_username
                    .clone()
                    .context("missing --azure-admin-username")?,
                ssh_public_key: ssh_key,
                disk_size_gb: args.azure_disk_size_gb,
                disk_sku: args.azure_disk_sku.clone(),
                delete_disk_on_pause: args.azure_delete_disk_on_pause,
                use_public_ip: args.azure_use_public_ip,
                worker_shim_url,
                mount_path: args.session_mount_path.clone(),
                workdir: args.workdir_path.clone(),
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
        worker_shim_path: args.worker_shim_path.clone(),
        auth_token,
        relays: Arc::new(RwLock::new(HashMap::new())),
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
        .route("/workers/:id/acp/worker", get(acp_worker_ws))
        .route("/workers/:id/acp/daemon", get(acp_daemon_ws))
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
        .route("/shim", get(get_worker_shim))
        .route("/health", get(health))
        .layer(middleware::from_fn_with_state(state.clone(), auth_middleware))
        .with_state(state);

    let addr: SocketAddr = args.bind.parse().context("parsing bind addr")?;
    info!("ctx-worker-gateway listening on {addr}");
    axum::serve(tokio::net::TcpListener::bind(addr).await?, app).await?;
    Ok(())
}

async fn get_worker_shim(State(state): State<AppState>) -> Result<impl IntoResponse, StatusCode> {
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

async fn health() -> impl IntoResponse {
    Json(json!({"ok": true}))
}

async fn auth_middleware(
    State(state): State<AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let Some(expected) = state.auth_token.as_deref() else {
        return next.run(req).await;
    };

    let path = req.uri().path();
    let method = req.method().as_str();

    // Public endpoints.
    if path == "/health" {
        return next.run(req).await;
    }
    if method == "GET"
        && (path.starts_with("/workers/")
            && !path.contains("/acp/")
            && !path.contains("/terminals/")
            && !path.ends_with("/export"))
    {
        // Allow read-only inspection without auth by default.
        return next.run(req).await;
    }

    let header_token = req
        .headers()
        .get("x-ctx-gateway-token")
        .and_then(|v| v.to_str().ok());
    let mut token_ok = header_token == Some(expected);

    if !token_ok && path == "/shim" {
        if let Some(query) = req.uri().query() {
            for part in query.split('&') {
                let Some((key, value)) = part.split_once('=') else {
                    continue;
                };
                if key == "token" && value == expected {
                    token_ok = true;
                    break;
                }
            }
        }
    }

    if token_ok {
        return next.run(req).await;
    }

    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
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
        track_id: request.track_id.clone(),
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
        .start(&worker_id, &driver_spec, &base_commit, &state.public_base_url)
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
        track_id: record.track_id.clone(),
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
    record.ssh = reg.ssh;
    record.updated_at = Utc::now();
    Ok(StatusCode::NO_CONTENT)
}

async fn acp_worker_ws(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    Ok(ws.on_upgrade(move |socket| handle_worker_socket(state, id, socket)))
}

async fn acp_daemon_ws(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl axum::response::IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
    Ok(ws.on_upgrade(move |socket| handle_daemon_socket(state, id, socket)))
}

async fn handle_worker_socket(state: AppState, worker_id: String, socket: WebSocket) {
    let relay = relay_for(&state, &worker_id).await;
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<RelayMessage>();

    {
        let mut relay_guard = relay.lock().await;
        relay_guard.worker_tx = Some(tx);
    }

    let relay_for_sender = relay.clone();
    let worker_id_for_sender = worker_id.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(text) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
        }
        let mut relay_guard = relay_for_sender.lock().await;
        if relay_guard.worker_tx.is_some() {
            relay_guard.worker_tx = None;
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            let Ok(relay_msg) = serde_json::from_str::<RelayMessage>(&text) else {
                continue;
            };
            let session_id = match &relay_msg {
                RelayMessage::Acp { session_id, .. }
                | RelayMessage::Close { session_id }
                | RelayMessage::Init { session_id, .. } => session_id,
            };
            let relay_guard = relay.lock().await;
            if let Some(session_tx) = relay_guard.sessions.get(session_id) {
                let _ = session_tx.send(relay_msg);
            } else {
                info!(
                    "worker relay message dropped; no daemon session for worker={} session={}",
                    worker_id_for_sender, session_id
                );
            }
        }
    }

    let _ = send_task.await;
}

async fn handle_daemon_socket(state: AppState, worker_id: String, socket: WebSocket) {
    let relay = relay_for(&state, &worker_id).await;
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<RelayMessage>();

    let mut session_id: Option<String> = None;
    if let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(relay_msg) = serde_json::from_str::<RelayMessage>(&text) {
                if let RelayMessage::Init { session_id: sid, .. } = &relay_msg {
                    session_id = Some(sid.clone());
                    let mut relay_guard = relay.lock().await;
                    relay_guard.sessions.insert(sid.clone(), tx.clone());
                    if let Some(worker_tx) = relay_guard.worker_tx.as_ref() {
                        let _ = worker_tx.send(relay_msg);
                    }
                }
            }
        }
    }

    let Some(session_id) = session_id else {
        return;
    };

    let relay_for_sender = relay.clone();
    let session_id_for_task = session_id.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(text) = serde_json::to_string(&msg) {
                if sender.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
        }
        let mut relay_guard = relay_for_sender.lock().await;
        relay_guard.sessions.remove(&session_id_for_task);
    });

    while let Some(Ok(msg)) = receiver.next().await {
        if let Message::Text(text) = msg {
            if let Ok(relay_msg) = serde_json::from_str::<RelayMessage>(&text) {
                let relay_guard = relay.lock().await;
                if let Some(worker_tx) = relay_guard.worker_tx.as_ref() {
                    let _ = worker_tx.send(relay_msg);
                }
            }
        }
    }

    let _ = send_task.await;
    let mut relay_guard = relay.lock().await;
    relay_guard.sessions.remove(&session_id);
}

async fn open_terminal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TerminalOpenRequest>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ensure_exists(&state, &id).await?;
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

async fn handle_terminal_control_socket(
    state: AppState,
    worker_id: String,
    socket: WebSocket,
) {
    let relay = terminal_relay_for(&state, &worker_id).await;
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

#[derive(Clone, Copy)]
enum TerminalSide {
    Daemon,
    Worker,
}

async fn handle_terminal_data_socket(
    state: AppState,
    worker_id: String,
    terminal_id: String,
    side: TerminalSide,
    socket: WebSocket,
) {
    let relay = terminal_relay_for(&state, &worker_id).await;
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    {
        let mut relay_guard = relay.lock().await;
        let entry = relay_guard
            .sessions
            .entry(terminal_id.clone())
            .or_insert_with(TerminalSessionRelay::default);
        match side {
            TerminalSide::Daemon => entry.daemon_tx = Some(tx),
            TerminalSide::Worker => entry.worker_tx = Some(tx),
        }
    }

    let relay_for_sender = relay.clone();
    let terminal_id_for_sender = terminal_id.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
        let mut relay_guard = relay_for_sender.lock().await;
        if let Some(entry) = relay_guard.sessions.get_mut(&terminal_id_for_sender) {
            match side {
                TerminalSide::Daemon => entry.daemon_tx = None,
                TerminalSide::Worker => entry.worker_tx = None,
            }
            if entry.daemon_tx.is_none() && entry.worker_tx.is_none() {
                relay_guard.sessions.remove(&terminal_id_for_sender);
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        let other_tx = {
            let relay_guard = relay.lock().await;
            relay_guard.sessions.get(&terminal_id).and_then(|entry| match side {
                TerminalSide::Daemon => entry.worker_tx.clone(),
                TerminalSide::Worker => entry.daemon_tx.clone(),
            })
        };
        if let Some(tx) = other_tx {
            match msg {
                Message::Text(_) | Message::Binary(_) | Message::Close(_) => {
                    let _ = tx.send(msg);
                }
                Message::Ping(_) | Message::Pong(_) => {}
            }
        }
    }

    let _ = send_task.await;
    let mut relay_guard = relay.lock().await;
    if let Some(entry) = relay_guard.sessions.get_mut(&terminal_id) {
        match side {
            TerminalSide::Daemon => entry.daemon_tx = None,
            TerminalSide::Worker => entry.worker_tx = None,
        }
        if entry.daemon_tx.is_none() && entry.worker_tx.is_none() {
            relay_guard.sessions.remove(&terminal_id);
        }
    }
}

async fn relay_for(state: &AppState, worker_id: &str) -> Arc<Mutex<RelayState>> {
    let mut relays = state.relays.write().await;
    relays
        .entry(worker_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(RelayState::new())))
        .clone()
}

async fn terminal_relay_for(
    state: &AppState,
    worker_id: &str,
) -> Arc<Mutex<TerminalRelayState>> {
    let mut relays = state.terminal_relays.write().await;
    relays
        .entry(worker_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(TerminalRelayState::default())))
        .clone()
}

async fn update_state(state: &AppState, worker_id: &str, new_state: WorkerState) {
    let mut workers = state.store.workers.write().await;
    if let Some(record) = workers.get_mut(worker_id) {
        record.state = new_state;
        record.updated_at = Utc::now();
    }
}

async fn update_state_with_ssh(
    state: &AppState,
    worker_id: &str,
    new_state: WorkerState,
    ssh: Option<ctx_worker_protocol::SshInfo>,
) {
    let mut workers = state.store.workers.write().await;
    if let Some(record) = workers.get_mut(worker_id) {
        record.state = new_state;
        if ssh.is_some() {
            record.ssh = ssh;
        }
        record.updated_at = Utc::now();
    }
}

async fn ensure_exists(
    state: &AppState,
    worker_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    workers.get(worker_id).ok_or_else(not_found)?;
    Ok(())
}

async fn get_request_spec(
    state: &AppState,
    worker_id: &str,
) -> Result<StartWorkerRequest, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(worker_id).ok_or_else(not_found)?;

    Ok(record.spec.clone())
}

async fn get_base_commit(
    state: &AppState,
    worker_id: &str,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(worker_id).ok_or_else(not_found)?;
    Ok(record.base_commit_sha.clone())
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new("worker_not_found")),
    )
}

#[derive(Debug, serde::Serialize)]
struct ErrorResponse {
    error: String,
}

impl ErrorResponse {
    fn new(error: &str) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}
