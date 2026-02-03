use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::{Request, StatusCode};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use chrono::{DateTime, Utc};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, info};

use ctx_worker_protocol::{
    new_worker_id, DiffArtifact, ExportPatchResponse, StartWorkerRequest, StartWorkerResponse,
    TerminalControlMessage, TerminalOpenRequest, WorkerInfo, WorkerRegistration, WorkerState,
};
use serde::Deserialize;
use tokio_util::io::ReaderStream;

use crate::bootstrap::render_bootstrap_script;
use crate::drivers::aws::{AwsConfig, AwsDriver};
use crate::drivers::azure::{AzureConfig, AzureDriver};
use crate::drivers::gcp::{GcpConfig, GcpDriver};
use crate::drivers::local::LocalDriver;
use crate::drivers::WorkerDriver;
use serde_json::json;
use tracing::warn;

const DEFAULT_BIND: &str = "0.0.0.0:8787";
const DEFAULT_DRIVER: &str = "local";
const DEFAULT_WORKER_SHIM_PATH: &str = "ctx-worker-shim";
const DEFAULT_PUBLIC_BASE_URL: &str = "http://127.0.0.1:8787";
const DEFAULT_SESSION_MOUNT_PATH: &str = "/mnt/ctx-session";
const DEFAULT_WORKDIR_PATH: &str = "/mnt/ctx-session/workdir";
const DEFAULT_AWS_VOLUME_SIZE_GB: i32 = 100;
const DEFAULT_AWS_VOLUME_TYPE: &str = "gp3";
const DEFAULT_AWS_DATA_DEVICE_NAME: &str = "/dev/sdf";
const DEFAULT_GCP_DISK_SIZE_GB: i64 = 100;
const DEFAULT_GCP_DISK_TYPE: &str = "pd-ssd";
const DEFAULT_AZURE_DISK_SIZE_GB: i32 = 100;
const DEFAULT_AZURE_DISK_SKU: &str = "Premium_LRS";

#[derive(Debug, Default, Deserialize)]
struct GatewayConfigFile {
    #[serde(default)]
    server: Option<ServerConfigFile>,
    #[serde(default)]
    aws: Option<AwsConfigFile>,
    #[serde(default)]
    gcp: Option<GcpConfigFile>,
    #[serde(default)]
    azure: Option<AzureConfigFile>,
}

#[derive(Debug, Default, Deserialize)]
struct ServerConfigFile {
    bind: Option<String>,
    driver: Option<String>,
    worker_shim_path: Option<String>,
    worker_shim_url: Option<String>,
    auth_token: Option<String>,
    public_base_url: Option<String>,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    session_mount_path: Option<String>,
    workdir_path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AwsConfigFile {
    region: Option<String>,
    ami_id: Option<String>,
    instance_type: Option<String>,
    subnet_id: Option<String>,
    security_group_ids: Option<Vec<String>>,
    key_name: Option<String>,
    instance_profile: Option<String>,
    ssh_user: Option<String>,
    volume_size_gb: Option<i32>,
    volume_type: Option<String>,
    data_device_name: Option<String>,
    delete_volume_on_pause: Option<bool>,
    wait_for_snapshot: Option<bool>,
    availability_zone: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct GcpConfigFile {
    project_id: Option<String>,
    zone: Option<String>,
    machine_type: Option<String>,
    image: Option<String>,
    network: Option<String>,
    subnetwork: Option<String>,
    service_account: Option<String>,
    scopes: Option<Vec<String>>,
    disk_size_gb: Option<i64>,
    disk_type: Option<String>,
    ssh_user: Option<String>,
    delete_disk_on_pause: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct AzureConfigFile {
    subscription_id: Option<String>,
    resource_group: Option<String>,
    location: Option<String>,
    vm_size: Option<String>,
    image: Option<String>,
    vnet: Option<String>,
    subnet: Option<String>,
    admin_username: Option<String>,
    ssh_public_key: Option<String>,
    ssh_public_key_path: Option<String>,
    disk_size_gb: Option<i32>,
    disk_sku: Option<String>,
    delete_disk_on_pause: Option<bool>,
    use_public_ip: Option<bool>,
}

#[derive(Debug)]
struct ResolvedArgs {
    bind: String,
    driver: String,
    worker_shim_path: String,
    worker_shim_url: Option<String>,
    auth_token: Option<String>,
    public_base_url: String,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    session_mount_path: String,
    workdir_path: String,
    aws_region: Option<String>,
    aws_ami_id: Option<String>,
    aws_instance_type: Option<String>,
    aws_subnet_id: Option<String>,
    aws_security_group_ids: Vec<String>,
    aws_key_name: Option<String>,
    aws_instance_profile: Option<String>,
    aws_ssh_user: Option<String>,
    aws_volume_size_gb: i32,
    aws_volume_type: String,
    aws_data_device_name: String,
    aws_delete_volume_on_pause: bool,
    aws_wait_for_snapshot: bool,
    aws_availability_zone: Option<String>,
    gcp_project_id: Option<String>,
    gcp_zone: Option<String>,
    gcp_machine_type: Option<String>,
    gcp_image: Option<String>,
    gcp_network: Option<String>,
    gcp_subnetwork: Option<String>,
    gcp_service_account: Option<String>,
    gcp_scopes: Vec<String>,
    gcp_disk_size_gb: i64,
    gcp_disk_type: String,
    gcp_ssh_user: Option<String>,
    gcp_delete_disk_on_pause: bool,
    azure_subscription_id: Option<String>,
    azure_resource_group: Option<String>,
    azure_location: Option<String>,
    azure_vm_size: Option<String>,
    azure_image: Option<String>,
    azure_vnet: Option<String>,
    azure_subnet: Option<String>,
    azure_admin_username: Option<String>,
    azure_ssh_public_key: Option<String>,
    azure_ssh_public_key_path: Option<String>,
    azure_disk_size_gb: i32,
    azure_disk_sku: String,
    azure_delete_disk_on_pause: bool,
    azure_use_public_ip: bool,
}

#[derive(Parser, Debug)]
#[command(name = "ctx-worker-gateway")]
struct Args {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    bind: Option<String>,
    #[arg(long)]
    driver: Option<String>,
    #[arg(long)]
    worker_shim_path: Option<String>,
    #[arg(long)]
    worker_shim_url: Option<String>,
    #[arg(long)]
    auth_token: Option<String>,
    #[arg(long)]
    public_base_url: Option<String>,
    #[arg(long)]
    tls_cert_path: Option<String>,
    #[arg(long)]
    tls_key_path: Option<String>,
    #[arg(long)]
    session_mount_path: Option<String>,
    #[arg(long)]
    workdir_path: Option<String>,

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
    #[arg(long)]
    aws_volume_size_gb: Option<i32>,
    #[arg(long)]
    aws_volume_type: Option<String>,
    #[arg(long)]
    aws_data_device_name: Option<String>,
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
    #[arg(long)]
    gcp_disk_size_gb: Option<i64>,
    #[arg(long)]
    gcp_disk_type: Option<String>,
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
    #[arg(long)]
    azure_disk_size_gb: Option<i32>,
    #[arg(long)]
    azure_disk_sku: Option<String>,
    #[arg(long)]
    azure_delete_disk_on_pause: bool,
    #[arg(long)]
    azure_use_public_ip: bool,
}

fn default_config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".ctx/worker-gateway.toml"))
}

fn load_gateway_config(path: &std::path::Path) -> Result<GatewayConfigFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading gateway config at {}", path.display()))?;
    toml::from_str(&text).context("parsing gateway config")
}

fn resolve_args(args: Args, config: Option<GatewayConfigFile>) -> ResolvedArgs {
    let server = config.as_ref().and_then(|cfg| cfg.server.as_ref());
    let aws = config.as_ref().and_then(|cfg| cfg.aws.as_ref());
    let gcp = config.as_ref().and_then(|cfg| cfg.gcp.as_ref());
    let azure = config.as_ref().and_then(|cfg| cfg.azure.as_ref());

    let bind = args
        .bind
        .or_else(|| server.and_then(|s| s.bind.clone()))
        .unwrap_or_else(|| DEFAULT_BIND.to_string());
    let driver = args
        .driver
        .or_else(|| server.and_then(|s| s.driver.clone()))
        .unwrap_or_else(|| DEFAULT_DRIVER.to_string());
    let worker_shim_path = args
        .worker_shim_path
        .or_else(|| server.and_then(|s| s.worker_shim_path.clone()))
        .unwrap_or_else(|| DEFAULT_WORKER_SHIM_PATH.to_string());
    let worker_shim_url = args
        .worker_shim_url
        .or_else(|| server.and_then(|s| s.worker_shim_url.clone()));
    let auth_token = args
        .auth_token
        .or_else(|| server.and_then(|s| s.auth_token.clone()));
    let public_base_url = args
        .public_base_url
        .or_else(|| server.and_then(|s| s.public_base_url.clone()))
        .unwrap_or_else(|| DEFAULT_PUBLIC_BASE_URL.to_string());
    let tls_cert_path = args
        .tls_cert_path
        .or_else(|| server.and_then(|s| s.tls_cert_path.clone()));
    let tls_key_path = args
        .tls_key_path
        .or_else(|| server.and_then(|s| s.tls_key_path.clone()));
    let session_mount_path = args
        .session_mount_path
        .or_else(|| server.and_then(|s| s.session_mount_path.clone()))
        .unwrap_or_else(|| DEFAULT_SESSION_MOUNT_PATH.to_string());
    let workdir_path = args
        .workdir_path
        .or_else(|| server.and_then(|s| s.workdir_path.clone()))
        .unwrap_or_else(|| DEFAULT_WORKDIR_PATH.to_string());

    let aws_security_group_ids = if !args.aws_security_group_ids.is_empty() {
        args.aws_security_group_ids
    } else {
        aws.and_then(|c| c.security_group_ids.clone())
            .unwrap_or_default()
    };
    let gcp_scopes = if !args.gcp_scopes.is_empty() {
        args.gcp_scopes
    } else {
        gcp.and_then(|c| c.scopes.clone()).unwrap_or_default()
    };

    ResolvedArgs {
        bind,
        driver,
        worker_shim_path,
        worker_shim_url,
        auth_token,
        public_base_url,
        tls_cert_path,
        tls_key_path,
        session_mount_path,
        workdir_path,
        aws_region: args
            .aws_region
            .or_else(|| aws.and_then(|c| c.region.clone())),
        aws_ami_id: args
            .aws_ami_id
            .or_else(|| aws.and_then(|c| c.ami_id.clone())),
        aws_instance_type: args
            .aws_instance_type
            .or_else(|| aws.and_then(|c| c.instance_type.clone())),
        aws_subnet_id: args
            .aws_subnet_id
            .or_else(|| aws.and_then(|c| c.subnet_id.clone())),
        aws_security_group_ids,
        aws_key_name: args
            .aws_key_name
            .or_else(|| aws.and_then(|c| c.key_name.clone())),
        aws_instance_profile: args
            .aws_instance_profile
            .or_else(|| aws.and_then(|c| c.instance_profile.clone())),
        aws_ssh_user: args
            .aws_ssh_user
            .or_else(|| aws.and_then(|c| c.ssh_user.clone())),
        aws_volume_size_gb: args
            .aws_volume_size_gb
            .or_else(|| aws.and_then(|c| c.volume_size_gb))
            .unwrap_or(DEFAULT_AWS_VOLUME_SIZE_GB),
        aws_volume_type: args
            .aws_volume_type
            .or_else(|| aws.and_then(|c| c.volume_type.clone()))
            .unwrap_or_else(|| DEFAULT_AWS_VOLUME_TYPE.to_string()),
        aws_data_device_name: args
            .aws_data_device_name
            .or_else(|| aws.and_then(|c| c.data_device_name.clone()))
            .unwrap_or_else(|| DEFAULT_AWS_DATA_DEVICE_NAME.to_string()),
        aws_delete_volume_on_pause: if args.aws_delete_volume_on_pause {
            true
        } else {
            aws.and_then(|c| c.delete_volume_on_pause).unwrap_or(false)
        },
        aws_wait_for_snapshot: if args.aws_wait_for_snapshot {
            true
        } else {
            aws.and_then(|c| c.wait_for_snapshot).unwrap_or(false)
        },
        aws_availability_zone: args
            .aws_availability_zone
            .or_else(|| aws.and_then(|c| c.availability_zone.clone())),
        gcp_project_id: args
            .gcp_project_id
            .or_else(|| gcp.and_then(|c| c.project_id.clone())),
        gcp_zone: args.gcp_zone.or_else(|| gcp.and_then(|c| c.zone.clone())),
        gcp_machine_type: args
            .gcp_machine_type
            .or_else(|| gcp.and_then(|c| c.machine_type.clone())),
        gcp_image: args.gcp_image.or_else(|| gcp.and_then(|c| c.image.clone())),
        gcp_network: args
            .gcp_network
            .or_else(|| gcp.and_then(|c| c.network.clone())),
        gcp_subnetwork: args
            .gcp_subnetwork
            .or_else(|| gcp.and_then(|c| c.subnetwork.clone())),
        gcp_service_account: args
            .gcp_service_account
            .or_else(|| gcp.and_then(|c| c.service_account.clone())),
        gcp_scopes,
        gcp_disk_size_gb: args
            .gcp_disk_size_gb
            .or_else(|| gcp.and_then(|c| c.disk_size_gb))
            .unwrap_or(DEFAULT_GCP_DISK_SIZE_GB),
        gcp_disk_type: args
            .gcp_disk_type
            .or_else(|| gcp.and_then(|c| c.disk_type.clone()))
            .unwrap_or_else(|| DEFAULT_GCP_DISK_TYPE.to_string()),
        gcp_ssh_user: args
            .gcp_ssh_user
            .or_else(|| gcp.and_then(|c| c.ssh_user.clone())),
        gcp_delete_disk_on_pause: if args.gcp_delete_disk_on_pause {
            true
        } else {
            gcp.and_then(|c| c.delete_disk_on_pause).unwrap_or(false)
        },
        azure_subscription_id: args
            .azure_subscription_id
            .or_else(|| azure.and_then(|c| c.subscription_id.clone())),
        azure_resource_group: args
            .azure_resource_group
            .or_else(|| azure.and_then(|c| c.resource_group.clone())),
        azure_location: args
            .azure_location
            .or_else(|| azure.and_then(|c| c.location.clone())),
        azure_vm_size: args
            .azure_vm_size
            .or_else(|| azure.and_then(|c| c.vm_size.clone())),
        azure_image: args
            .azure_image
            .or_else(|| azure.and_then(|c| c.image.clone())),
        azure_vnet: args
            .azure_vnet
            .or_else(|| azure.and_then(|c| c.vnet.clone())),
        azure_subnet: args
            .azure_subnet
            .or_else(|| azure.and_then(|c| c.subnet.clone())),
        azure_admin_username: args
            .azure_admin_username
            .or_else(|| azure.and_then(|c| c.admin_username.clone())),
        azure_ssh_public_key: args
            .azure_ssh_public_key
            .or_else(|| azure.and_then(|c| c.ssh_public_key.clone())),
        azure_ssh_public_key_path: args
            .azure_ssh_public_key_path
            .or_else(|| azure.and_then(|c| c.ssh_public_key_path.clone())),
        azure_disk_size_gb: args
            .azure_disk_size_gb
            .or_else(|| azure.and_then(|c| c.disk_size_gb))
            .unwrap_or(DEFAULT_AZURE_DISK_SIZE_GB),
        azure_disk_sku: args
            .azure_disk_sku
            .or_else(|| azure.and_then(|c| c.disk_sku.clone()))
            .unwrap_or_else(|| DEFAULT_AZURE_DISK_SKU.to_string()),
        azure_delete_disk_on_pause: if args.azure_delete_disk_on_pause {
            true
        } else {
            azure.and_then(|c| c.delete_disk_on_pause).unwrap_or(false)
        },
        azure_use_public_ip: if args.azure_use_public_ip {
            true
        } else {
            azure.and_then(|c| c.use_public_ip).unwrap_or(false)
        },
    }
}

#[derive(Clone)]
struct AppState {
    store: Arc<WorkerStore>,
    driver: Arc<dyn WorkerDriver>,
    public_base_url: String,
    worker_shim_path: String,
    session_mount_path: String,
    workdir_path: String,
    auth_token: Option<String>,
    terminal_relays: Arc<RwLock<HashMap<String, Arc<Mutex<TerminalRelayState>>>>>,
}

struct WorkerStore {
    workers: RwLock<HashMap<String, WorkerRecord>>,
}

#[derive(Default)]
struct TerminalRelayState {
    control_tx: Option<mpsc::UnboundedSender<TerminalControlMessage>>,
    pending: VecDeque<TerminalControlMessage>,
    sessions: HashMap<String, TerminalSessionRelay>,
}

const MAX_PENDING_TERMINAL_MESSAGES: usize = 256;

#[derive(Default)]
struct TerminalSessionRelay {
    daemon_tx: Option<mpsc::UnboundedSender<Message>>,
    worker_tx: Option<mpsc::UnboundedSender<Message>>,
    pending_for_daemon: VecDeque<Message>,
    pending_for_worker: VecDeque<Message>,
}

#[derive(Clone)]
struct WorkerRecord {
    worker_id: String,
    spec: StartWorkerRequest,
    task_id: String,
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
